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
| `gossip/src/contact_info.rs:530-583` | Custom Deserialize for ContactInfo via ContactInfoLite (pattern we replicated) |
| `gossip/src/contact_info.rs:312-330` | get_socket() with cumulative port offsets (critical for socket_by_key fix) |
| `gossip/src/contact_info.rs:344-377` | set_socket() showing how port offsets are computed |
| `gossip/src/contact_info.rs:46` | SOCKET_CACHE_SIZE = SOCKET_TAG_ALPENGLOW + 1 = 14 |

### v2.2.0 (what devnet runs)

| Command | What We Learned |
|---------|-----------------|
| `git show v2.2.0:gossip/src/contact_info.rs` | Same serde annotations as v3, SOCKET_CACHE_SIZE = 13 |
| `git show v2.2.0:gossip/src/protocol.rs` | Same discriminant ordering as v3 |

### Additional Files We Created

| File | What It Does |
|------|-------------|
| `crates/dc-gossip/src/contact_info.rs` | ContactInfo with custom Deserialize, cumulative port offsets in socket_by_key |
| `crates/dc-gossip/src/crds.rs` | CrdsTable with all_contact_infos() for table display |
| `crates/dc-gossip/src/protocol.rs` | Protocol::decode_from with per-value CrdsValue fallback |
| `crates/dc-gossip/GOSSIP_DETAILS.md` | This document |

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
