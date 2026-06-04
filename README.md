# SG32 — Solana Light Node

Shred recovery, FEC reconstruction, and trustless Merkle proofs. One binary, no sidecars.

## Quick Start

```bash
cargo run --release
```

Open http://localhost:8899 in your browser. The live dashboard shows real-time shred progress and recovered blocks.

### CLI Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--rpc-port` | 8899 | HTTP RPC server port |
| `--repair-port` | 8003 | UDP repair socket port |
| `--gossip-port` | 8001 | UDP gossip socket port |
| `--data-dir` | data | Directory for block storage |
| `--entrypoint` | entrypoint.devnet.solana.com:8001 | Gossip entrypoint |

### Mainnet

```bash
cargo run --release -- --entrypoint entrypoint.mainnet-beta.solana.com:8001
```

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                   sg32 (single binary)               │
├─────────────────────────────────────────────────────┤
│                                                      │
│  Gossip ──> Repair ──> FEC Recovery ──> Deshredder  │
│  (:8001)    (:8003)     (Reed-Solomon)   Entries→TXs │
│                                              │       │
│                                              ▼       │
│  RPC ←── Merkle Tree ←── Deshredder                 │
│  (:8899)                                            │
│    │                                                 │
│    ▼                                                 │
│  Memory (ring buffer) + Flat File (disk)            │
│                                                      │
└─────────────────────────────────────────────────────┘
         │
         ▼
  Your Bot / Relayer / Light Client
  verify(tx)  verify(proof)
```

## API

All methods use JSON-RPC 2.0 over POST to `/jsonrpc`.

### getLatestSlot

```bash
curl -X POST http://localhost:8899/jsonrpc \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getLatestSlot","id":1}'
```

### getSlot

```bash
curl -X POST http://localhost:8899/jsonrpc \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getSlot","params":[466680759],"id":1}'
```

### getBlock

```bash
curl -X POST http://localhost:8899/jsonrpc \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getBlock","params":[466680759],"id":1}'
```

### getProof (Merkle inclusion proof)

```bash
curl -X POST http://localhost:8899/jsonrpc \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","method":"getProof","params":[466680759, 0],"id":1}'
```

Response includes `verified: true` if the transaction is cryptographically proven in the block.

### /stats

GET `/stats` returns live pipeline state:

```json
{
  "latest_slot": 467199041,
  "current_batch": { "slot": 467199041, "data_shreds": 31, "num_data": 32, "code_shreds": 0, "num_code": 32 },
  "total_blocks_recovered": 37,
  "blocks_in_ring_buffer": 5,
  "files_on_disk": 45,
  "latest_block_txs": 0,
  "latest_block_root": "0000000000000000000000000000000000000000000000000000000000000000"
}
```

## Dashboard

Visit http://localhost:8899 for the live terminal-themed dashboard with real-time shred bars, slot tracking, and recovered block count.

## For Relayers and Bridges

1. User submits transaction to Solana
2. Your relayer detects the tx in a confirmed block
3. Your relayer calls `getProof(slot, tx_index)` on sg32
4. It checks `proof.verified == true`
5. It independently verifies the Merkle root against a trusted source
6. If both match, the transaction is provably included

No RPC provider to trust. No full node to run.

## License

MIT
