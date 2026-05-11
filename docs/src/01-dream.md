## 1. The Dream

Build a minimal Rust crate that speaks the Solana gossip protocol — the peer-to-peer discovery and data propagation layer that every Solana validator uses to find each other and exchange information.

The goal: connect to the Solana devnet entrypoint (`35.197.53.105:8001`), perform the handshake, send a PullRequest asking "who's on the network?", and actually receive valid gossip data back.

No fork of Agave. No importing the entire Solana monorepo. Just a standalone binary that sends and receives the right bytes.
