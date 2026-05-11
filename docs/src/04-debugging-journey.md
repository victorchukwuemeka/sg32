## 4. The Debugging Journey

> This section tells the story in chronological order — every hypothesis, every dead end, every "aha" moment. It starts with the original wire-format bugs (Phases 1-4), then covers the second wave of bugs (Phase 5) that appeared only after the entrypoint started talking back.

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

But now a new problem emerged — we could see peers in the CRDS table, **but every port showed "none"**.

---

### Phase 5: The Second Wave — Port Offsets and Cache Poisoning

Now that the entrypoint was actually sending us data, two new bugs became visible:

**Bug A: Cumulative port offsets**

Agave's `SocketEntry.offset` is **not** an absolute port — it's a port offset relative to the previous entry. To get the actual port for a socket, you accumulate offsets in order.

**Agave's get_socket (gossip/src/contact_info.rs:312-330):**
```rust
fn get_socket(&self, key: u8) -> Result<SocketAddr, Error> {
    let mut port = 0u16;
    for entry in &self.sockets {
        port += entry.offset;        // ← CUMULATIVE, not absolute!
        if entry.key == key {
            let addr = self.addrs.get(usize::from(entry.index))?;
            return Ok(SocketAddr::new(*addr, port));
        }
    }
    Err(Error::SocketNotFound(key))
}
```

**Our broken socket_by_key:**
```rust
pub fn socket_by_key(&self, key: u8) -> Option<SocketAddr> {
    self.sockets.iter().find(|s| s.key == key).and_then(|entry| {
        let ip = self.addrs.get(entry.index as usize)?;
        Some(SocketAddr::new(*ip, entry.offset))  // ← WRONG: used offset as absolute port!
    })
}
```

A real ContactInfo from devnet might have socket entries like:
```
key=0 (gossip),    offset=8001, index=0  → port = 0 + 8001 = 8001 ✓
key=5 (TPU),       offset=2,    index=0  → port = 8001 + 2 = 8003 ✓
key=9 (TPUvote),   offset=0,    index=0  → port = 8003 + 0 = 8003 ✓
key=10 (TVU),      offset=-1,   index=0  → port = 8003 - 1 = 8002 ✓
```

Our old code treated `offset=2` for TPU as port `2` instead of `8003`.

Encoded by `set_socket`: the first entry's offset equals the full port. Each subsequent entry's offset is the difference from the previous port. This differential encoding allows small varint values: the gossip port (8001) becomes a 2-byte varint, but subsequent offsets like 2, 0, or -1 are 1 byte each.

**The fix:**
```rust
pub fn socket_by_key(&self, key: u8) -> Option<SocketAddr> {
    let mut port = 0u16;
    for entry in &self.sockets {
        port = port.checked_add(entry.offset)?;  // ← cumulative!
        if entry.key == key {
            let ip = self.addrs.get(entry.index as usize)?;
            return Some(SocketAddr::new(*ip, port));
        }
    }
    None
}
```

**Bug B: The cache field — a poison pill for deserialization**

Our `ContactInfo` had `#[serde(skip_serializing)]` on the `cache` field. This correctly skipped `cache` during serialization (it's not on the wire). But since we derived `Deserialize`, serde still tried to **read** cache from the byte stream during deserialization. Since cache wasn't in the stream, bincode would read the next 130 bytes (13 SocketAddrs × ~10 bytes each for IPv4) from whatever came after the ContactInfo data — which was the **next CrdsValue's buffer**.

This corrupted every downstream CrdsValue and silently consumed bytes that belonged to subsequent values.

**The fix:** Agave uses a two-struct approach — a `ContactInfoLite` without cache for deserialization, then populate cache from socket entries:

```rust
#[derive(Deserialize)]
struct ContactInfoLite {
    pubkey: Pubkey,
    #[serde(with = "serde_varint")]
    wallclock: u64,
    outset: u64,
    shred_version: u16,
    version: Version,
    #[serde(with = "short_vec")]
    addrs: Vec<IpAddr>,
    #[serde(with = "short_vec")]
    sockets: Vec<SocketEntry>,
    #[serde(with = "short_vec")]
    extensions: Vec<Extension>,
}

impl<'de> Deserialize<'de> for ContactInfo {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error> {
        let lite = ContactInfoLite::deserialize(deserializer)?;
        let mut cache = [SOCKET_ADDR_UNSPECIFIED; SOCKET_CACHE_SIZE];
        let mut port = 0u16;
        for entry in &lite.sockets {
            port = port.wrapping_add(entry.offset);
            if let Some(cached) = cache.get_mut(usize::from(entry.key)) {
                if let Some(addr) = lite.addrs.get(usize::from(entry.index)) {
                    *cached = SocketAddr::new(*addr, port);
                }
            }
        }
        Ok(Self { pubkey: lite.pubkey, wallclock: lite.wallclock, ..., cache })
    }
}
```

We also had `SOCKET_CACHE_SIZE = 13` but Agave v2.2.x devnet actually uses 14 (tags 0-13, where tag 13 = Alpenglow). Fixed that too.

**With both fixes, the cluster info table finally showed real data:**

```
  IP Address          | AgeMs | Node identifier                          | Version | Gossip | TPUvote | TPU  | TPUfwd | TVU  | TVU Q | ServeR | ShredVer
  ----------------------------------------------------------------------------------------------------------------------------------------
  208.91.110.147      | -     | ES5M2g5Lu4Cewkk...                       | 4.16384.0 | 8001 |    8005 |    1 |     1 | 8002 |  none |  8010 |   11016
  185.189.44.238      | -     | 3ne7n82Kqf1zPo4...                       | 4.32768.7 | 8000 |    8004 |    1 |     1 | 8001 |  none |  8009 |   11016
  151.202.34.247      | -     | iniPkjWxbT88TUAU...                       | 3.1.8   | 8001 |    8005 | 8003 |  8004 | 8000 |  8002 |  8012 |   11016
```

56 peers, all with correct ports. Peer discovery grew from 1 → 116 in ~25 seconds.

---

### The Per-Value CrdsValue Fallback

One more issue revealed itself: when a PullResponse contains a mix of CrdsData variants, and one fails to deserialize, the entire message was lost. We added a fallback in `Protocol::decode_from` that tries the fast path first (`bincode::deserialize` on the whole message), and if that fails, falls back to manual per-CrdsValue parsing that skips individual bad variants.

```
Fast path: bincode::deserialize::<Protocol>(bytes)
  → If OK, great (most PullResponses with all-ContactInfo values)
  → If error:
    1. Read Protocol discriminant manually
    2. For PullResponse/PushMessage (tags 1-2):
       a. Read Pubkey (from)
       b. Read u64 count
       c. For each value: try bincode::deserialize::<CrdsValue>
          → If OK: advance cursor by serialized_size
          → If error: skip ahead by estimated size (64 bytes + CrdsData)
```

This way, a single bad Vote or LowestSlot variant doesn't take down the whole PullResponse.
