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
│   ├── Validator Monitor
│   │   ├── Real-time uptime tracking
│   │   ├── Vote analysis
│   │   └── Stake-weighted leader schedule viewer
│   │
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
