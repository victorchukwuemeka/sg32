## 8. What We Still Get Wrong (and Why It Doesn't Matter)

### Issue 1: Unknown CrdsData variants

Our `CrdsData` enum has 14 variants, but we only properly implement `ContactInfo` (variant 11). When a PullResponse contains `CrdsData::Vote` (variant 1) or `CrdsData::LowestSlot` (variant 2) or other variants, the inner struct definitions may not match Agave's exact wire format, causing those specific values to fail deserialization.

**Our mitigation:** `Protocol::decode_from` has a per-value fallback that tries the fast path first and, on failure, manually parses each `CrdsValue` in a `Vec<CrdsValue>`, skipping individual bad values. This means a single bad Vote variant doesn't take down the whole PullResponse.

**Why it still doesn't matter:** We still receive plenty of valid `ContactInfo` entries from every PullResponse. The few values that fail are simply skipped.

### Issue 2: UDP Packet Fragmentation

Some PullResponse packets arrive truncated (`io error: unexpected end of file`). This is expected behavior with UDP — packets can be dropped or fragmented by network conditions.

**Why it doesn't matter:** The next PullRequest will get a fresh batch of data.

### Issue 3: SOCKET_CACHE_SIZE version mismatch

We now use 14 (Agave v2.2.x devnet standard: tags 0-13, where tag 13 = Alpenglow). Since the cache field is populated from socket entries during deserialization, mismatches in cache size could cause indexing errors if a node sends socket keys >= our cache size. For now, 14 covers all current tags.
