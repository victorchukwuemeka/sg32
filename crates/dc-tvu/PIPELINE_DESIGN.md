# Pipeline Design — Speed + Proofs

> How recovered blocks flow through memory, storage, and proof generation
> without slowing down.

---

## 1. The Problem

Solana produces blocks every 400ms. Each block has thousands of shreds. Validators
need to:
1. Receive all shreds
2. Recover lost ones via RS
3. Reassemble into entries
4. Generate Merkle proofs over transactions
5. Serve proofs to bots/bridges/light clients

If any step blocks the next, we drop packets and lose data. The solution is a
**lock-free pipeline** where each stage runs on its own thread and communicates
via channels.

---

## 2. The Pipeline

```
                    ┌──────────────┐
   UDP ────────────►│  INGESTER    │─── shreds ──►┌──────────────┐
   socket           │  (thread 1)  │              │  FEC BATCH   │
                    │  parse +     │              │  TRACKER     │
                    │  classify    │              │  (in-memory) │
                    └──────────────┘              └──────┬───────┘
                                                         │ batch complete
                                                         ▼
                    ┌──────────────┐              ┌──────────────┐
                    │  RECOVERER   │◄─── recover ─┤  RECOVERY    │
                    │  (thread 2)  │              │  QUEUE       │
                    │  RS decode   │              │              │
                    └──────┬───────┘              └──────────────┘
                           │ recovered shreds
                           ▼
                    ┌──────────────┐
                    │  RING BUFFER │  ← hot storage, 500 slots in memory
                    └──────┬───────┘
                           │ slot complete
                           ▼
                    ┌──────────────┐
                    │  MERKLE      │  ← build Merkle tree over slot's txs
                    │  PROVER      │
                    │  (thread 3)  │
                    └──────┬───────┘
                           │ slot + proof
                           ▼
              ┌────────────────────────┐
              │  FLAT FILE STORE       │  ← cold storage, append-only
              │  /data/slot_1000.dat   │     evicted from ring buffer
              │  /data/slot_1000.proof │
              └────────────────────────┘
                           │
                           ▼
              ┌────────────────────────┐
              │  RPC SERVER            │  ← serves proofs from memory or disk
              │  (thread 4)            │
              └────────────────────────┘
```

Each arrow is a `tokio::sync::mpsc` channel. The receiver can't block the sender.
If the receiver is slow, messages buffer in the channel (up to a limit), then
the sender drops the oldest. This is intentional — newer shreds are more valuable
than old ones.

---

## 3. Ring Buffer Design

### 3.1 Why a Ring Buffer

A `HashMap<Slot, SlotData>` would cause:
- Memory fragmentation from constant insert/remove
- Cache misses from random access patterns
- Unbounded memory growth if old slots aren't evicted

A ring buffer (fixed-size array, circular index) solves all three:

```
     head (oldest)                        tail (newest)
       │                                     │
       ▼                                     ▼
    ┌─────┬─────┬─────┬─────┬─────┬─────┬─────┬─────┐
    │  N  │ N+1 │ N+2 │ N+3 │ ... │     │     │     │
    └─────┴─────┴─────┴─────┴─────┴─────┴─────┴─────┘
       ↑                                     ↑
    evicted to disk                      writing now
```

### 3.2 Structure

```rust
pub struct SlotRingBuffer {
    slots: Vec<Option<Arc<SlotData>>>,
    capacity: usize,
    head: AtomicU64,  // oldest slot
    tail: AtomicU64,  // next slot to write
}

pub struct SlotData {
    pub slot: u64,
    pub entries: Vec<Entry>,
    pub merkle_root: [u8; 32],
    pub merkle_tree: Option<Arc<MerkleTree>>,  // computed lazily
    pub parent_slot: u64,
    pub num_transactions: usize,
    pub block_time: Option<i64>,
}
```

### 3.3 Lookup

```rust
fn get(&self, slot: u64) -> Option<Arc<SlotData>> {
    let head = self.head.load(Ordering::Acquire);
    let tail = self.tail.load(Ordering::Acquire);
    
    if slot < head || slot >= tail {
        return None;  // not in ring buffer
    }
    
    let idx = (slot % self.capacity as u64) as usize;
    match &self.slots[idx] {
        Some(data) if data.slot == slot => Some(data.clone()),
        _ => None,
    }
}
```

The `None` means the slot was either:
- Too old (evicted to disk — check flat file store)
- Too new (not assembled yet — still in FEC tracker)
- Doesn't exist

### 3.4 Eviction Policy

When the ring buffer is full and a new slot arrives:
1. The oldest slot (`head`) gets evicted
2. Before eviction: if not already saved to flat file, serialize and write
3. Increment head
4. Insert new slot at tail
5. Increment tail

No locks needed because:
- Writers only touch `tail` and `tail % capacity`
- Readers compare `slot` field to verify the data matches
- `Arc<SlotData>` makes reads atomic — the slot's contents are immutable
  once written

---

## 4. Thread Model

### Thread 1: Ingester
```
Loop:
  recv_from(socket)        ← blocking recv, but tokio's async handles it
  if len < 89: skip        ← too short to be a valid shred
  parse shred_variant      ← one byte read, instant
  get slot, index, fec     ← byte offset reads
  route to FEC batch:
    Data:   batch.add_data_shred(data_index, bytes)
    Code:   batch.add_code_shred(code_position, bytes)
  if batch.received_count() >= num_data:
    send batch to RecoveryQueue  ← channel send, non-blocking
```

Max throughput: bounded by UDP receive speed. On Linux with SO_REUSEPORT,
you can spawn multiple ingester threads each on their own socket.

### Thread 2: Recoverer
```
Loop:
  recv batch from RecoveryQueue
  cauchy = generate_cauchy_matrix(num_data, num_code)
  recovered = decode(received, row_indices, cauchy, num_data)
  if recovered.is_some():
    assemble slot from its FEC batches
    ring_buffer.put(slot, slot_data)
    if slot is complete:
      send slot to MerkleQueue
```

RS decode is the only expensive operation here (~1ms for 32×32 matrix inversion
+ 1228×32 column multiply). At 1000+ batches/sec, this thread is the bottleneck.
Optimization: precompute Cauchy matrix once and reuse.

### Thread 3: Merkle Prover
```
Loop:
  recv slot from MerkleQueue
  tree = MerkleTree::new(slot.entries)
  slot.merkle_tree = Some(Arc::new(tree))
  // Proof is now queryable: merkle_tree.prove(tx_index) → MerkleProof
```

Building a Merkle tree over ~1000 transactions (SHA-256 hashing) takes ~1-2ms
on modern hardware. Only runs once per completed slot. Proof generation is O(log N)
and happens on demand — not precomputed for every possible transaction.

### Thread 4: RPC Server
```
Incoming request: "prove tx X in slot Y"
  1. Look up slot Y in ring buffer
  2. If found: merkle_tree.prove(tx_index) → instant (in memory)
  3. If not found: load from flat file
  4. Return proof to client
```

Proof generation is a single tree walk: `O(log N)` SHA-256 hashes. ~5µs.

---

## 5. Flat File Store

### Format

```
/data/
  slot_0000001000.dat      ← binary: bincode serialized Vec<Entry>
  slot_0000001000.proof    ← binary: Merkle tree (all nodes, ready to serve proofs)
  slot_0000001000.meta     ← JSON: { slot, parent_slot, block_time, num_txs }
  slot_0000001001.dat
  slot_0000001001.proof
  slot_0000001001.meta
  ...
```

`.dat` files are written once when the slot is evicted from the ring buffer.
`.proof` files are written after the Merkle tree is built.
`.meta` files are small JSON objects for fast filtering without deserializing the
full block.

### Index

An in-memory `BTreeMap<Slot, (file_offset, file_path)>` tracks which files exist.
Loaded at startup from a directory scan. O(log N) lookup, negligible memory
(~24 bytes per slot).

---

## 6. Bottleneck Analysis

| Stage | Latency | Throughput | Bottleneck? |
|-------|---------|-----------|-------------|
| UDP recv | ~1µs | ~1M pkt/s per core | ❌ No (we do this) |
| Shred parse | ~0.5µs | 2M+ pkt/s | ❌ No |
| RS decode | ~1ms | 1000 batches/s | ⚠️ Maybe (need benchmarks) |
| Merkle tree build | ~2ms | 500 slots/s | ❌ No |
| Proof query | ~5µs | 200k proofs/s | ❌ No |

The only potential bottleneck is RS decode if we receive 1000+ FEC batches per
second (each needing recovery). Fixes:
- Precompute Cauchy matrix once
- Batch RS operations — decode multiple columns in parallel with SIMD or threads
- Use lookup tables for GF multiplication (already done)

---

## 7. Starting Up

On cold start:
1. Scan `/data/` for existing `.dat` files
2. Build in-memory index of all known slots
3. Start listening on UDP (all new slots go into ring buffer)
4. If someone asks for an old slot, load from flat file

This means the node can restart without losing data. The ring buffer starts
empty and fills as new slots arrive. Old slots are always available from disk.

---

## 8. Design Principles

1. **No locks in hot path** — atomic writes + immutability after write
2. **Newer data > older data** — drop old shreds before dropping new ones
3. **Computation is lazy** — build Merkle trees only when someone asks
4. **Disk is cold, memory is hot** — recent slots in RAM, old ones on disk
5. **Append-only on disk** — no random writes, no fragmentation
6. **One writer per stage** — no contention between threads
