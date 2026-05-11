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
