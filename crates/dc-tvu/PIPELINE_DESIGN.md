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
                    │  +           │              └──────────────┘
                    │  DESHREDDER  │
                    │  (same thread│
                    │   to avoid   │
                    │   extra copy)│
                    └──────┬───────┘
                           │ entries (Vec<Transaction>)
                           ▼
                    ┌──────────────┐
                    │  RING BUFFER │  ← assembled slot data, ready to prove
                    └──────┬───────┘
                           │ slot with entries
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
              │  /data/slot_1000.dat   │     entries saved here
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

## 4. Deshredder (Shreds → Entries)

### 4.1 The Problem

Recovered shreds are raw byte chunks, each ~1203 bytes, split across 32 data
shreds per FEC batch, across multiple batches per slot. Nobody can read them
like this. The deshredder puts the pieces back together.

### 4.2 What It Does

```
One slot has 3 FEC batches, each with up to 32 data shreds:

Data shreds (sorted by index):
  [index 0]     bytes 0-962      ← batch 0
  [index 1]     bytes 963-1925   ← batch 0
  ...
  [index 32]    bytes ...        ← batch 1
  [index 33]    bytes ...
  ...
  [index N]     last bytes

Deshredder:
  1. Collect ALL data shreds for the slot from all FEC batches
  2. Sort by shred index (ascending)
  3. For each shred: strip headers (88 bytes), strip Merkle proof footer
  4. Concatenate remaining payload bytes in order
  5. Trim trailing zeros (padding from incomplete FEC batches)
  6. Deserialize the byte blob as Vec<Entry>
  7. Each Entry is a batch of transactions
```

### 4.3 Visual

```
Shred 0:  [sig|hdr|←───── 963 bytes of entry data ──────→|Merkle|proof]
Shred 1:  [sig|hdr|←───── 963 bytes of entry data ──────→|Merkle|proof]
Shred 2:  [sig|hdr|←───── 963 bytes of entry data ──────→|Merkle|proof]
  ...
                                                              │
                                                   strip sig+headers+proof
                                                              │
                                                              ▼
                                ┌────────────────────────────────┐
                                │   Concatenated entry bytes     │
                                │   (triangle: sorted by index)  │
                                └───────────────┬────────────────┘
                                                │ bincode::deserialize
                                                ▼
                                ┌────────────────────────────────┐
                                │        Vec<Entry>              │
                                │   Entry { txs: [Tx, Tx, ...] } │
                                │   Entry { txs: [Tx, Tx, ...] } │
                                │   Entry { txs: [Tx, Tx, ...] } │
                                └────────────────────────────────┘
```

### 4.4 Where It Runs

The deshredder runs in the **Recoverer thread** (Thread 2). After RS recovery
is done and all shreds for a batch are present, we immediately deshred the
slot. This avoids copying the shred bytes to another thread just for reassembly.

Once deshredded, the Vec<Entry> goes into the ring buffer as SlotData.entries.
From there, the Merkle prover can build proofs over individual transactions.

### 4.5 Code Sketch

```rust
fn deshred(shreds: &[Shred]) -> Option<Vec<Entry>> {
    let mut data_shreds: Vec<&Shred> = shreds.iter()
        .filter(|s| s.shred_type() == ShredType::Data)
        .collect();
    data_shreds.sort_by_key(|s| s.index());

    let mut all_data = Vec::new();
    for shred in &data_shreds {
        // strip 88 bytes of headers, strip Merkle proof, keep only data
        let payload = shred.data_payload()?;
        all_data.extend_from_slice(payload);
    }

    // Trim padding zeros from incomplete last FEC batch
    while all_data.last() == Some(&0) {
        all_data.pop();
    }

    bincode::deserialize::<Vec<Entry>>(&all_data).ok()
}
```

---

## 5. Thread Model

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

## 7. Flat File Store (Cold Storage)

### 7.1 Why Do We Need It?

The ring buffer holds 500 slots in RAM. Solana produces one block every 400ms.
That's 2.5 blocks per second. After 500 / 2.5 = 200 seconds (~3 minutes), the
oldest slot gets evicted from the ring buffer.

If a bot asks for a proof from slot 100 and we already evicted it, what do we
say? "Sorry, I had it 3 minutes ago but forgot it"? That's unacceptable.

The flat file store is the **permanent memory** — disk-backed, indexed by slot,
never loses data. It's slower than RAM (~100µs seek vs ~1µs RAM lookup) but
has unlimited capacity. Together with the ring buffer, we get:

| | Ring Buffer | Flat File Store |
|---|---|---|
| Speed | ~1µs lookup | ~100-500µs lookup |
| Capacity | 500 slots (limited by RAM) | Unlimited (limited by disk) |
| Persistence | Lost on restart | Survives restart |
| Best for | Recent slots (last 3 min) | All historical slots |

### 7.2 When Data Moves to Disk

```
                    slot arrives
                         │
                         ▼
               ┌─────────────────┐
               │  Ring Buffer    │  ← hot, fast, in RAM
               │  (500 slots)    │
               └────────┬────────┘
                        │ ring buffer full → evict oldest slot
                        ▼
               ┌─────────────────┐
               │  Flat File      │  ← cold, permanent, on disk
               │  /data/         │
               └─────────────────┘
                        │
                        ▼
               Served to bots via RPC
               (proxy to our node asking for proof)
```

The key: a bot never sees a "not found" error as long as the flat file exists.
If the slot is in RAM → instant. If on disk → fast. Always available.

### 7.3 How Data Is Organized on Disk

```
/data/
  slot_0000001000.dat      ← binary: bincode serialized Vec<Entry>
  slot_0000001000.proof    ← binary: Merkle tree nodes, ready to serve proofs
  slot_0000001000.meta     ← JSON: { slot, parent_slot, block_time, num_txs }
  slot_0000001001.dat
  slot_0000001001.proof
  slot_0000001001.meta
  ...
```

Each slot is 3 files (data, proof, metadata). Files are written once, never
modified — append-only semantics for the directory level.

### 7.4 In-Memory Index

We keep a lightweight in-memory index of what's on disk:

```rust
struct FlatFileIndex {
    by_slot: BTreeMap<Slot, SlotFileInfo>,
    data_dir: PathBuf,
}

struct SlotFileInfo {
    data_path: PathBuf,
    data_size: u64,
    proof_path: Option<PathBuf>,  // None until proof is generated
    meta: SlotMeta,
}
```

Loaded by scanning `/data/` at startup. `BTreeMap<Slot, ...>` takes ~24 bytes
per slot — 1 million slots would use ~24 MB RAM for the index (negligible).

---

## 8. Bottleneck Analysis

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

## 9. Starting Up

On cold start:
1. Scan `/data/` for existing `.dat` files
2. Build in-memory index of all known slots
3. Start listening on UDP (all new slots go into ring buffer)
4. If someone asks for an old slot, load from flat file

This means the node can restart without losing data. The ring buffer starts
empty and fills as new slots arrive. Old slots are always available from disk.

---

## 10. Design Principles

1. **No locks in hot path** — atomic writes + immutability after write
2. **Newer data > older data** — drop old shreds before dropping new ones
3. **Computation is lazy** — build Merkle trees only when someone asks
4. **Disk is cold, memory is hot** — recent slots in RAM, old ones on disk
5. **Append-only on disk** — no random writes, no fragmentation
6. **One writer per stage** — no contention between threads
