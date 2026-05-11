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

**What changed:** 10 specific edits across two rounds.

#### Round 1 (Initial wire-format fixes)

**Edit 1 — Imports:**
```rust
// Added:
use solana_serde_varint as serde_varint;
use solana_short_vec as short_vec;
```

**Edit 2 — SOCKET_CACHE_SIZE:**
```rust
// Initially (wrong):
const SOCKET_CACHE_SIZE: usize = 13;

// After Phase 5 fix:
const SOCKET_CACHE_SIZE: usize = 14;
```
Why 14? Agave v2.2.x has socket tags 0-13 = 14 tags (`SOCKET_TAG_ALPENGLOW = 13`, so `SOCKET_CACHE_SIZE = 13 + 1 = 14`). The cache field is `skip_serializing` so this only affects in-memory layout, not wire format — but deserialization must match.

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

**Edit 8 — ContactInfo.addrs, .sockets, .extensions:**
```rust
// Before:
pub addrs: Vec<IpAddr>,
pub sockets: Vec<SocketEntry>,
pub extensions: Vec<Extension>,

// After:
#[serde(with = "short_vec")]
pub addrs: Vec<IpAddr>,
#[serde(with = "short_vec")]
pub sockets: Vec<SocketEntry>,
#[serde(with = "short_vec")]
pub extensions: Vec<Extension>,
```

#### Round 2 (Phase 5 bugs — more subtle)

**Edit 9 — socket_by_key: cumulative port offsets**

The old code treated `SocketEntry.offset` as an absolute port. In Agave's wire protocol, offsets are cumulative — you accumulate them while iterating.

```rust
// Before (WRONG):
pub fn socket_by_key(&self, key: u8) -> Option<SocketAddr> {
    self.sockets.iter().find(|s| s.key == key).and_then(|entry| {
        let ip = self.addrs.get(entry.index as usize)?;
        Some(SocketAddr::new(*ip, entry.offset))
    })
}

// After (CORRECT — matches Agave's get_socket):
pub fn socket_by_key(&self, key: u8) -> Option<SocketAddr> {
    let mut port = 0u16;
    for entry in &self.sockets {
        port = port.checked_add(entry.offset)?;
        if entry.key == key {
            let ip = self.addrs.get(entry.index as usize)?;
            return Some(SocketAddr::new(*ip, port));
        }
    }
    None
}
```

**Edit 10 — Custom Deserialize via ContactInfoLite**

The derived `#[derive(Deserialize)]` tried to read `cache: [SocketAddr; SOCKET_CACHE_SIZE]` from the byte stream even though it was `skip_serializing` — there were no bytes for it, so bincode consumed the next CrdsValue's buffer.

Replaced with Agave's two-struct pattern:

```rust
// Struct for deserialization only — no cache field
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

// ContactInfo only derives Serialize
#[derive(Debug, Clone, Serialize)]
pub struct ContactInfo {
    pub pubkey: Pubkey,
    // ... (same fields, no Deserialize derive)
    #[serde(skip_serializing)]
    pub cache: [SocketAddr; SOCKET_CACHE_SIZE],
}

// Custom Deserialize: deserialize into ContactInfoLite,
// then populate cache from socket entries (cumulative port offsets)
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
        Ok(Self { pubkey: lite.pubkey, wallclock: lite.wallclock,
            outset: lite.outset, shred_version: lite.shred_version,
            version: lite.version, addrs: lite.addrs,
            sockets: lite.sockets, extensions: lite.extensions, cache })
    }
}
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

### File: `src/protocol.rs`

**What changed:** Added per-value CrdsValue error handling fallback in `decode_from`.

When a PullResponse or PushMessage contains a mix of CrdsData variants and one variant fails to deserialize, the entire message previously failed. Now `decode_from` tries a fast path first (full `bincode::deserialize`), and if that fails, manually parses each CrdsValue individually, skipping bad ones:

```rust
pub fn decode_from(bytes: &[u8]) -> Result<Self> {
    // Fast path — bincode handles the whole thing
    if let Ok(msg) = bincode::deserialize(bytes) {
        return Ok(msg);
    }
    // Slow path — manual parse, skip bad CrdsValues
    let tag: u32 = bincode::deserialize_from(&mut cursor)?;
    match tag {
        1 | 2 => {  // PullResponse or PushMessage
            let from: Pubkey = bincode::deserialize_from(&mut cursor)?;
            let count: u64 = bincode::deserialize_from(&mut cursor)?;
            for _ in 0..count {
                match bincode::deserialize::<CrdsValue>(remaining) {
                    Ok(val) => values.push(val),
                    Err(_) => { /* skip bad value */ }
                }
            }
        }
        _ => bincode::deserialize(bytes)  // other variants: no fallback
    }
}
```

---

### File: `src/crds.rs`

**What changed:** Added `all_contact_infos()` method for the cluster info table display.

Previously only `get_contact_infos()` existed, returning `Vec<(Pubkey, SocketAddr)>` — enough for peer discovery but insufficient for the port table display which needs the full `ContactInfo` struct.

```rust
pub fn all_contact_infos(&self) -> Vec<(Pubkey, &ContactInfo)> {
    self.entries
        .iter()
        .filter_map(|(pk, value)| match &value.data {
            CrdsData::ContactInfo(ci) => Some((*pk, ci)),
            _ => None,
        })
        .collect()
}
```

The table row rendering in `main.rs` uses this every 30 seconds to print the cluster info table.
