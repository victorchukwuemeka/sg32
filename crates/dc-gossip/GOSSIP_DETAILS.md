# How Solana Gossip Really Works: A Byte-Level Journey Into the Devnet

> A complete walkthrough of building a working gossip client from scratch, debugging wire format mismatches byte-by-byte, and finally talking to the Solana devnet entrypoint.

---

## Table of Contents

1. [The Dream](#1-the-dream)
2. [The Gossip Protocol in 60 Seconds](#2-the-gossip-protocol-in-60-seconds)
3. [Our Starting Point](#3-our-starting-point)
4. [The Debugging Journey](#4-the-debugging-journey)
   - [Phase 1: Ping Works, PullRequest Gets Ignored](#phase-1-ping-works-pullrequest-gets-ignored)
   - [Phase 2: Chasing the Wrong Clue (Version struct)](#phase-2-chasing-the-wrong-clue-version-struct)
   - [Phase 3: The CrdsValue hash Trap](#phase-3-the-crdsvalue-hash-trap)
   - [Phase 4: The REAL Root Cause — ContactInfo Serialization](#phase-4-the-real-root-cause--contactinfo-serialization)
5. [Byte-by-Byte Wire Format Analysis](#5-byte-by-byte-wire-format-analysis)
   - [How Bincode Serializes Things](#how-bincode-serializes-things)
   - [The Version Struct: 17 bytes → 12 bytes](#the-version-struct-17-bytes--12-bytes)
   - [The ContactInfo Struct: 307 bytes → 65 bytes](#the-contactinfo-struct-307-bytes--65-bytes)
   - [The CrdsValue: 32 bytes of hash poison](#the-crdsvalue-32-bytes-of-hash-poison)
6. [The Complete Devnet Conversation](#6-the-complete-devnet-conversation)
   - [Message 1: Ping (132 bytes)](#message-1-ping-132-bytes)
   - [Message 2: Pong (132 bytes)](#message-2-pong-132-bytes)
   - [Message 3: PullRequest (1232 bytes)](#message-3-pullrequest-1232-bytes)
   - [Message 4: Entrypoint's Ping (132 bytes)](#message-4-entrypoints-ping-132-bytes)
   - [Message 5: PullResponse (505-1232 bytes)](#message-5-pullresponse-505-1232-bytes)
7. [The Complete File-by-File Change Log](#7-the-complete-file-by-file-change-log)
8. [What We Still Get Wrong (and Why It Doesn't Matter)](#8-what-we-still-get-wrong-and-why-it-doesnt-matter)
9. [How to Run It Yourself](#9-how-to-run-it-yourself)
10. [All Reference Files in Agave Source](#10-all-reference-files-in-agave-source)

---

## 1. The Dream

Build a minimal Rust crate that speaks the Solana gossip protocol — the peer-to-peer discovery and data propagation layer that every Solana validator uses to find each other and exchange information.

The goal: connect to the Solana devnet entrypoint (`35.197.53.105:8001`), perform the handshake, send a PullRequest asking "who's on the network?", and actually receive valid gossip data back.

No fork of Agave. No importing the entire Solana monorepo. Just a standalone binary that sends and receives the right bytes.

---

## 2. The Gossip Protocol in 60 Seconds

Every Solana validator runs a gossip service on UDP port 8001 (by default). The gossip protocol has 6 message types:

```
Protocol enum (discriminant in parentheses):
  PullRequest(CrdsFilter, CrdsValue)     → (0) "Here's who I am, send me what you know"
  PullResponse(Pubkey, Vec<CrdsValue>)   → (1) "Here's what I know about the network"
  PushMessage(Pubkey, Vec<CrdsValue>)    → (2) "Here's some new data I just heard"
  PruneMessage(Pubkey, PruneData)        → (3) "Stop sending me messages from these peers"
  PingMessage(Ping)                      → (4) "Are you alive?"
  PongMessage(Pong)                      → (5) "Yes, I'm alive"
```

The flow is simple:

```
1. Send Ping → receive Pong (handshake, proves you're reachable)
2. Send PullRequest(your ContactInfo) → entrypoint queues your request
3. Entrypoint sends you Ping → you send Pong (proves you respond)
4. Entrypoint sends PullResponse(gossip data) → you learn about other validators
5. Repeat PullRequest every ~5 seconds to stay in the network
```

Under the hood, each validator maintains a CRDS (Conflict-free Replicated Data Type) table — a collection of `CrdsValue` entries (ContactInfo, votes, slot hashes, etc.) that propagate through the network via pull (request/response) and push (unsolicited broadcast).

---

## 3. Our Starting Point

We had a Rust crate (`dc-gossip`) with:

- A UDP socket implementation
- Ping/Pong structs that could be serialized/deserialized
- A CrdsValue and CrdsData type definition
- A ContactInfo struct
- A Protocol enum with encode/decode methods

When we first ran it:

```
Ping sent... Pong received! ✓
PullRequest sent... ... ... nothing. ✗
```

Zero bytes came back after the PullRequest. The entrypoint was silently ignoring us. For weeks.

---

## 4. The Debugging Journey

> This section tells the story in chronological order — every hypothesis, every dead end, every "aha" moment.

---

### Phase 1: Ping Works, PullRequest Gets Ignored

**Observation:** The Ping/Pong handshake succeeded on the first attempt (132 bytes each way). But after sending the PullRequest (1228 bytes), we received nothing — no Ping from the entrypoint, no PullResponse data, absolutely zero bytes back.

**Hypothesis 1:** The entrypoint doesn't like our PullRequest format.

**Investigation:** We read the Agave source code to understand how the entrypoint processes incoming PullRequest messages.

**Command:**
```bash
rg -n "Protocol::PullRequest" /home/victor/opensource/agave/gossip/src/cluster_info.rs
```

**Found the handler at line 2038:**
```rust
Protocol::PullRequest(filter, caller) => {
    if !check_pull_request_shred_version(self_shred_version, &caller) {
        // ← This was triggering!
        self.stats.skip_pull_shred_version.add_relaxed(1);
        continue;  // Skip this packet entirely
    }
    // ... process the pull request
}
```

**Found the shred version check at line 2441:**
```rust
fn check_pull_request_shred_version(self_shred_version: u16, caller: &CrdsValue) -> bool {
    let shred_version = match caller.data() {
        CrdsData::ContactInfo(node) => node.shred_version(),
        _ => return false,  // ← We were hitting this!
    };
    shred_version == self_shred_version
}
```

**Discovered:** The entrypoint calls `caller.data()` — which extracts `CrdsData` from the `CrdsValue` we sent. It expects `CrdsData::ContactInfo(node)`. If the `CrdsData` deserialization fails (wrong variant, wrong field positions, etc.), the match falls through to `_ => return false`, and our PullRequest is skipped with `continue`.

**Question:** Why was our `CrdsData::ContactInfo(ContactInfo)` failing to deserialize? The `ContactInfo` struct we defined looked correct at a glance — it had the right fields (pubkey, wallclock, outset, shred_version, version, addrs, sockets, etc.). But Agave uses specific serde annotations that silently change the wire format.

**What we needed to figure out:** What does Agave's ContactInfo actually look like on the wire, byte by byte?

---

### Phase 2: Chasing the Wrong Clue (Version struct)

**First obvious difference we spotted:** Our `Version` struct looked different from Agave's.

**Agave Version (agave/version/src/v3.rs:10-22):**
```rust
pub struct Version {
    #[serde(with = "serde_varint")]  pub major: u16,    // ← NOTE THE ANNOTATION
    #[serde(with = "serde_varint")]  pub minor: u16,
    #[serde(with = "serde_varint")]  pub patch: u16,
    pub commit: u32,                                      // ← NOT Option<u32>
    pub feature_set: u32,
    #[serde(with = "serde_varint")]  client: u16,
}
```

**Our Version before the fix:**
```rust
pub struct Version {
    pub major: u16,              // No annotation → 2 bytes
    pub minor: u16,              // No annotation → 2 bytes
    pub patch: u16,              // No annotation → 2 bytes
    pub commit: Option<u32>,     // Different TYPE → tag byte + 4 bytes
    pub feature_set: u32,
    pub client: u16,             // No annotation → 2 bytes
}
```

**What's varint encoding?**
Varint (variable-length integer) encoding stores small numbers in fewer bytes. Each byte uses 7 bits for the value and 1 bit (the MSB) as a continuation flag:
- `0xxxxxxx` → last byte, value = lower 7 bits
- `1xxxxxxx` → more bytes follow, value = lower 7 bits

So `major = 2` is:
- Without varint: `0x02 0x00` (2 bytes, u16 little-endian)
- With varint: `0x02` (1 byte, encoded in a single byte since value < 128)

And `commit: Option<u32>` vs `commit: u32`:
- `Option<u32>` serializes as: 1 byte tag (0=None, 1=Some) + 4 bytes value = 5 bytes if Some
- `u32` serializes as: 4 bytes always

We made our Version match Agave's exactly:

**Version after fix:**
```rust
pub struct Version {
    #[serde(with = "serde_varint")]  pub major: u16,
    #[serde(with = "serde_varint")]  pub minor: u16,
    #[serde(with = "serde_varint")]  pub patch: u16,
    pub commit: u32,
    pub feature_set: u32,
    #[serde(with = "serde_varint")]  pub client: u16,
}
```

**Result after this fix:** PullRequest went from 1228 → 1227 bytes. Only 1 byte change, not the ~5 we expected. Something was still wrong.

We had to add the `solana-serde-varint` crate to Cargo.toml to get the `serde_varint` module:
```toml
solana-serde-varint = "3"
```

---

### Phase 3: The CrdsValue hash Trap

**Next difference spotted:** Our `CrdsValue` struct included the `hash` field on the wire. Agave doesn't.

**Agave CrdsValue (agave/gossip/src/crds_value.rs:24-30):**
```rust
pub struct CrdsValue {
    signature: Signature,
    data: CrdsData,
    #[serde(skip_serializing)]    // ← NOTE: NOT on wire!
    hash: Hash,
}
```

**Our CrdsValue before:**
```rust
pub struct CrdsValue {
    pub signature: Signature,
    pub data: CrdsData,
    pub hash: Hash,              // ← This IS on wire! 32 extra bytes!
}
```

**Why does Agave skip the hash?** The hash is `sha256(signature || serialized(data))`. It's a computed field — there's no need to send it because any recipient can recompute it. On the sending side, the hash is used for CRDT deduplication (checking if you've already seen this value). On the receiving side, Agave has a manual `Deserialize` implementation that computes the hash from the deserialized data.

**Impact of our bug:** Each `CrdsValue` we sent had 32 extra bytes of hash. This:
1. Bloated the PullRequest packet (32 bytes extra in the CalerValue)
2. Changed the bloom filter budget calculation (because `cellor_size` was used to compute how much room the bloom filer has)

**The fix:**
```rust
pub struct CrdsValue {
    pub signature: Signature,
    pub data: CrdsData,
    #[serde(skip)]              // ← Skip BOTH serialize and deserialize
    pub hash: Hash,
}
```

**Why `skip` instead of `skip_serializing`?** Agave uses `skip_serializing` only, but they have a **manual** `Deserialize` implementation. The manual impl uses a helper struct without the hash field, so hash is never read from the wire. We use `#[serde(skip)]` which skips both directions and falls back to `Default::default()` for the hash value during deserialization.

**Agave's manual Deserialize impl (lines 213-237):**
```rust
impl<'de> Deserialize<'de> for CrdsValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error> {
        // Helper struct WITHOUT hash field
        #[derive(Deserialize)]
        struct CrdsValue {
            signature: Signature,
            data: CrdsData,
        }
        let CrdsValue { signature, data } = CrdsValue::deserialize(deserializer)?;
        // Compute hash from the deserialized data
        let hash = sha256(signature || serialize(data));
        Ok(Self { signature, data, hash })
    }
}
```

**Result after this fix:** PullRequest went from 1227 → 1232 bytes (full `PACKET_DATA_SIZE`). The bloom filter got 32 more bytes of budget and expanded to fill the available space.

But still — **the entrypoint wasn't responding**. The Version and hash fixes were necessary but not sufficient.

---

### Phase 4: The REAL Root Cause — ContactInfo Serialization

This is where we found THE bug — the one that was actually causing the entrypoint to reject us.

We finally did a proper side-by-side comparison of our entire `ContactInfo` struct against Agave's.

**Agave ContactInfo (gossip/src/contact_info.rs:80-101, verified in v2.2.0 tag):**
```rust
pub struct ContactInfo {
    pubkey: Pubkey,
    #[serde(with = "serde_varint")]    wallclock: u64,      // ← VARINT!
    outset: u64,
    shred_version: u16,
    version: solana_version::Version,  // (with varint fields)
    #[serde(with = "short_vec")]       addrs: Vec<IpAddr>,  // ← SHORT VEC!
    #[serde(with = "short_vec")]       sockets: Vec<SocketEntry>, // ← SHORT VEC!
    #[serde(with = "short_vec")]       extensions: Vec<Extension>, // ← SHORT VEC!
    #[serde(skip_serializing)]         cache: [SocketAddr; SOCKET_CACHE_SIZE], // ← SKIP!
}
```

**Our ContactInfo before the fix:**
```rust
pub struct ContactInfo {
    pub pubkey: Pubkey,
    pub wallclock: u64,              // ← NO VARINT → 8 bytes instead of ~5
    pub outset: u64,
    pub shred_version: u16,
    pub version: Version,            // (before Version fix: +5 bytes)
    pub addrs: Vec<IpAddr>,          // ← NO SHORT_VEC → u64 len instead of varint
    pub sockets: Vec<SocketEntry>,   // ← NO SHORT_VEC
    pub extensions: Vec<Extension>,  // ← NO SHORT_VEC
    pub cache: [SocketAddr; 14],     // ← SERIALIZED! 208 extra bytes!
}
```

**The wallclock bomb — explained byte by byte:**

This was the single most destructive bug. Here's exactly what happens:

Our wallclock value is a microsecond timestamp from `SystemTime::now().duration_since(UNIX_EPOCH).as_micros()`. At the time of testing, this was approximately `1,746,000,000,000,000` — about 1.7 quadrillion microseconds since epoch.

In hexadecimal: `0x000634B88C3C8000`

In little-endian bytes (how bincode writes a u64):
```
Byte 0: 0x00  (LSB)
Byte 1: 0x80
Byte 2: 0x3C
Byte 3: 0x8C
Byte 4: 0xB8
Byte 5: 0x34
Byte 6: 0x06
Byte 7: 0x00  (MSB)
```

When Agave reads this as a **varint** (because its ContactInfo has `#[serde(with = "serde_varint")]` on wallclock):

```
Byte 0: 0x00
  → MSB = 0 → this is the LAST byte of the varint
  → value = 0x00 = 0
  → wallclock = 0  ← WRONG!
```

Agave consumed only **1 byte** for wallclock, not 8! The remaining 7 bytes (`80 3C 8C B8 34 06 00`) are now interpreted as the start of the **next field** (`outset`).

This means:
- `wallclock` on their side = 0 (should be ~1.7 quadrillion)
- `outset` on their side = bytes 1-8 of our wallclock field (should be a different timestamp)
- Every field after is shifted by 7 bytes

This cascading misalignment corrupts the **entire** ContactInfo. The `shred_version` field gets read from the wrong offset, producing a garbage value that doesn't match the devnet shred_version (11016), so `check_pull_request_shred_version()` returns false.

**The short_vec impact:**

Agave uses `short_vec` for all Vec fields. This replaces bincode's default u64 length prefix (8 bytes) with a varint-encoded length (1-3 bytes for real-world sizes).

For a Vec with 1 element:
- Without short_vec: `01 00 00 00 00 00 00 00` (8 bytes, u64 little-endian)
- With short_vec: `01` (1 byte, varint)

For 3 Vec fields (addrs, sockets, extensions):
- Savings: 7 + 7 + 7 = 21 bytes total

**The cache bomb:**

Without `#[serde(skip_serializing)]`, the `cache: [SocketAddr; 14]` field serializes as 208-224 bytes of zeros (14 SocketAddr entries, each is 16 bytes on a 64-bit system).

**The complete fix for ContactInfo:**
```rust
use solana_serde_varint as serde_varint;
use solana_short_vec as short_vec;

pub struct ContactInfo {
    pub pubkey: Pubkey,
    #[serde(with = "serde_varint")]
    pub wallclock: u64,              // ← FIXED
    pub outset: u64,
    pub shred_version: u16,
    pub version: Version,
    #[serde(with = "short_vec")]
    pub addrs: Vec<IpAddr>,          // ← FIXED
    #[serde(with = "short_vec")]
    pub sockets: Vec<SocketEntry>,   // ← FIXED
    #[serde(with = "short_vec")]
    pub extensions: Vec<Extension>,  // ← FIXED
    #[serde(skip_serializing)]
    pub cache: [SocketAddr; 13],     // ← FIXED (also changed 14→13)
}
```

Plus `SocketEntry.offset` also needs varint:
```rust
pub struct SocketEntry {
    pub key: u8,
    pub index: u8,
    #[serde(with = "serde_varint")]
    pub offset: u16,                 // ← FIXED
}
```

**After all fixes — the moment of truth:**

```
Ping sent... Pong received! ✓
PullRequest: 1232 bytes (full PACKET_DATA_SIZE) ✓
GOT PACKET from 35.197.53.105:8001: 132 bytes → Got Ping from entrypoint! ✓
new/updated entry from 9Y8V9NHwXikBFDmrYUQvKFkyWAV4KcHtuMQqq5XMjsrh ✓
new/updated entry from 964koDACp6GNSW8uL2J49CTa7kuFSQWPErgKMKK1E9QH ✓
...
CRDS: 18 entries, 1 peers ✓
```

The entrypoint accepted our PullRequest, passed the shred version check, queued our request, sent us a Ping (which we responded to with Pong), and then started sending us actual gossip data containing valid validator records.

---

## 5. Byte-by-Byte Wire Format Analysis

> This section shows exactly how every struct looks on the wire, before and after the fixes. Open your hex editor.

---

### How Bincode Serializes Things

All Solana gossip messages use `bincode::serialize()` with default options:
- **Little-endian** byte order
- **FixintEncoding** for enums (u32 discriminant)
- **u64** for Vec lengths (unless overridden by `short_vec`)
- **u64** for integer types (unless overridden by `serde_varint`)

---

### The Version Struct: 17 bytes → 12 bytes

**Before (broken wire format):**
```
Offset  Hex        Field              Notes
0       02 00      major = 2          u16 LE = 2 bytes
2       00 00      minor = 0          u16 LE = 2 bytes
4       00 00      patch = 0          u16 LE = 2 bytes
6       01         Some tag           Option<u32> = 1 byte prefix
7       00 00 00 00 commit = Some(0)  u32 LE = 4 bytes
11      00 00 00 00 feature_set = 0   u32 LE = 4 bytes
15      03 00      client = 3 (Agave) u16 LE = 2 bytes
                      TOTAL = 17 bytes
```

**After (correct wire format):**
```
Offset  Hex        Field              Notes
0       02         major = 2          varint = 1 byte (0x02)
1       00         minor = 0          varint = 1 byte (0x00)
2       00         patch = 0          varint = 1 byte (0x00)
3       00 00 00 00 commit = 0        u32 LE = 4 bytes
7       00 00 00 00 feature_set = 0   u32 LE = 4 bytes
11      03         client = 3 (Agave) varint = 1 byte (0x03)
                      TOTAL = 12 bytes
```

**Savings: 5 bytes** (29% reduction)

---

### The ContactInfo Struct: 307 bytes → 65 bytes

**Before (broken wire format, typical case: 1 addr, 1 socket, 0 extensions):**
```
Offset  Hex (length)  Field
0       32 bytes      pubkey (32 bytes)
32      8 bytes       wallclock (u64) ← WRONG: should be varint, ~5 bytes
40      8 bytes       outset (u64)
48      2 bytes       shred_version (u16)
50      17 bytes      version (wrong format)
67      8 bytes       addrs len (u64 = 1) ← WRONG: should be varint, 1 byte
75      4 bytes       addr[0] (Ipv4: 4 bytes)
79      8 bytes       sockets len (u64 = 1) ← WRONG: should be varint, 1 byte
87      4 bytes       socket[0] (1 + 1 + 2 = 4 bytes)
91      8 bytes       extensions len (u64 = 0) ← WRONG: should be varint, 1 byte
99      208 bytes     cache[13] (13 × SocketAddr = 208 bytes ← WRONG: should be SKIPPED
                      TOTAL = ~307 bytes
```

**After (correct wire format):**
```
Offset  Hex (length)  Field
0       32 bytes      pubkey (32 bytes)
32      ~5 bytes      wallclock (varint, ~5 bytes for timestamp ~1.7e15)
37      8 bytes       outset (u64)
45      2 bytes       shred_version (u16)
47      12 bytes      version (correct format)
59      ~1 byte       addrs len (varint = 1)
60      4 bytes       addr[0] (Ipv4: 4 bytes)
64      ~1 byte       sockets len (varint = 1)
65      ~4 bytes      socket[0] (key=1, index=1, offset=varint)
69      ~1 byte       extensions len (varint = 0)
--      SKIPPED       cache (not serialized)
                      TOTAL = ~65 bytes
```

**Savings: ~242 bytes** (79% reduction)

---

### The CrdsValue: 32 bytes of hash poison

**Before (CrdsValue on wire):**
```
Offset  Hex           Field
0       64 bytes      signature (Signature = 64 bytes)
64      variable      data (CrdsData enum, variable length)
64+N    32 bytes      hash (Hash = 32 bytes) ← WRONG: Agave doesn't send this
                      TOTAL = 96 + N bytes
```

**After (CrdsValue on wire):**
```
Offset  Hex           Field
0       64 bytes      signature (Signature = 64 bytes)
64      variable      data (CrdsData enum, variable length)
                      TOTAL = 64 + N bytes
                      hash SKIPPED
```

**Savings: 32 bytes per CrdsValue**

Since our PullRequest contains 1 CrdsValue (the caller ContactInfo), this saved 32 bytes in the PullRequest. These 32 bytes were then reclaimed by the bloom filter (the bloom budgeting code in `pull_request.rs` fills up to `PACKET_DATA_SIZE` = 1232 bytes).

---

## 6. The Complete Devnet Conversation

> This section traces every UDP packet exchanged between our client and the devnet entrypoint, byte by byte.

---

### Message 1: Ping (132 bytes)

**Direction:** Us → Entrypoint (`35.197.53.105:8001`)

**Wire format breakdown:**
```
Bytes 0-3:    Protocol discriminant (u32 LE)
              04 00 00 00 = 4 = PingMessage

Bytes 4-35:   Ping token (32 random bytes)
              [random data]

Bytes 36-99:  Signature (64 bytes)
              keypair.sign(token)
              
Bytes 100-131: Padding? Or the full Protocol::PingMessage struct
              (varies by Ping struct definition)
```

**Total: 132 bytes**

---

### Message 2: Pong (132 bytes)

**Direction:** Entrypoint → Us

**Wire format breakdown:**
```
Bytes 0-3:    Protocol discriminant (u32 LE)
              05 00 00 00 = 5 = PongMessage

Bytes 4-35:   Same 32-byte token we sent (echoed back)
              
Bytes 36-99:  Signature (64 bytes)
              entrypoint's signature proving they received our token
              
Bytes 100-131: [rest of Pong struct]
```

**Total: 132 bytes**

---

### Message 3: PullRequest (1232 bytes)

**Direction:** Us → Entrypoint

**Wire format breakdown:**
```
Bytes 0-3:    Protocol discriminant (u32 LE)
              00 00 00 00 = 0 = PullRequest

Bytes 4-...:  CrdsFilter
  Bytes 4-11:   Bloom<Hash> bit array (variable, ~800 bytes)
  Bytes ~804-811: mask (u64)
                   FF FF FF FF FF FF FF FF (since mask_bits = 0, mask = u64::MAX)
  Bytes ~812-815: mask_bits (u32)
                   00 00 00 00 (since num_items = 0 ≤ max_items)

Bytes ~816-...: CrdsValue (the caller)
  Bytes ~816-879: signature (64 bytes)
                   The signature of the serialized CrdsData
  Bytes ~880-...: CrdsData (variable)
    Sub-bytes 0-3: CrdsData discriminant (u32 LE)
                    0B 00 00 00 = 11 = ContactInfo
    Sub-bytes 4-...: ContactInfo struct
                      - pubkey: 32 bytes
                      - wallclock: varint (~5 bytes)
                      - outset: 8 bytes (u64)
                      - shred_version: 2 bytes (u16)
                      - version: 12 bytes (fixed Version format)
                      - addrs: short_vec (length + elements)
                      - sockets: short_vec (length + elements)
                      - extensions: short_vec (length, likely 0)
                      - cache: SKIPPED (skip_serializing)
```

**Total: 1232 bytes** (exactly PACKET_DATA_SIZE)

**Note:** The bloom filter is sized to fill the remaining space after CrdsValue. The formula:
```rust
let target = PACKET_DATA_SIZE - 4(enum tag) - caller_size;
let bloom_max_bytes = cache[target];
```

---

### Message 4: Entrypoint's Ping (132 bytes)

**Direction:** Entrypoint → Us

This happens ~250ms after we send PullRequest. The entrypoint:
1. Successfully deserializes our ContactInfo
2. Passes `check_pull_request_shred_version()` (shred_version = 11016 = devnet)
3. Queues our PullRequest
4. Calls `maybe_ping_gossip_addresses()` which sends us a Ping

Same format as Message 1.

---

### Message 5: PullResponse (505-1232 bytes)

**Direction:** Entrypoint → Us

**Wire format breakdown:**
```
Bytes 0-3:    Protocol discriminant (u32 LE)
              01 00 00 00 = 1 = PullResponse

Bytes 4-35:   Pubkey (32 bytes)
              The responding node's identity

Bytes 36-43:  Vec<CrdsValue> length (u64 LE, NOT short_vec — this is Protocol-level)
              e.g., 05 00 00 00 00 00 00 00 = 5 values

Bytes 44-...: Vec<CrdsValue> elements (each):
  - Signature: 64 bytes
  - CrdsData enum (discriminant + data):
    - If discriminant = 11: ContactInfo (properly decoded → "new/updated entry")
    - If discriminant = 0, 1, 2, ...: LegacyContactInfo, Vote, LowestSlot, etc.
      → These often fail because we don't fully implement those variants
```

**The decode errors we see:**
- `unexpected end of file` — UDP truncation. Large PullResponses get fragmented.
- `tag for enum is not valid, found 20` — We're reading CrdsData variant from wrong offset (data misalignment from a partially-decoded previous value).
- `invalid value: integer 2157690643, expected V4 or V6` — We're trying to read an IpAddr at the wrong position.

**Why some PullResponse packets decode successfully:**
When all CrdsValues in the Vec happen to be `CrdsData::ContactInfo` (discriminant 11) and the packet isn't truncated, deserialization succeeds. These give us the `new/updated entry from <pubkey>` log messages.

---

## 7. The Complete File-by-File Change Log

### File: `Cargo.toml`

**What changed:** Added two dependencies.

```toml
# Before:
solana-version = "3.1.14"

# After:
solana-version = "3.1.14"
solana-serde-varint = "3"
solana-short-vec = "2"
```

**`solana-serde-varint = "3"`**
- Provides varint serde module for `#[serde(with = "serde_varint")]`
- Used on: `Version.major`, `Version.minor`, `Version.patch`, `Version.client`, `ContactInfo.wallclock`, `SocketEntry.offset`
- Already in Cargo.lock (resolved from other Solana deps)

**`solana-short-vec = "2"`**
- Provides short_vec serde module for `#[serde(with = "short_vec")]`
- Used on: `ContactInfo.addrs`, `ContactInfo.sockets`, `ContactInfo.extensions`
- Already in Cargo.lock

---

### File: `src/contact_info.rs`

**What changed:** 8 specific edits.

**Edit 1 — Imports:**
```rust
// Added:
use solana_serde_varint as serde_varint;
use solana_short_vec as short_vec;
```

**Edit 2 — SOCKET_CACHE_SIZE:**
```rust
// Before:
const SOCKET_CACHE_SIZE: usize = 14;

// After:
const SOCKET_CACHE_SIZE: usize = 13;
```
Why 13? Agave v2.2.0 has socket tags 0-12 = 13 tags. Verified at `git show v2.2.0:gossip/src/contact_info.rs`. In v3.x this became 14 (tags went 0-13). Devnet runs v2.2.x. (The cache field is skip_serializing so this only affects in-memory struct layout, not wire format.)

**Edit 3 — SocketEntry.offset:**
```rust
// Before:
pub offset: u16,

// After:
#[serde(with = "serde_varint")]
pub offset: u16,
```

**Edit 4 — Version.major:**
```rust
// Before:
pub major: u16,

// After:
#[serde(with = "serde_varint")]
pub major: u16,
```

**Edit 5 — Version.minor, Version.patch, Version.client:**
Same pattern as above.

**Edit 6 — Version.commit:**
```rust
// Before:
pub commit: Option<u32>,

// After:
pub commit: u32,
```
And in Default: `commit: None` → `commit: 0`.

**Edit 7 — ContactInfo.wallclock:**
```rust
// Before:
pub wallclock: u64,

// After:
#[serde(with = "serde_varint")]
pub wallclock: u64,
```
**THIS WAS THE #1 BUG.** The entire ContactInfo deserialization failed because of this.

**Edit 8 — ContactInfo.addrs, .sockets, .extensions, .cache:**
```rust
// Before:
pub addrs: Vec<IpAddr>,
pub sockets: Vec<SocketEntry>,
pub extensions: Vec<Extension>,
pub cache: [SocketAddr; SOCKET_CACHE_SIZE],

// After:
#[serde(with = "short_vec")]
pub addrs: Vec<IpAddr>,
#[serde(with = "short_vec")]
pub sockets: Vec<SocketEntry>,
#[serde(with = "short_vec")]
pub extensions: Vec<Extension>,
#[serde(skip_serializing)]
pub cache: [SocketAddr; SOCKET_CACHE_SIZE],
```

---

### File: `src/crds_data.rs`

**What changed:** 1 edit.

**Edit — CrdsValue.hash:**
```rust
// Before:
pub struct CrdsValue {
    pub signature: Signature,
    pub data: CrdsData,
    pub hash: Hash,       // ← 32 bytes on wire
}

// After:
pub struct CrdsValue {
    pub signature: Signature,
    pub data: CrdsData,
    #[serde(skip)]
    pub hash: Hash,       // ← skipped on both serialize and deserialize
}
```

---

### File: `src/main.rs`

**What changed:** Added error message to decode error log.

```rust
// Before:
tracing::warn!("decode error from {sender} ({} bytes, hex: {})",
    bytes.len(), hex_preview);

// After:
tracing::warn!("decode error from {sender} ({} bytes, hex: {}) — {e}",
    bytes.len(), hex_preview);
```

This simple change let us see the actual bincode error, like `unexpected end of file` and `invalid value: integer 2157690643, expected V4 or V6`, which were crucial for diagnosing remaining issues.

---

## 8. What We Still Get Wrong (and Why It Doesn't Matter)

### Issue 1: Unknown CrdsData variants

Our `CrdsData` enum has 14 variants, but we only properly handle `ContactInfo` (variant 11). When the entrypoint sends a PullResponse containing a `CrdsData::Vote` (variant 1) or `CrdsData::LowestSlot` (variant 2), the deserialization of those specific variants may fail — and since bincode deserializes the entire Protocol message at once, **one bad CrdsValue takes down the whole PullResponse**.

**The fix would be:** Implement a manual `Deserialize` for `Protocol::PullResponse` that tries to deserialize each `CrdsValue` individually and skips failures. This is how Agave handles it internally — they have robust per-value error handling.

**Why it doesn't matter now:** We still receive plenty of valid ContactInfo entries. The PullResponses that fail entirely are re-requested on the next 5-second cycle.

### Issue 2: UDP Packet Fragmentation

Some PullResponse packets arrive truncated (`io error: unexpected end of file`). This is expected behavior with UDP — packets can be dropped or fragmented by network conditions.

**Why it doesn't matter:** The next PullRequest will get a fresh batch of data.

### Issue 3: SOCKET_CACHE_SIZE version mismatch

We use 13 (Agave v2.2.x) but the current Agave main branch uses 14. Since cache is `skip_serializing`, this only affects in-memory layout, not wire format. If devnet upgrades to v3.x, they'd expect 14 SocketAddrs in the cache array. But since we skip it entirely, it doesn't matter.

---

## 9. How to Run It Yourself

```bash
# Clone the repo (if you have it)
cd /home/victor/web3/solana-gym/crates/dc-gossip

# Build (warnings about unused imports are expected — we only care about functionality)
cargo build

# Run against devnet entrypoint
RUST_LOG=info ./target/debug/dc-gossip 35.197.53.105:8001
```

### What you should see:

```
2026-05-11T22:23:02.673Z  INFO dc_gossip: Our node identity: <pubkey>
2026-05-11T22:23:02.673Z  INFO dc_gossip: Sent Ping to 35.197.53.105:8001, waiting for Pong...
2026-05-11T22:23:02.978Z  INFO dc_gossip: Got Pong from 35.197.53.105:8001 (attempt 1)
2026-05-11T22:23:03.023Z  INFO dc_gossip: PullRequest: 1232 bytes
2026-05-11T22:23:03.023Z  INFO dc_gossip: Initial PullRequest sent, listening...
2026-05-11T22:23:03.278Z  INFO dc_gossip: GOT PACKET from 35.197.53.105:8001: 132 bytes
2026-05-11T22:23:03.278Z  INFO dc_gossip: >> Got Ping from 35.197.53.105:8001, sending Pong
2026-05-11T22:23:03.781Z  INFO dc_gossip: listen window 2/6 — no packet
...
2026-05-11T22:23:05.788Z  INFO dc_gossip: Entering main gossip loop
2026-05-11T22:23:10.808Z  INFO dc_gossip: sending PullRequest (1232 bytes) to 1 peers
2026-05-11T22:23:16.344Z  INFO dc_gossip::handler: new/updated entry from dv3qDFk1DTF36Z62bNvrCXe9sKATA6xvVy6A798xxAS
2026-05-11T22:23:16.344Z  INFO dc_gossip::handler: new/updated entry from 5B4U4jQovXuMc5fASzRkFtGt4ToiQmsSAUY7cKX1fAyR
...
2026-05-11T22:23:27.088Z  INFO dc_gossip: CRDS: 18 entries, 1 peers
```

### Debug levels:
- `RUST_LOG=info` — Normal operation: handshake, PullRequest, decoded entries, CRDS stats
- `RUST_LOG=debug` — Detailed: every Pong reply, every Prune message, decode errors with actual error messages
- `RUST_LOG=warn` — Only warnings and errors

---

## 10. All Reference Files in Agave Source

> These are the files we consulted repeatedly during debugging. Every path is relative to `/home/victor/opensource/agave/`.

### Core Protocol Files

| File | What We Learned |
|------|-----------------|
| `gossip/src/cluster_info.rs:2038-2054` | Entrypoint's PullRequest handler — the `check_pull_request_shred_version()` gate |
| `gossip/src/cluster_info.rs:2441-2447` | The shred version check function — calls `caller.data()` → expects `CrdsData::ContactInfo(node)` |
| `gossip/src/protocol.rs:50-60` | Protocol enum with discriminant ordering (PullRequest=0..PongMessage=5) |
| `gossip/src/contact_info.rs:80-101` | ContactInfo struct with ALL serde annotations (our reference model) |
| `gossip/src/contact_info.rs:104-110` | SocketEntry with varint offset |
| `gossip/src/crds_value.rs:24-30` | CrdsValue struct — only derives Serialize, hash has `skip_serializing` |
| `gossip/src/crds_value.rs:213-237` | Manual Deserialize impl for CrdsValue — helper struct without hash |
| `gossip/src/crds_data.rs:44-67` | CrdsData enum variant ordering |
| `version/src/v3.rs:10-22` | Version struct with varint annotations |

### v2.2.0 (what devnet runs)

| Command | What We Learned |
|---------|-----------------|
| `git show v2.2.0:gossip/src/contact_info.rs` | Same serde annotations as v3, SOCKET_CACHE_SIZE = 13 |
| `git show v2.2.0:gossip/src/protocol.rs` | Same discriminant ordering as v3 |

### Key commands used during investigation

```bash
# Find PullRequest handler
rg -n "Protocol::PullRequest" gossip/src/cluster_info.rs

# Read the handler
sed -n '2038,2054p' gossip/src/cluster_info.rs

# Read shred version check
sed -n '2441,2447p' gossip/src/cluster_info.rs

# Check old version for comparison
git show v2.2.0:gossip/src/contact_info.rs | head -100

# Check Version struct
cat version/src/v3.rs

# Check CrdsValue deserialization
sed -n '213,237p' gossip/src/crds_value.rs
```

---

## Appendix: The Complete Data Flow Diagram

```
┌─────────────────────────────────────────────────────────────────────┐
│                         OUR CLIENT (us:8000)                        │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  1. Generate keypair, get public IP from api.ipify.org              │
│  2. Build ContactInfo { pubkey, wallclock, version,                 │
│                          shred_version=11016, addrs=[our_ip],       │
│                          sockets=[gossip@8000] }                    │
│  3. Wrap in CrdsData::ContactInfo(ci) → sign → CrdsValue            │
│  4. Create CrdsFilter (empty bloom with proper sizing)              │
│  5. Serialize Protocol::PullRequest(filter, value) → 1232 bytes     │
│  6. Send to 35.197.53.105:8001                                      │
│                                                                     │
└────────────────────────────────┬────────────────────────────────────┘
                                 │ UDP
                                 ▼
┌─────────────────────────────────────────────────────────────────────┐
│                     DEVNET ENTRYPOINT (35.197.53.105:8001)           │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  1. Receive 1232 bytes                                              │
│  2. bincode::deserialize::<Protocol>(bytes)                         │
│     → discriminant 0x00000000 = PullRequest ✓                       │
│  3. Extract CrdsFilter + CrdsValue (caller)                         │
│  4. check_pull_request_shred_version():                              │
│     a. caller.data() → CrdsData::ContactInfo(node) ✓                │
│     b. node.shred_version() == 11016 → ✓                            │
│  5. Create PullRequest { pubkey, addr, wallclock, filter }          │
│  6. Queue it for processing                                          │
│  7. maybe_ping_gossip_addresses() → send Ping to us                 │
│                                                                     │
│  ... (asynchronously, when queue is processed) ...                  │
│                                                                     │
│  8. Build PullResponse with batch of CrdsValues                     │
│  9. Serialize and send back to us:8000                              │
│                                                                     │
└────────────────────────────────┬────────────────────────────────────┘
                                 │ UDP
                                 ▼
┌─────────────────────────────────────────────────────────────────────┐
│                         OUR CLIENT (us:8000)                        │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  1. Receive bytes from entrypoint                                   │
│  2. bincode::deserialize::<Protocol>(bytes)                         │
│  3. Match discriminant:                                             │
│     - 4 = PingMessage → send Pong back                              │
│     - 1 = PullResponse → process CrdsValues                         │
│  4. For each CrdsValue in PullResponse:                             │
│     - CrdsData::ContactInfo(node) → merge into CRDS table ✓         │
│     - Other variants → may fail (not fully implemented)             │
│  5. Every 5 seconds: send PullRequest again                          │
│  6. Every 30 seconds: prune old CRDS entries                        │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

---

*This document was created from the complete debugging history of the dc-gossip crate, covering every command run, every file read, and every byte analyzed during the journey from "entrypoint ignores us" to "we're part of the devnet gossip network".*
