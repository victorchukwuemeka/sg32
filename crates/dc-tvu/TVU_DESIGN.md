# dc-tvu design — Turbine Data Propagation

> Target network: Solana devnet  
> Transport: async tokio UDP + QUIC  
> Reference: Agave `turbine/`, `ledger/src/shred/`, `core/src/tvu.rs`

---

## Table of Contents

1. [What is TVU?](#1-what-is-tvu)
2. [Big Picture: Data Flow](#2-big-picture-data-flow)
3. [The Leader Side (Broadcast)](#3-the-leader-side-broadcast)
4. [Shred Wire Format](#4-shred-wire-format)
5. [Turbine Tree](#5-turbine-tree)
6. [The TVU Pipeline (Receiver Side)](#6-the-tvu-pipeline-receiver-side)
7. [Erasure Coding (Reed-Solomon)](#7-erasure-coding-reed-solomon)
8. [Merkle Tree Chaining](#8-merkle-tree-chaining)
9. [Retransmission](#9-retransmission)
10. [Repair Protocol](#10-repair-protocol)
11. [Module Structure](#11-module-structure)
12. [Implementation Plan](#12-implementation-plan)

---



## 1. What is TVU?

**TVU = Transaction Validation Unit.**

It's the **receiver side** of the validator. A validator has two halves:

```
                                ┌──────────────────────┐
                                │    YOUR VALIDATOR    │
                                │                      │
  Users send txs ─────────────► │  TPU (Transaction    │
  via RPC or QUIC               │  Processing Unit)    │
                                │  - accepts txs       │
                                │  - builds blocks     │
                                │  - only when leader  │
                                └──────────────────────┘

  Leader sends shreds ────────► │  TVU (Transaction    │
  via UDP/QUIC                  │  Validation Unit)    │
                                │  - receives blocks   │
                                │  - verifies them     │
                                │  - replays txs       │
                                │  - votes             │
                                │  - ALWAYS active     │
                                └──────────────────────┘
```

TVU is what **validators** run. It receives blocks from whoever the current leader is, verifies every transaction in the block, and decides whether to vote for it.

In our project (solana-protocol-gym), the TVU will:
1. Receive shreds from the network (on the TVU port)
2. Parse them (common header, data/coding headers, Merkle proofs)
3. Reassemble them into entries (the transactions)
4. Generate Merkle inclusion proofs over the transactions
5. Store everything for our RPC service

We are **not** replaying transactions against a bank. We are **not** voting. We are reading blocks from the wire and serving them with proofs.

---

## 2. Big Picture: Data Flow

Here is how a block travels from the leader's TPU to every validator's TVU:

```
LEADER'S MACHINE (TPU side):
┌────────────────────────────────────────────────────────────────┐
│                                                                │
│  BankingStage ──► entries ──► Shredder ──► Turbine Broadcast   │
│  (collects txs)        │      (split into   │                  │
│                        │       shreds +     │                  │
│                        │       RS encode)   │                  │
│                        ▼                    ▼                  │
│                  serialized            UDP packets             │
│                  Entry[]               to root node            │
└────────────────────────────────────────────────────────────────┘
                                                     │
                    Shreds fly over the network ──────┤
                    via Turbine tree                   │
                                                     │
                                                     ▼
OUR MACHINE (TVU side):
┌────────────────────────────────────────────────────────────────┐
│                                                                │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────────┐  │
│  │ ShredFetch   │───►│ SigVerify    │───►│ WindowService    │  │
│  │              │    │              │    │  +               │  │
│  │ raw UDP      │    │ check leader │    │ RetransmitStage  │  │
│  │ packets in   │    │ signature on │    │                  │  │
│  │              │    │ Merkle root  │    │ - insert shreds  │  │
│  │ filter by    │    │              │    │ - RS recover     │  │
│  │ shred_version│    │              │    │ - detect gaps    │  │
│  │ slot bounds  │    │              │    │ - repair missing │  │
│  └──────────────┘    └──────────────┘    │ - retransmit     │  │
│                                          │   to children    │  │
│                                          └────────┬─────────┘  │
│                                                   │            │
│                                          ┌────────▼─────────┐  │
│                                          │ Complete Slot    │  │
│                                          │ Detector         │  │
│                                          │                  │  │
│                                          │ - all shreds for │  │
│                                          │   slot received? │  │
│                                          │ - deshred into   │  │
│                                          │   entries (txs)  │  │
│                                          └────────┬─────────┘  │
│                                                   │            │
│                                          ┌────────▼─────────┐  │
│                                          │ Block Store      │  │
│                                          │ + Merkle Proofs  │  │
│                                          │                  │  │
│                                          │ - entries +      │  │
│                                          │   inclusion      │  │
│                                          │   proofs         │  │
│                                          └──────────────────┘  │
└────────────────────────────────────────────────────────────────┘
```

---

## 3. The Leader Side (Broadcast)

To understand TVU, you must first understand what the leader sends. The leader runs a **BroadcastStage** that turns entries into shreds.

### 3.1 How entries become shreds

```
Entry[] (e.g. 1000 transactions)
       │
       ▼
  Serialize with wincode (compact binary)
       │
       ▼
  Split into 1228-byte chunks (one per data shred)
  We get N data shreds
       │
       ▼
  Group into FEC (Forward Error Correction) batches
  Each batch = min(32, remaining) data shreds
       │
       ▼
  For each batch:
    1. Allocate 32 data shred slots + 32 coding shred slots
    2. Copy entry bytes into data shred payloads
    3. Fill empty data shred slots with zeros (padding)
    4. Reed-Solomon encode → fill coding shreds with parity bytes
    5. Build Merkle tree over all 64 shreds
    6. Sign the Merkle root with leader's keypair
    7. Attach Merkle proof to each shred
    8. For the LAST batch only: also attach retransmitter signature slot
       │
       ▼
  All shreds are sent to the "root" node in the Turbine tree
  The root retransmits to its children, and so on
```

### 3.2 The Shredder (Agave reference)

- **`Shredder::new(slot, parent_slot, reference_tick, shred_version)`**
- **`make_merkle_shreds_from_entries(keypair, entries, is_last_in_slot, chained_merkle_root, ...)`**

The reference_tick is used for `ShredFlags::SHRED_TICK_REFERENCE_MASK` — it tells which tick within the slot this shred was produced at. It saturates at 63 (only 6 bits).

If `is_last_in_slot` is true:
- The LAST FEC batch is "signed" (has retransmitter signature space)
- The very last data shred gets `LAST_SHRED_IN_SLOT` flag

### 3.3 Broadcast

After shreds are created, the leader:
1. Sends shreds to exactly **one** node: the "root" of the weighted shuffle for that slot
2. That node retransmits to its children
3. Each child retransmits to its children
4. Max 4 hops to cover the entire network

---

## 4. Shred Wire Format

This is the most important section. Every shred is exactly **1203 bytes** (data) or **1228 bytes** (coding) — sized to fit in a single UDP packet (MTU = 1500, minus IP/UDP headers).

### 4.1 Common Header (83 bytes, same for data and coding)

This is at the very start of every shred. The byte offsets are absolute (from byte 0 of the payload).

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 0 | 64 | `signature` | ed25519 signature of the Merkle root |
| 64 | 1 | `shred_variant` | Enum variant: `MerkleData{proof_size, resigned}` or `MerkleCode{proof_size, resigned}` |
| 65 | 8 | `slot` | The slot this shred belongs to (u64, LE) |
| 73 | 4 | `index` | Shred index within slot (u32, LE) — data shreds 0..N, coding shreds 0..M |
| 77 | 2 | `version` | The cluster's shred version (u16, LE) — e.g. 11016 for devnet |
| 79 | 4 | `fec_set_index` | The index of the first data shred in this FEC batch (u32, LE) |

**Important**: `signature` at offset 0 is the leader's signature of the Merkle root. This is NOT a signature of the entire shred — only the 32-byte Merkle root is signed.

**`shred_variant` byte encoding**:
- Bit 7 (128): 0 = MerkleData, 1 = MerkleCode
- Bits 4-6: `proof_size` (number of Merkle proof entries, 0-7)
  - For 32:32 FEC batch, proof_size = 6 (because 64 entries need log2(64) = 6 levels)
  - Actual value encoded = proof_size
- Bit 0: `resigned` — whether retransmitter signature is present (only last FEC batch in slot)

So the byte at offset 64 might be `0b10000110` = 134 for:
- MerkleCode (128) + proof_size 6 (bits 4-6 = 011) + resigned false (0)

### 4.2 Data Shred Header (5 bytes, after common header at offset 83)

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 83 | 2 | `parent_offset` | u16 LE — slot difference from parent. `parent_slot = slot - parent_offset` |
| 85 | 1 | `flags` | Bitmask: bits 0-5 = reference_tick (6 bits), bit 6 = DATA_COMPLETE_SHRED, bit 7 = LAST_SHRED_IN_SLOT |
| 86 | 2 | `size` | u16 LE — total size of the data including headers (used to find where data ends) |

Total data header size: **88 bytes** (83 common + 5 data).

### 4.3 Coding Shred Header (6 bytes, after common header at offset 83)

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 83 | 2 | `num_data_shreds` | u16 LE — number of data shreds in this FEC batch |
| 85 | 2 | `num_coding_shreds` | u16 LE — number of coding shreds in this FEC batch |
| 87 | 2 | `position` | u16 LE — this coding shred's position within the batch (0..num_coding_shreds-1) |

Total coding header size: **89 bytes** (83 common + 6 coding).

### 4.4 Payload Section (after headers)

Now the layout diverges for data vs coding:

**DATA SHRED payload (offset 88 onwards):**

```
Offset 88: entry data (variable length, up to ~1043 bytes)
           This is the actual transaction bytes (serialized Entry[])
           ERASURE CODED section starts here

           ...data continues until capacity is reached...

           Chained Merkle Root (32 bytes) — the Merkle root of the PREVIOUS FEC batch
           Also part of erasure coded section
           (if this is the first batch in the slot, it's the previous slot's last root)

           --- END of erasure coded section ---

           Merkle Proof (proof_size * 20 bytes)
           Each entry = 20 bytes (2 32-bit big-endian child indices + 1 SHA-256 hash truncated to first 20 bytes? No — let me be precise)

           Actually: MerkleProofEntry = 20 bytes
           Structure: [u8; 20] — the sibling hash needed to recompute the Merkle root.
           There are `proof_size` of these, where proof_size = ceil(log2(num_shreds_in_batch)).
           For 32+32=64 shreds, proof_size = 6 (because 2^6 = 64).

           [Optional] Retransmitter Signature (64 bytes)
           Only present if `resigned` bit is set in shred_variant.
           Only the LAST FEC batch in a slot has this.
           The retransmitter signs the Merkle root with their own key.
```

**CODING SHRED payload (offset 89 onwards):**

```
Offset 89: Erasure coded shard (variable length)
           This is the Reed-Solomon parity data.
           Same size as the erasure coded section in data shreds.

           Chained Merkle Root (32 bytes)

           --- END of erasure coded section ---

           Merkle Proof (proof_size * 20 bytes)

           [Optional] Retransmitter Signature (64 bytes)
```

### 4.5 Capacity Calculation

```
For a data shred with proof_size=6, not resigned:
  Total payload    = 1203 bytes
  Headers          = 88 bytes
  Merkle Root      = 32 bytes (erasure coded)
  Merkle Proof     = 6 * 20 = 120 bytes (NOT erasure coded)
  Remaining for data = 1203 - 88 - 32 - 120 = 963 bytes
```

So each data shred holds at most **963 bytes** of transaction data.

For the last batch (resigned=true):
```
  Remaining for data = 1203 - 88 - 32 - 120 - 64 = 899 bytes
```

One FEC batch (32 data shreds) can hold:
```
  32 * 963 = 30,816 bytes (unsigned) or 32 * 899 = 28,768 bytes (signed/last batch)
```

### 4.6 Quick Parse Without Deserialization

You can extract key fields from a raw shred without full deserialization using byte offsets (from `shred/wire.rs`):

```rust
fn get_slot(shred: &[u8]) -> Option<u64>     // bytes 65..73
fn get_index(shred: &[u8]) -> Option<u32>    // bytes 73..77
fn get_version(shred: &[u8]) -> Option<u16>  // bytes 77..79
fn get_fec_set_index(shred: &[u8]) -> Option<u32> // bytes 79..83
fn get_shred_variant(shred: &[u8]) -> ShredVariant // byte 64
fn get_shred_type(shred: &[u8]) -> ShredType // from variant
fn get_signature(shred: &[u8]) -> Signature  // bytes 0..64
```

---

## 5. Turbine Tree

Turbine is how the leader gets shreds to every validator without sending them individually to each one.

### 5.1 Problem

If there are 750 validators and each block has ~6400 shreds, the leader would need to send **4.8 million** individual UDP packets. The leader has limited bandwidth (maybe 1 Gbps = ~83k pps for 1500-byte packets). That's impossible.

### 5.2 Solution: Tree Broadcast

Instead the leader sends to **one** node, which sends to 200 nodes, which each send to 200, etc.

With fanout=200 and max 4 hops, you can reach 200^4 = 1.6 billion nodes. In practice:

```
Leader ──► Root ──► Layer 1 ──► Layer 2 ──► Layer 3
  |         1        200         40,000       ~1M (way more than needed)
```

With ~750 validators, we need ~4 layers with 200 fanout:
```
Layer 0: 1 node (root, receives from leader)
Layer 1: 200 nodes (each receives from root)
Layer 2: 200*200 = 40,000 nodes (more than enough)
```

But wait — validators have different stakes. The tree is weighted by stake. Important nodes are closer to the root.

### 5.3 How the Tree is Built (cluster_nodes.rs)

**Step 1: Collect all nodes**

Gather from gossip:
- The local node itself (our identity)
- All known TVU peers from gossip (their ContactInfo with TVU addresses)
- All staked nodes (even if no ContactInfo in gossip yet)

**Step 2: Sort by (stake, pubkey) descending**

Highest stake nodes come first. This means high-stake validators are closer to the root.

**Step 3: Dedup by TVU address**

If two nodes have the same TVU IP/port, only keep the higher-stake one.

**Step 4: Weighted shuffle**

A Fisher-Yates shuffle where each position in the shuffle gets picked proportional to its stake. This is deterministic because the PRNG is seeded with `(leader_pubkey, shred_id)`.

The PRNG is ChaCha8 or ChaCha20 (determined by feature flag).

**Step 5: The tree from the shuffle**

The shuffled array becomes the tree. It follows a specific formula:

```
Index 0 = Root (receives directly from leader)

For any node at index `k`:
  - Its children are at indices:
      anchor*fanout + offset + 1,
      anchor*fanout + offset + 2,
      ... for fanout children
    Where:
      offset = (k - 1) % fanout     if k > 0
      anchor = k - offset
    
    Special case: root (k=0) has children at indices 1, 2, ..., fanout

  - Its parent is at index:
      (k - 1) / fanout, then adjust for neighborhood
```

Here's a concrete example with fanout=2 and nodes [A, B, C, D, E, F, G, H]:

```
Index 0: A (root) ──► children: B, C    (indices 1, 2)
Index 1: B         ──► children: D, E    (indices 3, 4)
Index 2: C         ──► children: F, G    (indices 5, 6)
Index 3: D         ──► children: H       (index 7)
...
```

Visual tree:
```
         A (root)
        / \
       B   C
      / \ / \
     D  E F  G
    /
   H
```

### 5.4 How Your Node Decides What to Do

When your node receives a shred:

1. Look up the leader for this slot (from leader schedule)
2. Compute `ClusterNodes<RetransmitStage>` for the current epoch
3. Call `get_retransmit_addrs(leader, shred_id, fanout)`:
   - Exclude the leader from the shuffle
   - Shuffle with PRNG seeded by (leader_pubkey, shred_id)
   - Find our position in the shuffle
   - Compute children using the formula above
   - Return the TCP addresses of children
4. Forward the shred to those children (unless root_distance > 3, meaning there's no deeper layer)

---

## 6. The TVU Pipeline (Receiver Side)

This is what happens on every validator when shreds arrive.

### 6.1 Stage 1: ShredFetchStage

```
UDP sockets (TVU port, Repair port)
QUIC endpoint (newer protocol)

Raw packets arrive.
For each packet:
  1. Get shred bytes from packet (discard nonce if repair)
  2. Check shred_version matches our cluster
  3. Check slot is within reasonable range (not too far in future)
  4. For repair packets: verify repair nonce
  5. If any check fails: set discard flag on packet
  
Surviving packets are sent to the next stage via channel.
```

The `should_discard_shred()` function checks:
- `shred_version` matches our cluster
- `slot` is not too far in the future (> 2 epochs ahead)
- `slot` is not below the root bank slot
- FEC set index is valid

### 6.2 Stage 2: Signature Verification

```
For each shred:
  1. Determine the expected leader for this slot from LeaderScheduleCache
  2. Extract the Merkle root from the shred (via get_signed_data)
  3. Verify: signature on shred is leader's signature of Merkle root
  4. If invalid: discard
  5. Check if this is a retransmitted shred (retransmitter_signature present):
     - The retransmitter signs the SAME Merkle root with their key
     - This is additive — the leader signature is still there
     - Verify retransmitter signature separately

Verified shreds are sent to WindowService.
```

### 6.3 Stage 3: WindowService

This is the core logic. WindowService does two things in parallel:

**Thread A: Insert shreds into blockstore**

```
Loop:
  Receive a batch of (shred, is_repaired) pairs
  
  For each shred:
    - Deserialize from raw payload: common_header + data_header or coding_header
    - Check sanify: valid parent_offset, valid index range, valid Merkle proof
  
  Call blockstore.insert_shreds_handle_duplicate(batch):
    - Deduplicate by (slot, shred_index)
    - Store the shred bytes
    - If we have all data shreds for a FEC set → try erasure recovery
    - If recovery succeeds → we get more shreds back!
    - If we detect duplicate (same slot+index, different data) → emit PossibleDuplicateShred
    - If all shreds for a slot are received → emit CompletedDataSets
  
  Forward shreds to RetransmitStage for retransmission
  
  Send CompletedDataSets for block reassembly
```

**Thread B: Check for duplicate shreds**

```
Loop:
  Receive PossibleDuplicateShred events
  
  For each duplicate:
    - Store both conflicting shreds in blockstore
    - Propagate duplicate proof through gossip (so other validators know)
    - Notify the duplicate consensus state machine (ReplayStage)
      → the slot may need to be forked
```

### 6.4 Stage 4: RetransmitStage

```
Loop:
  Receive verified shreds from WindowService
  
  For each shred:
    1. Compute cluster_nodes for current epoch
    2. Determine leader for this slot
    3. Call get_retransmit_addrs(leader, shred_id, fanout)
       → returns list of child node addresses
    4. If this is the last FEC batch in the slot:
       - Add our retransmitter signature to the shred
       (We sign the Merkle root with our keypair)
    5. Send the shred to all children via UDP
    6. Cache (slot, shred_index) → [addresses] to avoid recomputing

The retransmitter signature is needed so downstream nodes can verify
that the shred came from a legitimate retransmitter, not a spammer.
Only the last FEC batch (the one with resigned=true) gets this treatment.
```

### 6.5 Stage 5: Deshredding (Reassembly)

```
When all shreds for a slot are received (CompletedDataSets):

  1. Collect all data shreds for the slot, sorted by index
  2. For each shred: extract the data portion (skip headers, skip Merkle proof)
  3. Concatenate all data portions in order
  4. Deserialize the concatenated bytes as Vec<Entry>
  5. Each Entry contains a vector of transactions

  Now we have: Slot X → Vec<Entry> → Vec<Transaction>

  From here, we can:
    - Store in our database
    - Generate Merkle inclusion proofs per transaction
    - Serve via RPC
```

### 6.6 Deshredding: The Exact Algorithm

```
fn deshred(shreds: &[Shred]) -> Result<Vec<Entry>> {
    // 1. Sort data shreds by index
    let mut data_shreds: Vec<&Shred> = shreds.iter()
        .filter(|s| s.is_data())
        .sorted_by_key(|s| s.index())
        .collect();
    
    // 2. Extract data from each shred
    let mut all_data = Vec::new();
    for shred in &data_shreds {
        let data = shred.data()?;  // skips headers, gives entry bytes
        all_data.extend_from_slice(data);
    }
    
    // 3. Trim trailing zeros (padding from last FEC batch)
    while all_data.last() == Some(&0) {
        all_data.pop();
    }
    
    // 4. Deserialize as Vec<Entry>
    let entries: Vec<Entry> = bincode::deserialize(&all_data)?;
    Ok(entries)
}
```

---

## 7. Erasure Coding (Reed-Solomon)

### 7.1 Why Erasure Coding?

UDP packets get dropped. Validators miss shreds. If we lose 1 out of 32 data shreds, we can't reassemble the block.

Reed-Solomon to the rescue: the leader generates 32 coding shreds from 32 data shreds. The coding shreds are **parity data** — mathematical combinations of the data shreds.

**You only need any 32 out of 64 shreds to recover all 32 originals.**

### 7.2 How It Works

```
Data shreds:   D0  D1  D2  ...  D31   (32 shreds)
Coding shreds: C0  C1  C2  ...  C31   (32 shreds)

If we receive: D0, D2, D3, ... (only 20 data shreds)
              C0, C5, C17, ... (12 coding shreds)
              Total = 32 ✓ → we can reconstruct!

The reconstruction:
  1. Matrix math: the coding shreds = encoding_matrix × data_shreds
  2. If some are lost, we invert a submatrix and solve for the missing ones
  3. The `reed_solomon_erasure` crate does all the heavy lifting
```

### 7.3 Erasure Shard

The "shard" is the portion of the shred that is erasure coded. It starts at byte 64 (after the signature) and goes up to the start of the Merkle proof.

```
Data shred:   [sig(64) | header | data... | chained_root(32) | MerkleProof | retrans_sig]
                ↑        ↑                                                      ↑
            byte 0   byte 64                                            byte 1203
                │                                                         │
                └────── erasure coded section ────────────────────────────┘
                (includes everything except signature and merkle proof)
```

For the recovery, all shards must be the same length. The coding shred's shard is the same size as the data shred's shard.

### 7.4 Recovery Algorithm

```rust
fn recover_missing_shreds(
    mut shreds: Vec<Shred>,      // received + stub shreds for missing ones
    reed_solomon_cache: &Cache,
) -> Result<Vec<Shred>> {
    // 1. Sort by erasure shard index (data first, then coding)
    shreds.sort_by_key(|s| s.erasure_shard_index());
    
    // 2. Figure out which positions are missing
    let num_data = coding_header.num_data_shreds;
    let num_coding = coding_header.num_coding_shreds;
    let mut mask = vec![false; num_data + num_coding];
    
    // 3. Create stub shreds for missing positions
    //    (empty payloads with correct headers)
    let mut batch = Vec::with_capacity(num_data + num_coding);
    for shred in shreds {
        let index = shred.erasure_shard_index()?;
        // Add stubs for gaps
        while batch.len() < index {
            batch.push(make_stub_shred(batch.len(), ...)?);
        }
        mask[index] = true;
        batch.push(shred);
    }
    // Add stubs for trailing gaps
    while batch.len() < num_data + num_coding {
        batch.push(make_stub_shred(batch.len(), ...)?);
    }
    
    // 4. Run Reed-Solomon reconstruction
    let mut shards: Vec<_> = batch.iter_mut()
        .zip(&mask)
        .map(|(s, m)| (s.erasure_shard_mut(), m))
        .collect();
    reed_solomon_cache
        .get(num_data, num_coding)?
        .reconstruct(&mut shards)?;
    
    // 5. The stubs now contain recovered data!
    //    Deserialize headers from recovered shreds
    //    Recompute Merkle proofs for recovered shreds
    
    Ok(batch)
}
```

---

## 8. Merkle Tree Chaining

### 8.1 Why Chain?

Each FEC batch generates a Merkle tree over its 64 shreds. These Merkle trees are **chained across batches** and **across slots**.

### 8.2 Chaining Within a Slot

```
Slot 42:
  ┌─────────────┐     ┌─────────────┐
  │ FEC Batch 0 │     │ FEC Batch 1 │
  │             │     │             │
  │ D0 D1...D31 │     │ D32 D33...  │
  │ C0 C1...C31 │     │ C32 C33...  │
  │             │     │             │
  │ MerkleRoot0 │────►│ MerkleRoot0 │ ← Batch 1's chained_merkle_root
  └─────────────┘     │  (stored in │     points to Batch 0's root
                       │   every     │
                       │   shred)    │
                       │             │
                       │ MerkleRoot1 │
                       └─────────────┘
```

The chained_merkle_root field in Batch 1's shreds stores the Merkle root of Batch 0.
This creates a chain: `MerkleRoot0 → MerkleRoot1 → ... → MerkleRootN`

### 8.3 Chaining Across Slots

```
Slot 41 (previous):
  last_merkle_root
       │
       ▼
Slot 42:
  Batch 0's chained_merkle_root = last_merkle_root_from_slot_41
  Batch 1's chained_merkle_root = Batch 0's MerkleRoot
  ...
  
  When the leader starts slot 42, it reads the last Merkle root from slot 41
  and uses it as the chained_merkle_root for slot 42's first batch.
```

This cryptographically links every shred in a slot, and every slot in the chain. You can prove a shred belongs to a specific slot and position by walking the Merkle chain.

---

## 9. Retransmission

When your node receives a shred and it's not the root, you must **retransmit** it to your children in the Turbine tree.

### 9.1 When to Retransmit

- Every shred that successfully passes sigverify MUST be retransmitted
- Shreds are retransmitted even if they are for slots we already have
- Exception: if turbine is disabled (feature flag)

### 9.2 The Retransmit Flow

```
Received shred ──► SigVerify passed ──► RetransmitStage
                                              │
         ┌────────────────────────────────────┤
         │                                    │
         ▼                                    ▼
  Insert into blockstore               Compute retransmit peers
  (WindowService)                      (cluster_nodes cache)
                                              │
                                              ▼
                                       For each child:
                                         - Determine protocol (UDP or QUIC)
                                         - Send the shred via multi_target_send
                                         - If last FEC batch: resign first
                                              │
                                              ▼
                                       Update metrics + slot stats
```

### 9.3 Resigning (Retransmitter Signature)

Only the **last FEC batch** of each slot has retransmitter signatures. Not all shreds — just the very last batch.

Why? So downstream nodes can verify that the retransmitter is a legitimate peer, not a spammer injecting garbage shreds.

```rust
fn resign_if_needed(shred: &mut Shred, keypair: &Keypair) {
    if shred.is_last_in_slot() && shred.is_data() {
        // This shred is in the last FEC batch
        // Sign the Merkle root with our key
        let root = shred.merkle_root().unwrap();
        let signature = keypair.sign_message(root.as_ref());
        shred.set_retransmitter_signature(&signature).unwrap();
    }
}
```

### 9.4 Address Caching

Computing retransmit addresses is expensive (weighted shuffle, gossip lookup). Agave caches the result:

- **AddrCache**: key = (slot, shred_index), value = Vec<SocketAddr>
- Cache miss: compute from scratch, populate cache
- Cache hit: fast path, just send

The cache is per-slot. When a slot is complete, its entries are evicted.

---

## 10. Repair Protocol

What happens when your node misses a shred?

### 10.1 Detection

During `blockstore.insert_shreds()`:
- Track gaps in shred indices for each slot
- If a FEC batch is mostly complete but missing some shreds, try erasure recovery
- If erasure recovery isn't possible (not enough shreds), schedule a repair request

### 10.2 Repair Request

```rust
struct RepairRequest {
    slot: Slot,
    shred_index: u32,
    shred_type: ShredType,  // Data or Code
    nonce: u32,              // Random nonce to avoid replay
}
```

The repair service:
1. Maintains a list of outstanding repair requests (`OutstandingShredRepairs`)
2. Periodically picks a peer to ask for the missing shred
3. Sends a RepairRequest to that peer's repair socket (UDP, separate port)
4. The peer responds with the shred data
5. The response includes our nonce so we can verify it's not a replay

### 10.3 Repair Flow

```
[us]                              [peer]
  │                                 │
  │── RepairRequest(slot, idx) ────►│
  │                                 │  Look up shred in blockstore
  │                                 │  Attach matching nonce
  │◄── ShredData + nonce ──────────│
  │                                 │
  │ Verify nonce matches            │
  │ Insert shred into blockstore    │
  │ Try erasure recovery again      │
```

The repair socket is separate from the main TVU socket. This prevents repair traffic from interfering with live shred reception.

---

## 11. Module Structure

Here is how we will structure our `dc-tvu` crate:

```
crates/dc-tvu/src/
├── mod.rs              — public API
├── main.rs             — entrypoint, wires everything
├── shred.rs            — Shred enum (Data/Code), parse from raw bytes
├── shred_header.rs     — ShredCommonHeader, DataShredHeader, CodingShredHeader structs
├── shredder.rs         — ReedsolomonCache, erasure coding helpers
├── deshred.rs          — reassemble shreds → entries
├── fetch.rs            — UDP socket receiver, filter packets
├── sigverify.rs        — verify leader signature on shreds
├── window.rs           — insert shreds, detect completed FEC sets
├── retransmit.rs       — forward shreds to Turbine children
├── cluster_nodes.rs    — weighted shuffle, tree topology computation
├── repair.rs           — detect gaps, request missing shreds
├── merkle.rs           — Merkle proof verification + generation
├── blockstore.rs       — simple storage for assembled blocks
├── types.rs            — shared types
└── rpc_provider.rs     — feed data to dc-rpc
```

---

## 12. Implementation Plan

### Phase 1: Receive and Parse (this sprint)

```
Goal: Bind to TVU port, receive shreds, parse headers

Files to write:
  1. shred_header.rs   — struct definitions + serde
  2. shred.rs          — parse raw bytes → Shred enum
  3. fetch.rs          — UDP socket, receive loop
  
Milestone: Can print "received data shred for slot X, index Y" with all header fields.
```

### Phase 2: Store and Reassemble

```
Goal: Collect shreds, detect complete slots, deshred into entries

Files to write:
  4. window.rs         — shred buffer, FEC set tracking
  5. deshred.rs        — concatenate shred data → deserialize entries
  6. blockstore.rs     — store assembled blocks
  
Milestone: Can dump all entries (transactions) from a complete slot.
```

### Phase 3: Erasure Recovery

```
Goal: Reconstruct missing shreds using Reed-Solomon

Files to write:
  7. shredder.rs       — ReedSolomonCache + recover()
  8. merkle.rs         — verify Merkle proofs on recovered shreds
  
Milestone: Can recover from 50% packet loss on a FEC set.
```

### Phase 4: Signature Verification

```
Goal: Verify leader signatures on incoming shreds

Files to write:
  9. sigverify.rs      — extract Merkle root, verify ed25519 sig
  
Milestone: Only accepting shreds from legitimate leaders.
```

### Phase 5: Retransmission

```
Goal: Participate in Turbine tree

Files to write:
  10. cluster_nodes.rs — weighted shuffle, tree computation
  11. retransmit.rs    — forward to children, resign last batch
  
Milestone: Other validators can discover us and we retransmit properly.
```

### Phase 6: Repair

```
Goal: Request and serve missing shreds

Files to write:
  12. repair.rs        — gap detection, peer requests
  
Milestone: No more missed shreds — repair fills all gaps.
```

---

## Key Constants

| Constant | Value | Description |
|----------|-------|-------------|
| PACKET_DATA_SIZE | 1232 | Max bytes per UDP packet payload |
| SIZE_OF_NONCE | 4 | Repair nonce is u32 appended to packet |
| ShredData::SIZE_OF_PAYLOAD | 1203 | Exact byte size of every data shred |
| ShredCode::SIZE_OF_PAYLOAD | 1228 | Exact byte size of every coding shred |
| SIZE_OF_SIGNATURE | 64 | ed25519 signature |
| SIZE_OF_COMMON_SHRED_HEADER | 83 | Common header bytes |
| SIZE_OF_DATA_SHRED_HEADERS | 88 | Common + data header bytes |
| SIZE_OF_CODING_SHRED_HEADERS | 89 | Common + coding header bytes |
| DATA_SHREDS_PER_FEC_BLOCK | 32 | Data shreds per erasure batch |
| CODING_SHREDS_PER_FEC_BLOCK | 32 | Coding shreds per erasure batch |
| MAX_DATA_SHREDS_PER_SLOT | 32768 | Upper bound on data shreds per slot |
| MAX_CODE_SHREDS_PER_SLOT | 32768 | Upper bound on coding shreds per slot |
| DATA_PLANE_FANOUT | 200 | Number of children per Turbine node |
| MAX_NUM_TURBINE_HOPS | 4 | Max depth of Turbine tree |
| Devnet shred version | 11016 | Devnet cluster identifier |

---

## Key Files in Agave Reference

| File (relative to agave/) | What We'll Learn |
|---------------------------|------------------|
| `ledger/src/shred.rs` | Shred enum, constants, ShredFlags, Error types |
| `ledger/src/shred/shred_data.rs` | Data shred parsing |
| `ledger/src/shred/shred_code.rs` | Coding shred parsing |
| `ledger/src/shred/merkle.rs` | Merkle tree root, proof, recovery |
| `ledger/src/shred/wire.rs` | Byte-offset helpers for raw shreds |
| `ledger/src/shred/payload.rs` | Payload type (wraps Bytes) |
| `ledger/src/shredder.rs` | Create shreds from entries |
| `core/src/tvu.rs` | TVU pipeline wiring |
| `core/src/window_service.rs` | Insert shreds, detect complete sets |
| `core/src/shred_fetch_stage.rs` | Receiving raw packets from network |
| `turbine/src/cluster_nodes.rs` | Turbine tree topology |
| `turbine/src/retransmit_stage.rs` | Forwarding shreds to children |
| `turbine/src/broadcast_stage/standard_broadcast_run.rs` | Leader shred creation + broadcast |
| `turbine/src/broadcast_stage/broadcast_utils.rs` | Utility functions |

---

## Glossary

| Term | Meaning |
|------|---------|
| Shred | A piece of a block, ≤ 1228 bytes, fits in one UDP packet |
| Data shred | Contains actual transaction bytes |
| Coding shred | Contains Reed-Solomon parity data for recovery |
| FEC set | A group of 32 data + 32 coding shreds, erasure coded together |
| FEC set index | Index of the first data shred in the FEC set |
| Shred version | u16 cluster identifier (different for mainnet/devnet/testnet) |
| Turbine | Tree-based broadcast protocol for block propagation |
| Fanout | Number of children each node sends to (default 200) |
| Root | The first node in the Turbine shuffle, receives directly from leader |
| Merkle root | SHA-256 root of Merkle tree over all shreds in a FEC set |
| Chained Merkle root | Merkle root of the previous FEC batch, linked cryptographically |
| Retransmitter signature | Additional signature by the forwarding node on the last FEC batch |
| Repair | Protocol for requesting and serving missing shreds |
| Deshred | Reassembling shreds back into entries (transactions) |
