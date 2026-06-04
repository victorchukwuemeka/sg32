# sg32 — future directions (internal, not public)
#
# We started with shred parsing and Merkle proofs.
# The goal: deepest Solana infra that exists.
# Everything below is buildable on top of what we already have.

## 0. Leader Signature Verification (Trustless Proofs)

### The Problem
Our Merkle proofs prove tx-inclusion within a downloaded set, but not that
the set is authentic. We trust whatever validators send us via repair protocol.

### How Leader Selection Works
```
At epoch boundary:
  1. Collect all validators with stake from the bank
  2. Sort by (stake, pubkey) descending
  3. Deterministic PRNG (ChaCha8) seeded with bank_hash
  4. For each slot in the epoch, pick validator weighted by stake
     Higher stake = more slots leader
```

Deterministic — same inputs → same result. But inputs require bank state.

### Why This Is Hard Without a Bank
Need `leader_schedule_seed` (from bank hash at epoch boundary) + stake
distribution. Both require replaying transactions. We don't replay.

### Options

| Option | How | Complexity | Trustless? |
|--------|-----|-----------|-----------|
| Trusted RPC query | Fetch `getLeaderSchedule` from public RPC on startup | Easy (1 curl) | ❌ (trusts RPC) |
| Gossip-assisted | Cross-reference shred senders with gossip peer data | Medium | Partial |
| Full replay | Implement Sealevel to replay txs up to epoch boundary | Very hard | ✅ |

### Implementation Sketch (when ready)
1. On startup, fetch leader schedule from public RPC (or compute if we have bank)
2. For each completed FEC batch:
   a. Build Merkle tree over all 64 shreds
   b. Compute batch Merkle root
   c. Look up leader pubkey for this slot from schedule
   d. Verify ed25519 signature on the shred against the batch root
   e. Verify chained Merkle root matches previous batch's root
3. Tag SlotData as "verified" before serving proofs

Cost: ~75μs per FEC batch for sig verification. Negligible.

---

## 1. ZK Coprocessor (Solana → Ethereum)
Wrap our Merkle proofs in a ZK circuit (RISC Zero / SP1).
Prove "tx X happened in Solana slot Y" on Ethereum.
No Solana node needed on the destination chain.
First end-to-end Solana ZK coprocessor.

## 2. Firedancer-scale RS decoder
Our decoder processes one byte at a time (scalar).
Rewrite using AVX2/AVX-512 — process 32 columns in parallel.
Target: line-rate decode at 100Gbps.
Match Jump's Firedancer performance from a cold start.

## 3. Real Turbine retransmission node
Not a simulator. A real node that:
- Receives shreds on the TVU port
- Computes its position in the weighted shuffle (stake-weighted Fisher-Yates)
- Forwards shreds to its designated children
- Signs the last FEC batch with our key before forwarding (resigning)
- Requests missing shreds from peers via repair protocol
- Participates as a real peer in the cluster

We have 90% of the pieces. Only missing: cluster topology (weighted shuffle),
a second UDP socket for retransmission, and the repair request protocol.
Building this means our node is a genuine validator on the network — just
without voting.

## 4. Light client protocol
Sub-100KB state proofs for Solana.
Download one slot's Merkle root + proof, verify.
No full node needed. Works on a phone.
Solana doesn't have this. We build it.

## 5. Custom gossip overlay
Replace Agave's CRDS with something faster.
Kademlia-inspired. Faster peer discovery, tighter convergence.
Publish a paper comparing against Agave's gossip.

## 6. The endgame
All of the above → sg32 is the reference implementation
for trustless Solana data. Bridges use us. Bots use us.
Light clients use us. ZK provers use us.

We're building the Nvidia of Solana infra.
Not the biggest team. The deepest.
