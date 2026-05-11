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
