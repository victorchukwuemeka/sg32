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
│  4. decode_from() tries fast path first:                            │
│     - bincode::deserialize::<Protocol>(bytes) → if OK, use it       │
│     - If fails: manual per-CrdsValue parse, skip bad variants       │
│  5. For each CrdsValue in PullResponse:                             │
│     - CrdsData::ContactInfo(node) → merge into CRDS table ✓         │
│     - Other variants → per-value fallback skips failures ✓          │
│  6. Every 5 seconds: send PullRequest to all known peers            │
│  7. Every 30 seconds:                                              │
│     a. Prune old CRDS entries (15 min TTL)                          │
│     b. Print cluster info table with all ports                      │
│     c. Extend known_peers with new gossip addresses                 │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

---

*This document was created from the complete debugging history of the dc-gossip crate, covering every command run, every file read, and every byte analyzed during the journey from "entrypoint ignores us" to "we're part of the devnet gossip network".*
