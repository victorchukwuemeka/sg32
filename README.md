# solana-protocol-gym

> A full-stack Solana protocol implementation in Rust — built for engineers who need trustless access to onchain data.

This is not a dApp framework. It is not a wallet SDK. It is not another JSON-RPC wrapper.

**solana-protocol-gym** is a ground-up implementation of the Solana validator stack — gossip, TVU, TPU, PoH, Tower BFT, Sealevel, and RPC — written in Rust, modular by design, and built to be read, hacked, and learned from.

**What makes this different:** We generate **Merkle inclusion proofs** and **ZK proofs** over the transaction set — trustless verification that a transaction was included in a specific slot, without running a full node. Solana's native RPC gives you none of this.

---

## Why this exists

Most Solana developers live at the smart contract layer. They use `@solana/web3.js`, call JSON-RPC endpoints, and never think about what happens underneath.

That's fine — until you want to work on the protocol itself.

The gap between "I can write Anchor programs" and "I understand how the validator processes a block" is enormous. Very few resources bridge it. Reading the Agave source cold is brutal. Running a full validator just to observe behavior requires datacenter hardware.

**solana-protocol-gym fills that gap.**

Every module implements a real piece of the Solana stack — same wire protocols, same data structures, same behaviors as Agave — but written to be understood, not just to run in production.

---

## What's inside

| Module | Status | What it implements |
|---|---|---|---|
| `dc-gossip` | ✅ **Working** | CRDS table, peer discovery, cluster info table — connects to devnet, discovers 50+ peers, shows versions and ports |
| `dc-tvu` | 🔜 Building | Shred receiver, erasure reconstruction, block assembly |
| `dc-prover` | 🔜 Building | **Merkle inclusion proofs** + **ZK proofs** over transaction sets — verify tx inclusion without trust |
| `dc-tpu` | ⏳ Planned | Transaction forwarding to leader via QUIC |
| `dc-ledger` | ⏳ Planned | Block storage, account state, ledger parsing |
| `dc-poh` | ⏳ Planned | Proof of History hash chain verifier |
| `dc-consensus` | ⏳ Planned | Tower BFT simulator — votes, forks, lockouts |
| `dc-runtime` | ⏳ Planned | Sealevel-lite parallel transaction execution |
| `dc-rpc` | ⏳ Planned | RPC server serving proofs + raw data for bots and onchain products |
| `dc-cli` | ⏳ Planned | CLI tools for every module |

---

## dc-gossip — the working module

`dc-gossip` speaks the **real Solana gossip wire protocol** — the same UDP-based CRDS protocol that every validator uses to discover peers and exchange cluster state.

### What it does

1. **Ping/Pong handshake** with any gossip entrypoint
2. **PullRequest/PullResponse** — asks "who's on the network?" and receives CRDS entries
3. **ContactInfo decoding** — parses validator identity, version, shred version, and all socket addresses (gossip, TPU, TVU, TPUvote, ServeR, etc.)
4. **CRDS table** — in-memory store of discovered validators, pruned after 15 minutes
5. **Cluster info table** — prints all discovered peers with ports every 30 seconds
6. **Automatic peer discovery** — starts with 1 entrypoint, grows to 100+ peers within seconds
7. **Per-value error recovery** — skips individual bad CRDS entries instead of dropping entire messages

### What it looks like

```
2026-05-11T22:23:32.479Z  INFO dc_gossip: CRDS: 117 entries, 56 gossip peers
2026-05-11T22:23:32.479Z  INFO dc_gossip:
    IP Address       | Age(ms) | Node ID                                      | Version    | Gossip | TPUvote | TPU | TPUfwd | TVU | ServeR | ShredVer
  ----------------------------------------------------------------------------------------------------------------------------------
  208.91.110.147     | -       | ES5M2g5Lu4CewkkTLn56wekb1wfN4AvMNtWAK9tTS14U | 4.16384.0  | 8001   | 8005    | 1   | 1      | 8002| 8010   | 11016
  185.189.44.238     | -       | 3ne7n82Kqf1zPo4obYTnQA8tJBDuSEYyvKYS89Mbky4q | 4.32768.7  | 8000   | 8004    | 1   | 1      | 8001| 8009   | 11016
  151.202.34.247     | -       | iniPkjWxbT88TUAUGiUVB3WoCeq2kUAQAs3dN4pjnEv | 3.1.8      | 8001   | 8005    | 8003| 8004   | 8000| 8012   | 11016
  Nodes: 56
```

### Debugging journey documented

The complete byte-level debugging story — from "entrypoint ignores us" to "56 peers discovered" — is documented in [`crates/dc-gossip/GOSSIP_DETAILS.md`](crates/dc-gossip/GOSSIP_DETAILS.md).

It covers:
- Why `#[serde(with = "serde_varint")]` on `wallclock` was the #1 bug (7-byte cascading field shift)
- How `SocketEntry.offset` is a cumulative port offset, not an absolute port
- Why the `cache` field needed a custom `Deserialize` via `ContactInfoLite`
- How `CrdsValue.hash` was adding 32 poison bytes to every message
- Byte-level wire format of every struct before and after fixes
- Complete 5-message devnet conversation traced byte-by-byte

---

## Why Proofs Matter

Solana's native RPC is **trusted** — when you ask "was tx X included in slot Y?" the RPC node just says "yes" and you have to trust it. There is no cryptographic proof.

**Solana doesn't have native Merkle inclusion proofs for transactions.** Ethereum has Patricia Merkle tries — you can prove inclusion with a 1KB proof. Solana has nothing comparable at the RPC layer.

We fix that:

```
Normal RPC flow:
  You ──► "was tx X in slot Y?" ──► RPC Node ──► "yes" (trust me bro)

Our flow:
  You ──► "was tx X in slot Y?" ──► Our Node
                                      │
                                      ▼
                               Builds Merkle proof over
                               the slot's transaction set
                                      │
                                      ▼
                               "yes + here's the Merkle proof"
                               You verify it yourself → no trust required

  For ZK: same data, wrapped in a ZK proof → constant-size, private, verifiable anywhere
```

This enables:
- **Trustless bridges** — prove Solana tx inclusion to Ethereum without running a Solana node
- **Light clients** — verify a handful of transactions with a small proof instead of downloading the whole block
- **Bots/trading** — verify their own tx submissions cryptographically
- **Coprocessors** — ZK provers that consume verified Solana state

---

## Architecture

```
solana-protocol-gym/
├── crates/
│   ├── dc-gossip/        # ✅ CRDS, peer discovery, UDP sockets
│   │   ├── src/
│   │   │   ├── main.rs           # gossip loop, cluster info table
│   │   │   ├── contact_info.rs   # ContactInfo, Version, SocketEntry
│   │   │   ├── crds.rs           # CRDS table with merge/prune
│   │   │   ├── crds_data.rs      # CrdsData enum, CrdsValue
│   │   │   ├── protocol.rs       # Protocol enum, encode/decode
│   │   │   ├── handler.rs        # message handler
│   │   │   ├── ping_pong.rs      # Ping/Pong structs
│   │   │   ├── pull_request.rs   # PullRequest builder
│   │   │   └── transport.rs      # UDP socket wrapper
│   │   └── GOSSIP_DETAILS.md     # complete debugging write-up
│   ├── dc-tvu/           # 🔜 Shred receiver, block reassembly
│   ├── dc-prover/        # 🔜 Merkle + ZK proof generation
│   ├── dc-tpu/           # ⏳ placeholder
│   ├── dc-ledger/        # ⏳ placeholder
│   ├── dc-poh/           # ⏳ placeholder
│   ├── dc-consensus/     # ⏳ placeholder
│   ├── dc-runtime/       # ⏳ placeholder
│   ├── dc-rpc/           # ⏳ placeholder
│   └── dc-cli/           # ⏳ placeholder
├── docs/
└── examples/
```

---

## Getting started

### Prerequisites
- Rust (latest stable)
- Cargo
- Git

### Clone
```bash
git clone https://github.com/victorchukwuemeka/solana-protocol-gym.git
cd solana-protocol-gym
```

### Build
```bash
cargo build
```

### Run gossip listener
```bash
RUST_LOG=info cargo run --bin dc-gossip
```

You should see validators discovered from Solana devnet within seconds, with a full cluster info table every 30 seconds.

---

## Roadmap

### ✅ Phase 1 — Gossip (done)
- ✅ Ping/Pong handshake with any gossip entrypoint
- ✅ PullRequest/PullResponse — discover 50+ peers
- ✅ ContactInfo decoding with all socket addresses and versions
- ✅ CRDS table with merge, prune, and dedup
- ✅ Per-value error recovery in PullResponse parsing
- ✅ Cluster info table display
- ✅ Full debugging write-up in `GOSSIP_DETAILS.md`

### 🔜 Phase 2 — TVU / Turbine (block receiver)
- Bind to TVU port and receive raw shreds from the network
- Parse data shreds and coding shreds
- Implement Reed-Solomon erasure reconstruction
- Reassemble shreds into full blocks
- Extract the native Merkle proofs embedded in every shred chain

### 🔜 Phase 3 — Merkle Inclusion Proofs
- Build a Merkle tree over every transaction in a slot
- Generate **inclusion proofs**: "tx X is in slot Y at position Z"
- Serve proofs via RPC and a queryable API
- Enable trustless tx verification without running a full node

### 🔜 Phase 4 — ZK Proofs
- Build ZK circuits for transaction set verification
- Generate succinct proofs that a set of transactions was included in a slot
- Enable light clients, bridges, and coprocessors to verify Solana data with constant-size proofs
- Integrate with existing ZK ecosystems (RISC Zero, SP1, Circom)

### 🔜 Phase 5 — Proof of History verifier
- Implement PoH hash chain replay in Rust
- Verify time-ordering of transactions cryptographically

### 🔜 Phase 6 — TPU / QUIC (transaction sender)
- Build a QUIC client that connects directly to validator TPU ports
- Send raw transaction bytes — no RPC, no middleware

### 🔜 Phase 7+ — Consensus, Runtime, RPC
- Tower BFT simulator with vote fork visualization
- Sealevel-lite parallel transaction execution
- High-performance RPC server serving proofs + raw block data for bots

---

## Who this is for

- **Bots and trading firms** — need RPC access with verified transaction inclusion proofs
- **Bridge builders** — trustless verification of Solana transactions on other chains
- **ZK researchers** — building light clients, coprocessors, or cross-chain protocols that need verified onchain data
- **Solana core contributors** — want to understand Agave/Firedancer from the wire up
- **Protocol engineers** — building the next generation of Solana infrastructure

---

## Relation to Agave

solana-protocol-gym is an independent implementation. It is not a fork of Agave.

Where Agave optimises for production performance, this project optimises for clarity. Where Agave abstracts away wire-level details, this project exposes them. The goal is not to replace Agave — it is to make Agave understandable.

Key source references used during gossip implementation:
- `solana/gossip/src/contact_info.rs` — ContactInfo struct, SocketEntry, cumulative port offsets, custom Deserialize via ContactInfoLite
- `solana/gossip/src/crds_value.rs` — CrdsValue with hash skip, manual Deserialize
- `solana/gossip/src/protocol.rs` — Protocol enum discriminants
- `solana/gossip/src/cluster_info.rs` — PullRequest handler, shred version check
- `solana/version/src/v3.rs` — Version struct with varint annotations

---

## Contributing

Contributions welcome — especially from developers working through the protocol stack for the first time. If something is unclear, that is a bug worth fixing.

See [CONTRIBUTING.md](CONTRIBUTING.md).

---

## License

MIT — build whatever you want with it.

---

*Built by a contributor to solana-program/token, solana-program/token-2022, Pinocchio, and the Agave validator client.*
