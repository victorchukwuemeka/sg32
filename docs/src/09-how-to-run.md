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
2026-05-11T22:23:32.479Z  INFO dc_gossip: CRDS: 117 entries, 56 gossip peers
2026-05-11T22:23:32.479Z  INFO dc_gossip:   IP Address          | Age(ms) | Node identifier                              | Version | Gossip | TPUvote | TPU  | TPUfwd | TVU  | TVU Q | ServeR | ShredVer
2026-05-11T22:23:32.479Z  INFO dc_gossip:   ----------------------------------------------------------------------------------------------------------------------------------------
2026-05-11T22:23:32.479Z  INFO dc_gossip:   208.91.110.147      | -       | ES5M2g5Lu4CewkkTLn56wekb1wfN4AvMNtWAK9tTS14U | 4.16384.0 | 8001 |    8005 |    1 |     1 | 8002 |  none |  8010 |   11016
2026-05-11T22:23:32.479Z  INFO dc_gossip:   185.189.44.238      | -       | 3ne7n82Kqf1zPo4obYTnQA8tJBDuSEYyvKYS89Mbky4q | 4.32768.7 | 8000 |    8004 |    1 |     1 | 8001 |  none |  8009 |   11016
2026-05-11T22:23:32.479Z  INFO dc_gossip:   151.202.34.247      | -       | iniPkjWxbT88TUAUGiUVB3WoCeq2kUAQAs3dN4pjnEv | 3.1.8   | 8001 |    8005 | 8003 |  8004 | 8000 |  8002 |  8012 |   11016
2026-05-11T22:23:32.479Z  INFO dc_gossip:   Nodes: 56
2026-05-11T22:23:33.484Z  INFO dc_gossip: sending PullRequest (1232 bytes) to 63 peers
```

### Debug levels:
- `RUST_LOG=info` — Normal operation: handshake, PullRequest, decoded entries, CRDS stats
- `RUST_LOG=debug` — Detailed: every Pong reply, every Prune message, decode errors with actual error messages
- `RUST_LOG=warn` — Only warnings and errors
