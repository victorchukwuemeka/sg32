# sg32 — Roadmap to Launch

## Phase 1: Ship (current week)

- [ ] Fix `getLatestSlot` to fall back to file store
- [ ] Publish SDK to npm (`@sg32/sdk`)
- [ ] Write a working 10-line example app (Node.js)
- [ ] Run node 24h+ on devnet, log crashes/stalls
- [ ] Deploy public endpoint (fly.io or your own VPS)
- [ ] Integration test suite — shell script that curls all endpoints and asserts shapes

## Phase 2: Earn Trust (next 2 weeks)

- [ ] Get a domain + SSL (e.g. rpc.sg32.io)
- [ ] Documentation site or README with copy-paste examples
- [ ] 7-day uptime on public endpoint
- [ ] Add `getBlocks` endpoint (list available slots)
- [ ] Add `getLatestBlockhash` (read from gossip/saved blocks)
- [ ] Handle edge cases gracefully (0-tx blocks, malformed shreds, etc.)

## Phase 3: Monetize (after Phase 2)

- [ ] Rate limiting + API keys
- [ ] Free tier: 10 req/s, 1,000 tx proofs/day
- [ ] Paid tier: 100 req/s, unlimited proofs, SLA, dedicated endpoint
- [ ] Paid tier: webhooks for bridge relayers (notify on slot finality)
- [ ] Bridge/relayer SDK integration guide
- [ ] Enterprise: custom deployment, private network, support

## Never

- Run a bank / account state / execute transactions
- Compete with Helius/Triton on standard RPC
- Become a general-purpose Solana node
