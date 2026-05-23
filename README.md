# solana-protocol-gym

> A full-stack Solana protocol implementation in Rust — learn how the validator works by building one, then ship an RPC that serves trustless proofs.

This is not a dApp framework. It is not a wallet SDK. It is not another JSON-RPC wrapper.

**solana-protocol-gym** is a ground-up implementation of the Solana validator stack — gossip, TVU, TPU, PoH, Tower BFT, Sealevel, RPC, and a **proof generation layer** (Merkle inclusion + ZK) — written in Rust, modular by design.

Every protocol module is real — same wire formats, same data structures as Agave — but written to be understood, not just to run in production. The endgame is a high-performance RPC node that serves block data with **cryptographic proofs of inclusion**, something Solana's native RPC doesn't provide.

---

## What's inside

| Module | Status | What it implements |
|---|---|---|---|
| `dc-gossip` | Working | CRDS table, peer discovery, cluster info table — connects to devnet, discovers 50+ peers, shows versions and ports |
| `dc-tvu` | Building | Full pipeline: shred receiver → RS recovery → deshredder → ring buffer → flat file store → Merkle prover. Everything below RPC lives here; will split into separate crates when stable |

Build order: you can't serve proofs (RPC) without building Merkle trees. You can't build trees without storing entries (ledger). You can't store them without receiving and recovering shreds (TVU). You can't receive them without knowing who to listen to (gossip).

---

## Why Proofs Matter

Solana's native RPC is **trusted** — when you ask "was tx X included in slot Y?" the RPC node just says "yes" and you have to trust it. There is no cryptographic proof.

**Solana doesn't have native Merkle inclusion proofs for transactions.** Ethereum has Patricia Merkle tries — you can prove inclusion with a ~1KB proof. Solana has nothing comparable at the RPC layer.

We fix that:

```
Normal RPC flow:
  You --> "was tx X in slot Y?" --> RPC Node --> "yes" (trust me bro)

Our flow:
  You --> "was tx X in slot Y?" --> Our Node
                                      |
                                      v
                               Builds Merkle proof over
                               the slot's transaction set
                                      |
                                      v
                               "yes + here's the Merkle proof"
                               You verify it yourself -- no trust required

  For ZK: same data, wrapped in a ZK proof -> constant-size, verifiable anywhere
```

This enables:
- **Trustless bridges** — prove Solana tx inclusion to Ethereum without running a Solana node
- **Light clients** — verify a handful of transactions with a small proof instead of downloading the whole block
- **Bots/trading firms** — verify their own tx submissions cryptographically
- **Coprocessors** — ZK provers that consume verified Solana state

---

## Data Flow

```
Gossip ──► TVU ────────────────► RPC ──► Bots / Bridges / Light Clients
  │          │                       │
  │    knows who          serves blocks
  │    to listen          + proofs
  │    to from                 
  ▼    gossip                  
peer list

TVU (internal pipeline):
  ShredFetch → RS Recover → Deshredder → Ring Buffer → Flat File Store → Merkle Prover                                              
```

Each module is a pipeline stage. Data moves forward. You can't skip a stage.

---

## Architecture

```
solana-protocol-gym/
crates/
  dc-gossip/         # Working - CRDS, peer discovery, UDP sockets
    src/
      main.rs           gossip loop, cluster info table
      contact_info.rs   ContactInfo, Version, SocketEntry
      crds.rs           CRDS table with merge/prune
      crds_data.rs      CrdsData enum, CrdsValue
      protocol.rs       Protocol enum, encode/decode
      handler.rs        message handler
      ping_pong.rs      Ping/Pong structs
      pull_request.rs   PullRequest builder
      transport.rs      UDP socket wrapper
    GOSSIP_DETAILS.md   complete debugging write-up

  dc-tvu/            # Building - full pipeline (shreds → storage → proofs)
    src/
      main.rs           UDP receiver, wires everything
      shred.rs          Shred enum, parse_from_bytes
      shred_header.rs   header structs, wire helpers, constants
      gf256.rs          GF(2^8) arithmetic for Reed-Solomon
      reed_solomon.rs   Cauchy RS encoder/decoder, matrix inversion
      fec_batch.rs      FEC batch tracker, triggers recovery
      deshredder.rs     reassemble shredded data → entries
      ring_buffer.rs    in-memory slot cache (hot storage)
      flat_file_store.rs persistent disk storage (cold)
    TVU_DESIGN.md      full protocol design doc
    PIPELINE_DESIGN.md performance pipeline design

  dc-rpc/            # Planned - RPC server for bots
  dc-poh/            # Planned - PoH verifier
  dc-consensus/      # Planned - Tower BFT
  dc-runtime/        # Planned - Sealevel executor
  dc-cli/            # Planned - CLI tools
docs/
  src/               # design docs as markdown
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

### Phase 1 — Gossip (done)
- Ping/Pong handshake with any gossip entrypoint
- PullRequest/PullResponse — discover 50+ peers
- ContactInfo decoding with all socket addresses and versions
- CRDS table with merge, prune, and dedup
- Per-value error recovery in PullResponse parsing
- Cluster info table display
- Full debugging write-up in GOSSIP_DETAILS.md

### Phase 2 — TVU / Full Pipeline (building now, includes ledger + prover)
- Bind to TVU port and receive raw shreds from the network
- Parse data shreds and coding shreds (shred_header + shred modules)
- Implement GF(2^8) arithmetic for Reed-Solomon (gf256 module)
- Cauchy RS encoder/decoder with matrix inversion (reed_solomon module)
- FEC batch tracking and automatic recovery trigger (fec_batch module)
- Deshredder: reassemble raw shred bytes into entries (deshredder module)
- Ring buffer: hot in-memory slot cache (ring_buffer module)
- Flat file store: cold persistent disk storage (flat_file_store module)
- Merkle tree builder over slot transactions (next up)
- Ledger + prover built inside dc-tvu; will split into separate crates when stable

### Phase 3 — ZK Proofs
- Build ZK circuits for transaction set verification
- Generate succinct proofs that a set of transactions was included in a slot
- Enable light clients, bridges, and coprocessors to verify Solana data with constant-size proofs
- Integrate with existing ZK ecosystems (RISC Zero, SP1, Circom)

### Phase 6 — RPC Server
- Serve blocks, transactions, and proofs to bots and onchain products
- JSON-RPC and REST interfaces
- WebSocket subscriptions for real-time data

### Phase 7 — TPU / QUIC
- Build a QUIC client that connects directly to validator TPU ports
- Send raw transaction bytes — no RPC, no middleware

### Phase 8+ — PoH, Consensus, Runtime
- Proof of History hash chain verifier
- Tower BFT simulator — votes, forks, lockouts
- Sealevel-lite parallel transaction execution

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

Where Agave optimises for production performance and abstracts away wire-level details, this project exposes them. The goal is to make the protocol understandable by building it from scratch — then ship something useful with it.

---

## Contributing

Contributions welcome — especially from developers working through the protocol stack for the first time. If something is unclear, that is a bug worth fixing.

See [CONTRIBUTING.md](CONTRIBUTING.md).

---

## License

MIT — build whatever you want with it.


