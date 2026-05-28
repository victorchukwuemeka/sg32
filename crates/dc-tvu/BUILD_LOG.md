# Build Log — dc-tvu Pipeline

> A running record of what was built, why, and how we debugged every issue along the way.  
> For anyone reading this code and wondering "why did they do it like that?"

---

## Table of Contents

1. [Overview: What We Built](#1-overview-what-we-built)
2. [Phase 1 — Shred Parsing](#2-phase-1--shred-parsing)
3. [Phase 2 — FEC Recovery (Reed-Solomon)](#3-phase-2--fec-recovery-reed-solomon)
4. [Phase 3 — Repair Protocol + Main Loop](#4-phase-3--repair-protocol--main-loop)
5. [Phase 4 — Deshredder](#5-phase-4--deshredder)
6. [Phase 5 — Merkle Tree + Storage](#6-phase-5--merkle-tree--storage)
7. [Known Quirks & Future Work](#7-known-quirks--future-work)

---

## 1. Overview: What We Built

`dc-tvu` is the Transaction Validation Unit — the receiver half of a Solana validator. It:

1. Connects to **Solana devnet** via gossip to discover validators
2. Sends **repair requests** asking for shreds (pieces of blocks)
3. Receives shreds over UDP, **parses** them into data shreds and coding shreds
4. Groups shreds into **FEC batches** (32 data + 32 code)
5. When enough shreds arrive, runs **Reed-Solomon recovery** to reconstruct missing ones
6. **Deshreds** the recovered bytes back into transaction entries
7. Builds a **Merkle tree** over the transactions (one leaf per tx)
8. Stores everything in a **ring buffer** (hot, in-memory) and **flat files** (cold, on disk)

The end goal: serve Merkle proofs so anyone can verify "was this transaction in this block?" without trusting a node.

### Architecture at a Glance

```
                      ┌──────────────────┐
                      │   dc-gossip       │── discovers validators, feeds peer list
                      │   (port 8000)     │
                      └────────┬─────────┘
                               │ ContactInfo (serve_repair, tvu, gossip ports)
                               ▼
                      ┌──────────────────┐
                      │   Repair Sender   │── sends RepairProtocol::WindowIndex for each shred index
                      │   (in main loop)  │    (32 sequential requests per validator)
                      └────────┬─────────┘
                               │ response: [shred bytes] + [nonce (8 bytes)]
                               ▼
                      ┌──────────────────┐
                      │   Shred Parser    │── strips nonce, parses common header + variant headers
                      │   (ingest path)   │    classifies as MerkleData or MerkleCode
                      └────────┬─────────┘
                               │ classified shred
                               ▼
                      ┌──────────────────┐
                      │   FecBatch        │── collects shreds, deduplicates by index
                      │   (per erasure    │    triggers recovery when received_count >= num_data
                      │    set)           │
                      └────────┬─────────┘
                               │ recovered data payloads
                               ▼
                      ┌──────────────────┐
                      │   Deshredder      │── concatenates payloads in index order
                      │                   │    trims padding zeros
                      │                   │    deserializes Vec<Entry> → Vec<VersionedTransaction>
                      │                   │    re-serializes each tx individually
                      └────────┬─────────┘
                               │ Vec<Vec<u8>> (one serialized tx per element)
                               ▼
                      ┌──────────────────┐
                      │   MerkleTree      │── hashes each tx with SHA-256
                      │                   │    builds binary Merkle tree
                      │                   │    root = single 32-byte hash for all txs
                      └────────┬─────────┘
                               │ SlotData { slot, parent_slot, entries, merkle_root, ... }
                               ▼
                      ┌──────────────────┐
                      │   RingBuffer      │── hot storage, in-memory, 500 slots
                      │   + FlatFileStore │── cold storage, /data/slot_{slot}.dat + .meta
                      └──────────────────┘
```

---

## 2. Phase 1 — Shred Parsing

### 2.1 What Is a Shred?

A **shred** is a piece of a Solana block, sized to fit in one UDP packet. Every shred is exactly 1203 bytes (data) or 1228 bytes (coding). The leader splits block entries into data shreds and generates coding shreds as parity data.

### 2.2 Shred Wire Format

Every shred has a **83-byte common header** at offset 0:

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 0 | 64 | `signature` | ed25519 signature of the Merkle root |
| 64 | 1 | `shred_variant` | Enum variant byte |
| 65 | 8 | `slot` | u64 LE — the slot this shred belongs to |
| 73 | 4 | `index` | u32 LE — shred index within slot |
| 77 | 2 | `version` | u16 LE — cluster shred version |
| 79 | 4 | `fec_set_index` | u32 LE — first data shred index in this FEC batch |

The `shred_variant` byte encodes:
- Bit 7: 0 = MerkleData, 1 = MerkleCode
- Bits 4-6: proof_size (log2 of total shreds in batch)
- Bit 0: resigned (retransmitter signature present)

After the common header:
- **Data shred**: 5 more bytes (parent_offset, flags, size) → 88 bytes total headers
- **Coding shred**: 6 more bytes (num_data, num_coding, position) → 89 bytes total headers

### 2.3 Parsing Implementation — `shred_header.rs`

We defined `ShredCommonHeader`, `DataShredHeader`, and `CodingShredHeader` as plain structs with a manual `from_bytes()` method.

**Why manual parsing instead of bincode `#[derive(Deserialize)]`?**

Because the `signature` field is `[u8; 64]` on the wire — raw 64 bytes with no length prefix. But `#[serde(with = "serde_bytes")]` (which we tried first) injects a u64 length prefix before the bytes, making it 8 + 64 = 72 bytes instead of 64. Bincode expects exactly what `serde_bytes` gives it, but the actual wire doesn't have a length prefix for fixed-size arrays.

**The fix**: `ShredCommonHeader::from_bytes()` reads each field by raw byte offset:

```rust
pub fn from_bytes(data: &[u8]) -> Option<ShredCommonHeader> {
    if data.len() < 83 { return None; }
    let mut signature = [0u8; 64];
    signature.copy_from_slice(&data[0..64]);
    let shred_variant = data[64];
    let slot = u64::from_le_bytes(data[65..73].try_into().ok()?);
    let index = u32::from_le_bytes(data[73..77].try_into().ok()?);
    // ... etc
}
```

This directly reads the wire bytes without going through serde at all.

### 2.4 Shred Enum — `shred.rs`

The `Shred` enum has two variants:

```rust
pub enum Shred {
    MerkleData {
        common_header: ShredCommonHeader,
        data_header: DataShredHeader,
        data: Vec<u8>,          // payload minus headers
        proof: Vec<[u8; 20]>,   // Merkle proof entries (sibling hashes)
        merkle_root: [u8; 32],  // chained Merkle root
    },
    MerkleCode {
        common_header: ShredCommonHeader,
        coding_header: CodingShredHeader,
        code: Vec<u8>,          // parity data payload
        proof: Vec<[u8; 20]>,
        merkle_root: [u8; 32],
    },
}
```

`Shred::parse_from_bytes()` dispatches based on the `shred_variant` byte:
- Bit 7 = 0 → parse as MerkleData (88 byte headers, data payload, proof, chained root)
- Bit 7 = 1 → parse as MerkleCode (89 byte headers, code payload, proof, chained root)

### 2.5 Debugging the Signature Field

**The bug**: `ShredCommonHeader` initially used `#[serde(with = "serde_bytes")]` on the signature field. This added a length prefix that doesn't exist on the wire.

**How we found it**: The first byte of the signature was always `0x08` (length 8) instead of `0x20` (the actual ed25519 sig start). All shreds failed to parse. We dumped the raw first 20 bytes of a packet, saw the mismatch, and traced it back to serde_bytes.

**The fix**: Manual byte reads with `copy_from_slice`. No serde involved for parsing.

---

## 3. Phase 2 — FEC Recovery (Reed-Solomon)

### 3.1 Why FEC?

UDP drops packets. If a validator misses 1 out of 32 data shreds, the whole block is unreadable. Reed-Solomon erasure coding creates parity shreds so you only need _any_ 32 out of 64 to recover all 32 originals.

### 3.2 How Reed-Solomon Works

The leader creates a **Vandermonde matrix** (or Cauchy matrix) of size 32×32. Multiplying the 32 data shreds by this matrix gives 32 parity shreds.

If some shreds are lost on the network, the receiver:
1. Builds a **reduced matrix** from the rows corresponding to received shreds
2. Inverts it
3. Multiplies received shreds by the inverse to recover the missing ones

All math is done in **GF(2^8)** — Galois field of 256 elements — which is why we have `gf256.rs`.

### 3.3 Our Implementation — `gf256.rs` + `reed_solomon.rs`

**`gf256.rs`**: Pure GF(2^8) arithmetic:
- `gf_add(a, b)` = XOR (addition in GF is XOR)
- `gf_mul(a, b)` = multiply with polynomial reduction
- `gf_inv(a)` = multiplicative inverse using extended Euclidean
- Log/exp tables for fast multiplication

**`reed_solomon.rs`**: 
- `generate_cauchy_matrix(n, k)`: builds a Cauchy matrix (n data × k code positions)
- `decode(received, row_indices, cauchy, n)`: 
  1. Sub-selects `n` rows from the Cauchy matrix (matching received shreds)
  2. Inverts the sub-matrix via Gaussian elimination
  3. Multiplies received data by the inverse → recovered missing shreds

The decode only runs when `received_count >= num_data`. If all 32 data shreds arrived, no recovery is needed.

### 3.4 FEC Batch Tracker — `fec_batch.rs`

`FecBatch` is per (slot, fec_set_index):

```rust
pub struct FecBatch {
    pub slot: u64,
    pub fec_set_index: u32,
    pub parent_slot: u64,        // extracted from first data shred's parent_offset
    pub num_data: usize,         // typically 32
    pub num_code: usize,         // typically 32
    pub data_shreds: Vec<Option<Vec<u8>>>,  // slot per index, None = missing
    pub code_shreds: Vec<Option<Vec<u8>>>,
}
```

- `add_data_shred(index, data)`: inserts at `index`, returns false if already present
- `add_code_shred(position, data)`: same for coding shreds
- `try_recover()`: checks if enough shreds received, runs RS decode, returns `Option<Vec<Vec<u8>>>` (the recovered data payloads)

---

## 4. Phase 3 — Repair Protocol + Main Loop

### 4.1 The Main Loop — `main.rs`

The main binary does:

1. Generate an ed25519 keypair for our node identity
2. Bind UDP socket to **0.0.0.0:8003** (repair response port)
3. Spawn `run_gossip_loop()` in a background task (gossip on port 8000)
4. Wait for peers from gossip
5. For each discovered validator: send 32 repair requests (one per shred index)
6. Listen for responses: handle pings (respond with pong), handle shreds (route to FEC batch)

### 4.2 Repair Requests

The repair protocol asks a validator for a specific shred using `RepairProtocol::WindowIndex`:

```
[4b enum_tag][64b signature][32b sender_pubkey][32b recipient_pubkey][8b timestamp][8b nonce][8b slot][8b shred_index]
```

We send **32 sequential requests** per validator (indices 0 through 31, all data shreds in the first FEC batch).

**Why sequential?** Ed25519 signing each request takes ~75μs. 32 × 75μs ≈ 2.4ms per validator. Concurrent signing didn't help because the bottleneck was the signer, not the network.

### 4.3 Nonce Stripping

Validator responses append an 8-byte nonce to the shred bytes:

```
[shred_bytes (1203)] + [nonce (8 bytes bincode)]
```

We strip the last 8 bytes before parsing. The nonce is used for deduplication but we currently ignore it (simplified from the full repair spec).

### 4.4 Batch Management

Batches are stored in a `HashMap<ErasureSetId, FecBatch>`. `ErasureSetId` is `(slot, fec_set_index)`. When `try_recover()` succeeds:
- Deshred the recovered payloads
- Build Merkle tree over transactions
- Store in ring buffer + flat file
- Remove the batch from the map

---

## 5. Phase 4 — Deshredder

### 5.1 What the Deshredder Does

The deshredder is the reverse of the leader's shredder. The leader takes entries (transactions), serializes them as `Vec<Entry>`, then splits into 963-byte chunks (one per data shred). The deshredder:

1. Takes the **recovered data payloads** (already stripped of headers + proofs)
2. **Concatenates** them in shred index order
3. **Trims trailing zeros** (padding from alignment to FEC batch boundary)
4. **Deserializes** the blob as `Vec<Entry>` using bincode
5. Each `Entry` contains: `num_hashes: u64`, `hash: Hash`, `transactions: Vec<VersionedTransaction>`
6. **Flattens** all transactions from all entries into one list
7. **Re-serializes** each transaction individually into `Vec<u8>` (for Merkle hashing)

### 5.2 The Entry Type Problem

**The bug**: `solana_sdk::entry::Entry` was removed in Solana SDK 2.x (the `Entry` type moved to `solana-entry` crate, which we don't depend on).

**The fix**: Define our own `Entry` struct matching the exact bincode wire format:

```rust
#[derive(serde::Deserialize, serde::Serialize)]
pub struct Entry {
    pub num_hashes: u64,
    pub hash: Hash,                          // solana_sdk::hash::Hash = [u8; 32]
    pub transactions: Vec<VersionedTransaction>,  // solana_transaction::versioned::VersionedTransaction
}
```

No external dependency needed — `Hash` and `VersionedTransaction` are available from crates already in our dependency tree.

### 5.3 Why Round-Trip Serialize?

You might wonder: why deserialize entries only to re-serialize each transaction? Why not just split the raw bytes directly?

Because we need **transaction boundaries**. The raw blob is just one ~30KB string of bytes. Without deserializing into structured types, we don't know where tx1 ends and tx2 begins. Deserializing parses the bincode format and gives us individual `VersionedTransaction` objects with clear boundaries. Re-serializing each one produces clean `Vec<u8>` ready for Merkle hashing.

The flow:

```
shred bytes → bincode::deserialize → Vec<Entry> → flattened Vec<VersionedTransaction>
                                                        ↓
                                              bincode::serialize each one
                                                        ↓
                                              Vec<Vec<u8>> → MerkleTree::new()
```

### 5.4 What We Don't Do

The deshredder **does not** reorder, rearrange, or reformat the data. It's pure concatenation + parsing. The bytes come off the wire in entry-order (shred 0 has the first bytes, shred 1 has the next, etc.). We just glue them back and trim padding.

---

## 6. Phase 5 — Merkle Tree + Storage

### 6.1 Merkle Tree — `merkle_prover.rs`

A binary Merkle tree where:
- Each **leaf** = SHA-256 hash of one serialized transaction
- Each **parent** = SHA-256 hash of its two children concatenated
- The **root** = a single 32-byte hash representing the entire transaction set

```rust
pub struct MerkleTree {
    pub leaves: Vec<[u8; 32]>,   // hashed transactions
    pub nodes: Vec<[u8; 32]>,    // internal nodes
    pub root: [u8; 32],          // root hash
}
```

**Why Merkle trees?** Without them, proving "tx X was in slot Y" requires the full transaction list. With them, you just need ~log2(N) sibling hashes (~10 for 1000 txs). The verifier walks from leaf to root rebuilding hashes — if the result matches the stored root, inclusion is cryptographically proven.

### 6.2 Ring Buffer — `ring_buffer.rs`

A fixed-size (500 slots) circular buffer holding recent `SlotData` in memory:

```rust
pub struct SlotData {
    pub slot: u64,
    pub parent_slot: u64,
    pub entries: Vec<u8>,                // bincode-serialized entries
    pub num_transactions: usize,
    pub merkle_root: Option<[u8; 32]>,
}
```

Fast lookups (~1μs) for recent slots. When full, the oldest slot gets evicted (should be saved to disk first).

### 6.3 Flat File Store — `flat_file_store.rs`

Cold storage on disk, organized as:

```
data/
  slot_{slot}.dat      ← raw bytes (all concatenated recovered shred payloads)
  slot_{slot}.meta     ← JSON metadata (slot, size)
```

Files are written once, never modified. Slots can be reloaded from disk if evicted from the ring buffer.

### 6.4 Data Flow Through the Pipeline

```
Validator responds with shred bytes (1203 bytes + 8 byte nonce)
    ↓ strip nonce
Shred::parse_from_bytes()
    ↓ yields Shred::MerkleData { common_header, data_header, data, proof, merkle_root }
FecBatch::add_data_shred(data_index, data)
    ↓ repeated for all shreds in the FEC set
FecBatch::try_recover()
    ↓ returns Vec<Vec<u8>> (recovered data payloads)
deshredder::deshred_into_txs(&recovered)
    ↓ concatenates, trims, deserializes Vec<Entry>, flattens transactions
    ↓ returns DeshredResult { entries, transactions (Vec<Vec<u8>>) }
MerkleTree::new(&result.transactions)
    ↓ builds tree over serialized txs
SlotData { slot, parent_slot, entries, num_transactions, merkle_root }
    ↓
RingBuffer::put(slot_data)      ← hot storage
FlatFileStore::save_slot(slot, &raw_bytes)  ← cold storage
```

---

## 7. Known Quirks & Future Work

### Current Limitations

1. **Only first FEC set per slot**: We only request shred indices 0..31 (one FEC batch). A slot can have multiple FEC batches for large blocks. We need to request code shreds (indices 32..63) and subsequent batches.

2. **No batch GC**: FEC batches that will never complete (orphaned because too many shreds were lost) stay in the HashMap forever. They need periodic cleanup.

3. **Sequential repair requests**: Sending 32 requests per validator sequentially works but is slow. Could batch or pipeline them.

4. **No signature verification**: We parse the signature field but don't verify it. A production node should verify the leader's signature on each shred's Merkle root.

5. **No retransmission**: The design doc describes a Turbine tree where nodes forward shreds to their children. We don't implement this — we only receive, not forward.

6. **Single-threaded**: The design envisions a multi-threaded pipeline (ingester thread, recoverer thread, Merkle prover thread, RPC thread). We run everything in a single tokio task.

7. **No RPC server**: The final stage — serving proofs to clients — doesn't exist yet.

### Future Work

| Priority | Feature | Why |
|----------|---------|-----|
| P0 | RPC server | Serve proofs to clients |
| P1 | Request code shreds (indices 32+) | Better resilience when data shreds are dropped |
| P1 | Multiple FEC batches per slot | Handle large blocks (>32 shreds) |
| P2 | Batch GC | Prevent memory leak from orphaned batches |
| P2 | Signature verification | Don't trust, verify |
| P3 | Multi-threaded pipeline | Performance at scale |
| P3 | Turbine retransmission | Participate in block propagation |

---

## Appendix: Key Debugging Lessons

### Lesson 1: Don't use serde_bytes for raw byte arrays

If you have `[u8; 64]` and use `#[serde(with = "serde_bytes")]`, bincode will write/expect:
```
[length_prefix: u64][actual_bytes: 64]
```
But the wire just has 64 bytes. Use manual `copy_from_slice` instead.

### Lesson 2: Vote.slot is skip_serializing on the wire

Agave's `Vote` struct marks `slot` as `#[serde(skip_serializing)]` — it's not on the wire. If you use standard bincode `#[derive(Deserialize)]` for votes, you'll get wrong slot values. Custom Deserialize is required (done in `dc-gossip`'s `crds_data.rs`).

### Lesson 3: Entry type moved in Solana SDK 2.x

`solana_sdk::entry::Entry` was removed when Solana split the SDK into smaller crates. If you hit this, define your own Entry struct — the bincode wire format is just `num_hashes: u64 + hash: [u8; 32] + transactions: Vec<VersionedTransaction>`.

### Lesson 4: FEC padding zeros

The last data shred in a FEC batch may be padded with zeros to align to the batch boundary. These zeros must be trimmed **after** concatenation, not before. Otherwise the last entry gets corrupted with zero bytes appended.

---

*Last updated: May 2026*
