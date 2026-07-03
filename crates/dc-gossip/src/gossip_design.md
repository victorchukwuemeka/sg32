# dc-gossip design

> Target network: Solana devnet  
> Transport: async tokio UDP  
> On-wire format: bincode + short_vec + serde_varint (wincode-compatible)  
> Library entry: `lib.rs` `run_gossip_loop()`  
> Standalone binary: `main.rs`  

---

## What this module does

`dc-gossip` connects to the Solana devnet gossip network, communicates
using the real Solana gossip wire protocol, and emits structured events
that other crates (`dc-tvu`, `dc-cli`, `dc-ledger`) subscribe to.

It is not a full validator. It does not vote. It does not produce blocks.
It listens, merges state, and tells the rest of the system what it learned.

---

## Module structure

```
crates/dc-gossip/src/
├── gossip_design.md       — this file
├── lib.rs                 — public API: run_gossip_loop()
├── main.rs                — standalone binary with full gossip loop
│
├── contact_info.rs        — ContactInfo struct, wire serialization
├── crds_data.rs           — CrdsData enum, CrdsValue, Signable impl
├── crds_filter.rs         — CrdsFilter (bloom filter + mask bits)
├── crds.rs                — CRDS table: merge, dedup, peer extraction
├── emitter.rs             — channel types for decoupled event dispatch
├── handler.rs             — routes incoming Protocol variants to logic
├── keypair.rs             — key generation helpers
├── legacy_contact_info.rs — deprecated format, read-only
├── ping_pong.rs           — Ping + Pong + signature verification
├── protocol.rs            — Protocol enum, encode/decode
├── pull_request.rs        — build a PullRequest that fits in one UDP packet
├── transport.rs           — async UDP bind/send/recv
│
├── types.rs               — ValidatorInfo, SlotInfo, ClusterHealth
├── ip_echo.rs             — not yet wired
├── short_vec.rs           — manual ShortU16 encode/decode helpers
└── playground.rs          — scratch file, not part of the crate
```

---

## On-wire format

Solana's gossip protocol uses `wincode` on the serving side, but our
`bincode` + custom annotations produce identical bytes:

| Annotation | Effect | Wincode equivalent |
|---|---|---|
| `#[serde(with = "short_vec")]` | `Vec` length = 1–3 byte ShortU16 | `ShortU16` len encoding |
| `#[serde(with = "serde_varint")]` | Integer = LEB128 varint | `Leb128Int<u64>` |
| `#[serde(skip_serializing)]` | Field omitted on wire | `#[wincode(skip)]` |

Enum tags, fixed-width integers (`u16`, `u32`, `u64`), and byte arrays
(`Pubkey` = 32 B, `Signature` = 64 B) are identical in both.

Result: **every byte we send is exactly what Agave (Solana's validator
client) expects**.

---

## Bootstrap sequence

```
  ┌─ us ──┐                              ┌─ devnet entrypoint ──┐
  │       │  1. PingMessage ────────────► │                      │
  │       │  2. ◄──────────────────── Pong │                      │
  │       │                                │  (now in ping cache) │
  │       │  3. PullRequest ─────────────► │                      │
  │       │                                │  check shred_ver ✓  │
  │       │                                │  sanitize ✓          │
  │       │                                │  verify sig ✓        │
  │       │  4. ◄────── Ping (cache miss) │                      │
  │       │  5. Pong ────────────────────► │  (now verified)      │
  │       │                                │                      │
  │       │  6. PullRequest (5s later) ──► │  ping cache hit ✓    │
  │       │  7. ◄──────── PullResponse ── │                      │
  │       │                                │                      │
```

1. **Ping/Pong** — prove we are reachable at our UDP address. The
   entrypoint stores us in its `ping_cache`. Without this step,
   PullRequests are silently discarded (entrypoint: "who are you? I
   have never pinged you").
2. **PullRequest** — send a `CrdsFilter` (bloom filter) describing what
   we already know, plus our own `ContactInfo` (signed). The entrypoint
   responds with entries our bloom filter does **not** contain.
3. **Ping (from entrypoint)** — first PullRequest always misses the
   ping cache. The entrypoint sends a Ping; we reply with Pong. This
   caches our address for future PullRequests.
4. **Next PullRequest** — hits the ping cache, gets a `PullResponse`.

**Critical constants**:
- `DEVNET_SHRED_VERSION` = `7016` (devnet, NOT 11016 — that's mainnet)
- `PACKET_DATA_SIZE` = `1232` (UDP MTU-safe payload max)

---

## PullRequest construction

```
pull_request.rs
     │
     ├─ 1. Build CrdsValue::ContactInfo (sign with our keypair)
     │     data = CrdsData::ContactInfo(ci)
     │     sig  = keypair.sign(wincode_serialize(data))
     │
     ├─ 2. Measure caller serialized size
     │     caller_size = serialized_size(CrdsValue)
     │
     ├─ 3. Compute bloom budget
     │     available = PACKET_DATA_SIZE - 4(enum tag) - caller_size
     │     bloom_max_bytes = cache[available]  (precomputed)
     │
     ├─ 4. Create CrdsFilter with that budget & min 65536 items
     │     mask_bits = ceil(log2(65536 / max_items(bloom_bits)))
     │     filter = Bloom::random(max_items, false_rate=0.1, bloom_bits)
     │
     └─ 5. Serialize Protocol::PullRequest(filter, crds_value)
           → fits in 1232 bytes
```

The bloom filter is **empty** (no entries pre-inserted). The entrypoint
will send us everything whose hash prefix matches our `mask` — we get
all values by requesting a broad hash-space slice and inserting nothing
into the bloom.

---

## Data flow — receive path

```
UDP socket
     │
     ▼
transport.recv() → (Vec<u8>, SocketAddr)
     │
     ▼
Protocol::decode_from(bytes) → Protocol enum
     │
     ├── PushMessage(pk, values) ──┐
     ├── PullResponse(pk, values) ─┤──► handler::handle()
     ├── PingMessage(ping) ────────┘       │
     ├── PongMessage(_)                    ├── crds::merge(value) → dedup by wallclock
     └── PruneMessage(pk, _)              ├── extract gossip addrs → new_peers
                                           └── drain_events() → tx.send(event)
```

- `PushMessage` / `PullResponse` → merge `CrdsValue`s into the table
- `PingMessage` → reply with `PongMessage`
- `PongMessage` / `PruneMessage` → logged (no action yet)

---

## Data flow — transmit path (gossip loop)

Every **5 seconds** (configurable in `main.rs`):
```
1. Build ContactInfo (current wallclock, public IP, shred_version)
2. Build PullRequest (bloom filter sized to 1232 bytes)
3. Send to known_peers (all discovered gossip addresses)
```

Every **30 seconds**:
```
1. crds_table.prune() — remove stale entries
2. tx.send(ci) — emit ContactInfos to subscribers
3. update known_peers from CRDS table
```

---

## CRDS table — crds.rs

```
Merge rule: higher wallclock wins
Prune: entries with wallclock older than 15 min
Index: by Pubkey (one entry per validator)
Events: drain_events() returns gossip events from last mutation
```

```rust
// Actual signatures (not the design doc's stale mock):
impl CrdsTable {
    pub fn new() -> Self
    pub fn merge(&mut self, value: CrdsValue) -> bool
    pub fn len(&self) -> usize
    pub fn prune(&mut self)
    pub fn get_contact_infos(&self) -> Vec<(Pubkey, SocketAddr)>
    pub fn all_contact_infos(&self) -> Vec<(Pubkey, ContactInfo)>
    pub fn get_highest_slot(&self) -> Option<Slot>
    pub fn drain_events(&mut self) -> Vec<GossipEvent>
}
```

---

## Wire types — byte layout

### Protocol enum tag

`Protocol` has 7 variants (tag 0–6, bincode u32 LE):

| Tag | Variant |
|-----|---------|
| 0   | PullRequest(CrdsFilter, CrdsValue) |
| 1   | PullResponse(Pubkey, Vec\<CrdsValue\>) |
| 2   | PushMessage(Pubkey, Vec\<CrdsValue\>) |
| 3   | PruneMessage(Pubkey, PruneData) |
| 4   | PingMessage(Ping) |
| 5   | PongMessage(Pong) |
| 6   | Unknown |

### ContactInfo (72 bytes with 1 addr + 1 socket)

```
pubkey        [u8; 32]         32 B
wallclock     serde_varint      3–9 B
outset        u64 LE            8 B
shred_version u16 LE            2 B
version       Version          12 B  (see below)
addrs         short_vec[IpAddr] 1 + N*8 B  (ShortU16 len + IpAddr elements)
sockets       short_vec[...]    1 + N*4 B
extensions    short_vec[Ext]    1 B (empty)
cache         #[serde(skip)]    0 B
```

### Version (12 bytes)

```
major         LEB128 varint    1 B
minor         LEB128 varint    1 B
patch         LEB128 varint    1 B
commit        u32 LE            4 B
feature_set   u32 LE            4 B
client        LEB128 varint    1 B
```

### Ping (132 bytes) / Pong (100 bytes)

```
Ping:  pubkey[32] + token[32] + signature[64] = 128 B + 4 B tag = 132 B
Pong:  pubkey[32] + hash[32]  + signature[64] = 128 B + 4 B tag = 132 B
```

### CrdsData enum tag

`CrdsData` has 14 variants (bincode u32 LE). The only one we
produce/consume:

| Tag | Variant |
|-----|---------|
| 11  | ContactInfo(ContactInfo) |

---

## Key files — what each one does

| File | Responsibility |
|---|---|
| `contact_info.rs` | `ContactInfo` struct + `SocketEntry`, field-level serde annotations |
| `crds_data.rs` | `CrdsData` enum (14 variants), `CrdsValue`, signing/verification |
| `crds_filter.rs` | `CrdsFilter` with bloom + mask, mask_bits calculation |
| `crds.rs` | `CrdsTable` — indexed CRDS state, merge by wallclock, prune stale |
| `pull_request.rs` | Builds a full `Protocol::PullRequest` fitting in 1232 B |
| `ping_pong.rs` | `Ping<N>`, `Pong`, sign and verify |
| `handler.rs` | Matches incoming `Protocol` variants, dispatches to CRDS / reply |
| `protocol.rs` | `Protocol` enum definition + `encode_to()`/`decode_from()` |
| `transport.rs` | Async UDP wrapper (tokio) |
| `keypair.rs` | Key generation |

---

## What dc-gossip does NOT do

- Does not vote
- Does not produce or validate blocks
- Does not push its own state to peers (pull-only at the moment)
- Does not handle shreds (that is dc-tvu's job)
- Does not implement the full CRDS shard scan for PullResponses
  (relies on the entrypoint to send us everything)

These are intentional limits. dc-gossip's job is to discover peers,
learn their addresses, and track what slot they are on.

---

## Debugging notes

**Why did PullRequests silently fail before the fix?**
1. Wrong `shred_version` (11016 → 7016) — Agave's
   `check_pull_request_shred_version()` rejects the PullRequest before
   any response is generated. Ping/Pong bypass this check, so the
   handshake worked but data exchange did not.

**How to verify wire compatibility**
1. Run `cargo run -- --self-test` — validates round-trip serialization
   and signature verification.
2. Check `mask_bits >= MIN_PULL_REQUEST_MASK_BITS` (6 in Agave v4.x
   release builds). Our code produces `mask_bits = 6` (correct).
3. Check bloom `contains()` returns `false` for an empty filter
   (otherwise every value is filtered out).
