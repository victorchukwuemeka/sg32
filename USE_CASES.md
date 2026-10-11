## sg32 — Shipping Priority 

---

### TIER 1: Ship in 2–4 weeks

| # | Product | Why First | Buyer |
|---|---------|-----------|-------|
| 1 | Trading Bot API | You already parse shreds → txs. Add program filtering (Jupiter, Orca), account delta tracking, WebSocket stream. One quant with a strategy pays more than 100 devs playing with an SDK. | Quant funds, MEV searchers |
| 2 | WebSocket Feed | Same core, different interface. Stream proven txs. Charge by connection/volume. | Bot builders, dashboards, wallets |
| 3 | MEV Analysis Feed | Parse swaps, detect sandwiches, track validator tips. Tweet the results daily. This is marketing that ships itself — every thread gets retweeted by MEV researchers. | Researchers, Twitter, inbound leads |

---

### TIER 2: Ship in 1–2 months

| # | Product | Why Now | Buyer |
|---|---------|---------|-------|
| 4 | Trustless Price Oracle | Index DEX prices from your stream, serve with Merkle proof. DeFi protocols are paranoid about oracle hacks. You offer "prove the price came from the chain" as a feature. | DeFi protocols (Drift, Jupiter, margin protocols) |
| 5 | Data Archive | Store every block with Merkle root. Query by slot, program, time range. Researchers and compliance need history, not just real-time. | Researchers, compliance tools, AI/ML |
| 6 | Block Explorer | Your own explorer with real data. Not for revenue — for credibility. "Look, we built this ourselves." | Marketing, developer trust |

---

### TIER 3: Ship in 3–6 months

| # | Product | Why Wait | Buyer |
|---|---------|----------|-------|
| 7 | ZK Coprocessor demo | Wrap one Merkle proof in RISC Zero/SP1. Prove one Jupiter swap on Ethereum. One working demo = inbound from every bridge team. Hard but narrative-defining. | Bridge teams, L2s, cross-chain protocols |
| 8 | Light Client SDK | Open source it. JS/Python/Rust. Sub-100KB proofs. Let the community build on it. Revenue comes from hosted proof generation, not the SDK itself. | Developers, wallets, mobile apps |
| 9 | Trustless RPC | Drop-in replacement for public RPC. Needs scale and reliability. Charge by request. Harder to differentiate until you have reputation. | Dapps, wallets, small protocols |

---

### TIER 4: 6+ months / needs core work first

| # | Product | Blocker |
|---|---------|---------|
| 10 | Solana → EVM Bridge | Needs ZK coprocessor + leader signature verification |
| 11 | Custom Gossip Node | Needs weighted shuffle, repair protocol, second UDP socket |
| 12 | Real Turbine Node | Needs full cluster topology participation |
| 13 | Compliance / Forensics | Needs sales cycle, regulatory relationships, legal review |
| 14 | Private Mempool | Needs validator stake + relationships |

---

### TIER 5: The vision, not the product

| # | Product | When |
|---|---------|------|
| 15 | On-Chain Bloomberg Terminal | When you have 3+ revenue streams from Tier 1–3, hire a designer, and have 50 API users asking for a UI. |

---

### The Clarity

Months 1–2:   Trading API + MEV feed. Get one paying quant. Tweet daily.
Months 3–4:   Oracle + archive. Land one DeFi protocol.
Months 5–6:   ZK demo. Blow up on Twitter.
Month 7+:     SDK open source. Let community build the Bloomberg while you sell the pipes.

---

### The Rule

Sell pipes, not terminals. Bloomberg built terminals because they owned proprietary data. Your data is public — your moat is trust and speed, not exclusivity. So sell the infrastructure that makes trust and speed valuable.


































# sg32 — Use Case Tree

sg32 is a trustless Solana data pipeline. Every product below is built on one core primitive: **real-time, verifiable block data with Merkle proofs, no RPC trust required.**

```
sg32
├── Infrastructure Layer
│   ├── Trustless RPC
│   │   └── Drop-in replacement for public RPC endpoints
│   │     └── No rate limits, no API keys, no trust
│   │
│   ├── Custom Gossip Node
│   │   ├── Receives shreds as a real cluster peer (Turbine)
│   │   ├── Retransmits to downstream peers
│   │   └── Requests missing data via repair protocol
│   │
│   ├── Data Archive
│   │   ├── Every block stored with Merkle root
│   │   ├── Queryable by slot, time range, or program ID
│   │   └── Verifiable at rest (re-derive root from stored data)
│   │
│   └── Private Mempool
│       └── Dedicated validator connections for exclusive tx flow
│
├── Bridge & Cross-Chain
│   ├── Solana → EVM Bridge
│   │   └── Prove tx inclusion on Ethereum with Merkle proof
│   │
│   ├── Solana → Cosmos Bridge
│   │   └── IBC-compatible light client powered by sg32 proofs
│   │
│   ├── ZK Coprocessor (RISC Zero / SP1)
│   │   ├── Wrap Merkle proof in ZKSNARK
│   │   └── Verify Solana state on any chain for < 100k gas
│   │
│   └── Wormhole / LayerZero Integration
│       └── Feed proven block data into existing bridge networks
│
├── Financial Data
│   ├── On-Chain Bloomberg Terminal
│   │   ├── Real-time price feeds (Jupiter, Orca, Raydium)
│   │   ├── Pool TVL, volume, fee tracking
│   │   ├── Liquidation alerts
│   │   ├── Whale wallet tracker
│   │   └── MEV analysis (sandwich, backrun detection)
│   │
│   ├── Oracle
│   │   ├── Trustless price oracle for protocols
│   │   ├── No off-chain aggregation dependency
│   │   └── Each data point backed by a Merkle proof
│   │
│   ├── Trading Bot API
│   │   ├── Low-latency tx stream with proofs
│   │   ├── Program-specific filtering (Jupiter swaps only, etc.)
│   │   └── Real-time account delta tracking
│   │
│   └── Portfolio Tracker
│       └── Historical + real-time PnL across all holdings
│
├── Compliance & Security
│   ├── Compliance Monitor
│   │   ├── Track wallet activity with provable data
│   │   ├── Sanctions screening on-chain
│   │   └── Audit trail for regulators
│   │
│   ├── Proof of Reserves
│   │   ├── Exchanges prove holdings via on-chain data
│   │   └── Verifiable without sharing private keys
│   │
│   └── Forensics Tool
│       ├── Trace stolen funds across accounts
│       ├── Reconstruct hack timelines
│       └── Evidence admissible via cryptographic proof
│
├── Developer Tools
│   ├── Light Client SDK
│   │   ├── Verify tx proofs in browser, mobile, or server
│   │   ├── JS / Python / Rust SDK
│   │   └── Sub-100KB state proof downloads
│   │
│   ├── Block Explorer
│   │   └── Your own explorer with real data, no third-party API
│   │
│   ├── Debugger & Simulator
│   │   ├── Replay any slot locally
│   │   └── Inspect entry-by-entry, tx-by-tx
│   │
│   └── WebSocket Feed
│       └── Stream proven txs to subscribers in real-time
│
├── Staking & Validators
│   ├── MEV Distribution Tracker
│   │   └── Track validator tips & MEV rewards per slot
│   │
│   └── Delegation Analytics
│       └── Compare validator performance for stakers
│
└── Solana Ecosystem
    ├── AI / ML
    │   ├── Train models on provable on-chain data
    │   └── Market prediction, anomaly detection
    │
    ├── Gaming
    │   ├── Verify in-game actions on-chain
    │   └── Provable randomness from block hashes
    │
    └── Research
        ├── Economic analysis of Solana network
        ├── MEV research with complete data
        └── Network performance measurement
```

## The Pattern

Every product follows the same flow:

```
sg32 core (shreds → proofs)
  → filter / parse / index
    → serve with proof
      → consumer verifies independently
```

No one in the ecosystem offers this. The closest is a public RPC — but that requires trust. The closest to *us* is a full validator — but that requires terabytes of state and weeks of setup. sg32 sits in the middle: trustless without the overhead.
