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
