# Pipeline Design — Speed + Proofs

> How recovered blocks flow through memory, storage, and proof generation
> without slowing down.

---

## 0. How Shreds Are Acquired

We don't passively wait for shreds. We actively request them via the **repair protocol**:

```
[dc-gossip] discovers validators with their serve_repair + tvu ports
       ↓ ContactInfo{serve_repair: IP:port4, tvu: IP:port10}
[repair sender] constructs RepairProtocol::WindowIndex{
       slot, shred_index,
       header{nonce, sender=our_pubkey, recipient=validator_pubkey}
       } signed with our keypair
       ↓ UDP to validator's serve_repair port
[validator] verifies signature, looks up shred in blockstore
       ↓ response: [shred_bytes (1232)] + [nonce (8 bytes bincode)]
[our UDP socket] strips last 8 bytes (nonce), feeds remainder to pipeline
```

### Repair Wire Format

```
RepairProtocol::WindowIndex {
    header: RepairRequestHeader {
        signature: Signature,    // 64 bytes, set to default then signed over
        sender: Pubkey,          // 32 bytes — our node identity
        recipient: Pubkey,       // 32 bytes — target validator pubkey
        timestamp: u64,          // 8 bytes — unix timestamp
        nonce: u64,              // 8 bytes — unique per request, dedup on response
    },
    slot: u64,                   // 8 bytes — slot we're requesting
    shred_index: u64,            // 8 bytes — specific shred index in slot
}
```

The response from the validator is simply:
`[raw_shred_bytes] + [nonce (8 bytes bincode)]`

We strip the nonce, and the remaining bytes are fed directly to `Shred::parse_from_bytes()`.

---

## 1. The Problem

Solana produces blocks every 400ms. Each block has thousands of shreds. Validators
need to:
1. Discover peers via gossip
2. Request shreds via repair protocol
3. Receive all shreds
4. Recover lost ones via RS
5. Reassemble into entries
6. Generate Merkle proofs over transactions
7. Serve proofs to bots/bridges/light clients

If any step blocks the next, we drop packets and lose data. The solution is a
**lock-free pipeline** where each stage runs on its own thread and communicates
via channels.

**Current status: v1 runs single-threaded** for simplicity. All stages run inside
one `tokio::select!` loop. No channels, no queues, no thread coordination bugs.
Multi-threaded optimization is deferred until we have real users and benchmarks
proving it's the bottleneck.

---

## 2. The Pipeline (Current — Single-Threaded)

```
┌──────────────┐
│  dc-gossip   │  discovers validators, maintains peer table
│  0.0.0.0:8000│  (spawned as background tokio task)
└──────┬───────┘
       │ ContactInfo via mpsc channel
       ▼
┌──────────────────┐
│  MAIN LOOP       │  single tokio::select!:
│  (single thread) │    - gossip peer arrives → send 32 repair requests
│                  │    - UDP packet arrives → parse shred → route to FEC batch
│                  │    - batch complete → RS recover → deshred → Merkle tree
│                  │    - store in ring buffer + flat file
│                  │    - every 10s: print status
└──────────────────┘
       │
       ▼ (future: separate RPC thread)
┌──────────────────┐
│  RPC SERVER      │  serves proofs from ring buffer or flat file
│  (planned)       │  JSON-RPC over HTTP
└──────────────────┘
```

**Why single-threaded for v1:**
- Zero coordination bugs — everything is sequential in one loop
- No channel backpressure to tune or debug
- RS decode (~1ms) and Merkle build (~2ms) are fast enough for devnet traffic
- Threading can be added later when measured as a bottleneck

**Future upgrade path:**
1. RPC server starts on its own thread (first split — it's a TCP listener, different concern)
2. Merkle building moves to a background thread (when slot throughput exceeds ~500/sec)
3. RS recovery moves to a thread pool (when FEC batch rate exceeds ~1000/sec)

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

### 3.2 Structure (Current — Simplified for v1)

```rust
pub struct SlotRingBuffer {
    slots: Vec<Option<Arc<SlotData>>>,
    capacity: usize,       // 500
    head: u64,             // oldest slot (plain u64, not atomic — single-threaded)
    tail: u64,             // next slot to write
}

pub struct SlotData {
    pub slot: u64,
    pub parent_slot: u64,
    pub entries: Vec<u8>,          // bincode-serialized entries
    pub num_transactions: usize,
    pub merkle_root: Option<[u8; 32]>,
    // NOTE: merkle_tree not stored — rebuilt from flat file on demand
    // if slot is evicted from ring buffer. ~2ms rebuild for 1000 txs.
}
```

**Deviation from ideal design:** No `merkle_tree: Option<Arc<MerkleTree>>` field.
The tree is built inline when the slot is first processed, and the root is stored.
If the slot gets evicted and later requested via RPC, the tree is rebuilt from
the flat file. This saves memory and avoids complexity — at the cost of ~2ms
rebuild time for cold slots.
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

### 4.4 Entry Type

**Important:** `solana_sdk::entry::Entry` was removed in Solana SDK 2.x. We define
our own `Entry` struct matching the bincode wire format:

```rust
#[derive(serde::Deserialize, serde::Serialize)]
pub struct Entry {
    pub num_hashes: u64,
    pub hash: Hash,                                     // solana_sdk::hash::Hash
    pub transactions: Vec<VersionedTransaction>,         // solana_transaction::versioned
}
```

The wire format is: `u64` (num_hashes) + `[u8; 32]` (hash) + `Vec<VersionedTransaction>`.
No length prefix for the hash — it's a fixed 32-byte field.

### 4.5 Where It Runs

Currently runs **inline** in the main loop, right after RS recovery succeeds
and before the slot data is stored. The deshredder output feeds directly into
Merkle tree construction on the next line. No queue, no thread hop.

### 4.6 Actual Code

```rust
// In main.rs recovery handler:
if let Some(result) = deshredder::deshred_into_txs(&recovered) {
    let tree = MerkleTree::new(&result.transactions);
    let slot_data = SlotData {
        slot: batch.slot,
        parent_slot: batch.parent_slot,
        entries: bincode::serialize(&result.entries).unwrap_or_default(),
        num_transactions: result.transactions.len(),
        merkle_root: Some(tree.root),
    };
    ring_buffer.put(slot_data);
    file_store.save_slot(batch.slot, &recovered.concat());
}
```

Deshredder takes `&[Vec<u8>]` (recovered shred payloads) and returns:
- `entries: Vec<Entry>` — the parsed entries for re-serialization into SlotData
- `transactions: Vec<Vec<u8>>` — each individual tx re-serialized, one per Merkle leaf


---

## 5. Merkle Prover

### 5.1 What Is a Merkle Tree?

A binary tree where every leaf is a SHA-256 hash of one piece of data (a transaction),
and every parent is the hash of its two children concatenated.

```
        root = H(H12 + H34)
       /                    \
   H12 = H(H1 + H2)     H34 = H(H3 + H4)
    /        \            /        \
  H1=H(tx1) H2=H(tx2)  H3=H(tx3) H4=H(tx4)
```

The root is a single 32-byte hash that represents the entire set of transactions.

### 5.2 Why Do We Need It?

Without Merkle: a bot asking "was tx1 in slot 1000?" must either trust our node
or download all 1000 transactions to verify.

With Merkle: we answer "yes — here's a short proof. Verify it yourself."
The proof is just ~5 hashes (for 1000 leaves, log2(1000) ≈ 10, but we store
the path). The bot verifies in microseconds. No trust, no massive download.

### 5.3 How a Proof Works

To prove tx1 is in the tree above:

1. We give the bot: `leaf_hash = H(tx1)`, `proof = [H2, H34]` (sibling hashes
   along the path from leaf to root)
2. Bot computes: `H12 = H(leaf_hash + H2)`, then `root' = H(H12 + H34)`
3. If `root' == root`, tx1 was definitely in the tree. No other transaction
   could produce the same root — hash collisions are cryptographically infeasible.

The `prove(tx_index)` method returns the sibling path. The `verify()` method
walks from leaf to root hashing at each step.

### 5.4 Where It Runs (Current)

Currently built **inline** in the main loop, immediately after deshredding.
The tree is built eagerly (not lazily — not waiting for a client to ask).

```rust
let tree = MerkleTree::new(&result.transactions);
// root extracted, tree discarded after SlotData is created
```

**Current behavior:**
- Tree built immediately when the slot completes
- Only `merkle_root` is stored in `SlotData` (not the full tree)
- If a client later asks for a proof and the slot was evicted from ring buffer,
  the tree is rebuilt from the flat file on demand (~2ms for 1000 txs)
- This is the simplest possible approach: no caching, no LRU, no disk trees

**Design doc ideal (future):**
The tree stored as `Arc<MerkleTree>` in `SlotData.merkle_tree` on a dedicated
Merkle thread. Multiple clients request proofs concurrently from the same tree
— immutable after construction, no locks needed.

---

## 6. Thread Model (Current — Single-Threaded)

### Thread 0: Gossip + Peer Discovery (background tokio task)
```
Loop:
  ping/pong with entrypoint to join gossip
  send PullRequest every 5s → receive PushResponses with ContactInfos
  maintain CRDS table: prune old entries every 30s
  on each discovery cycle:
    emit ContactInfo via mpsc channel → consumed by main loop
```

### Main Loop (single tokio::select!)
This is the entire pipeline in one place. No separate threads. No queues.

```
loop {
    tokio::select! {
        // Arm 1: New peer discovered via gossip
        Some(ci) = gossip_rx.recv() => {
            for idx in 0..NUM_DATA_SHREDS {
                send_repair_request(socket, peer, keypair, slot, idx)
            }
        }

        // Arm 2: UDP packet arrives (shred or ping)
        Ok((len, peer)) = socket.recv_from(&mut buf) => {
            packet = &buf[..len]
            match parse_response(packet) {
                Ping  → respond with Pong + re-send repair requests
                Shred → {
                    parse => classify as Data or Code
                    add to FecBatch
                    if batch complete:
                        RS decode (try_recover)
                        deshred_into_txs (deshredder)
                        MerkleTree::new (build tree)
                        ring_buffer.put + file_store.save_slot
                        remove batch from pending map
                }
            }
        }
    }
}
```

**Why this works for v1:**
- At devnet traffic levels, a slot completes every ~400ms
- RS decode (~1ms), deshred (~0.1ms), Merkle build (~2ms) = ~3ms total per slot
- CPU utilization is well under 10% — no bottleneck
- `tokio::select!` naturally prioritizes I/O (UDP receive) over processing

**When to split into threads:**
- RPC server needs its own thread first (blocking I/O + JSON parsing shouldn't
  delay shred reception)
- Merkle prover can stay inline until slot rate exceeds ~300/sec (3ms × 300 = 90% CPU)
- RS recovery can move to thread pool when FEC batches exceed ~1000/sec

### Thread 4: RPC Server (separate tokio task)
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
  slot_0465322655.dat      ← binary: concatenated recovered shred payloads (30816 bytes)
  slot_0465322655.meta     ← JSON: { "slot": 465322655, "size": 30816 }
  slot_0465322696.dat
  slot_0465322696.meta
  ...
```

**Note:** The `.dat` file stores the raw concatenated shred payloads (not
deserialized entries). This is a v1 simplification — it's what comes out of
RS recovery glued together. When a cold slot needs to be served, the bytes
are deserialized → entries → Merkle tree → proof.

**Future:** Store pre-deserialized entries in `.dat` to avoid re-parsing on
every cold request.

**Current v1:** Each slot is 2 files (data + metadata). No `.proof` file is
written — proof is generated on demand from the data. Files are written once,
never modified — append-only semantics for the directory level.

**Future:** Add `.proof` file to cache the Merkle tree on disk, avoiding rebuild
for cold slots.

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

## 8. Bottleneck Analysis (Single-Threaded v1)

| Stage | Latency | Notes |
|-------|---------|-------|
| UDP recv | ~1µs | Fine |
| Shred parse | ~0.5µs | Fine |
| RS decode | ~1ms | Fine at devnet rates (~2.5 slots/sec) |
| Deshred | ~0.1ms | Basically free |
| Merkle tree build | ~2ms | Fine at devnet rates |
| Flat file write | ~0.5ms | Sequential, fine |
| Proof query | ~5µs | Fine |

Total per slot: ~4ms processing. Solana produces 1 slot per 400ms.
CPU utilization: ~1%. No bottleneck.

**When bottlenecks appear (and how to fix them):**

| Scenario | Bottleneck | Fix |
|----------|-----------|-----|
| 100+ slots/sec | RS decode | Move to thread pool |
| 300+ slots/sec | Merkle build | Separate Merkle thread |
| 10k+ RPS | Proof queries | Cache rebuilt trees in LRU |
| Network flood | UDP recv | SO_REUSEPORT + multiple sockets |

---

## 9. Starting Up

On cold start:
1. Start dc-gossip to join devnet and discover validators (port 8000)
2. Wait until at least one validator is discovered in CRDS table
3. Open UDP socket for repair responses (port 8003)
4. For each discovered validator, send repair requests for recent slots
5. Ingest responses → FEC batches → recovery → deshred → ring buffer
6. If someone asks for an old slot, load from flat file
7. Periodically re-query gossip for fresh ContactInfos

This means the node can restart without losing data. The ring buffer starts
empty and fills as repair responses arrive. Old slots are always available from disk.

---

## 10. Design Principles

1. **No locks** — single-threaded for v1. No mutexes, no atomics, no contention.
2. **Newer data > older data** — drop old shreds before dropping new ones.
3. **Merkle tree built eagerly** — tree built inline when slot completes and root
   stored. Tree NOT stored — rebuilt from flat file if evicted (~2ms cost).
   Simpler than lazy compute-on-query for v1.
4. **Disk is cold, memory is hot** — recent slots in RAM, old ones on disk.
5. **Append-only on disk** — no random writes, no fragmentation.
6. **Single writer** — everything runs in one thread. No scheduling anomalies.

**Future principles (when multi-threaded):**
1. No locks in hot path — atomic writes + immutability after write
2. One writer per stage — no contention between threads
3. Computation is lazy — defer Merkle building to background thread
