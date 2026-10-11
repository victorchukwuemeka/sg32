pub mod contact_info;
pub mod crds;
pub mod crds_data;
pub mod crds_filter;
pub mod emitter;
pub mod handler;
pub mod ip_echo;
pub mod keypair;
pub mod legacy_contact_info;
pub mod ping_pong;
pub mod protocol;
pub mod pull_request;
pub mod short_vec;
pub mod transport;
pub mod types;
use anyhow::Result;
use contact_info::ContactInfo;
use handler::Handler;
use keypair::NodeKeypair;
use ping_pong::{Ping, Pong};
use protocol::Protocol;
use pull_request::{create_pull_request_message, create_pull_request_messages};
use solana_sdk::timing::timestamp;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::lookup_host;
use tokio::sync::mpsc;
use tokio::time::sleep;
use transport::Transport;
const DEVNET_SHRED_VERSION: u16 = 7016;
/// Fetches our public IP from a few well-known echo services, each with a short
/// timeout so a network outage can't hang startup. Falls back to "0.0.0.0".
async fn fetch_public_ip() -> String {
    const ENDPOINTS: [&str; 3] = [
        "https://api.ipify.org",
        "https://icanhazip.com",
        "https://ifconfig.me/ip",
    ];
    for url in ENDPOINTS {
        match tokio::time::timeout(Duration::from_secs(5), reqwest::get(url)).await {
            Ok(Ok(r)) => match r.text().await {
                Ok(t) if !t.trim().is_empty() => return t.trim().to_string(),
                _ => eprintln!("[GOSSIP] {url} returned empty body"),
            },
            Ok(Err(e)) => eprintln!("[GOSSIP] {url} failed: {e}"),
            Err(_) => eprintln!("[GOSSIP] {url} timed out"),
        }
    }
    eprintln!("[GOSSIP] all public-IP lookups failed, using 0.0.0.0");
    "0.0.0.0".into()
}
/// Run the gossip loop in the background, sending every discovered
/// ContactInfo through the channel and updating latest_slot with the
/// highest voted slot from the CRDS table.
pub async fn run_gossip_loop(
    tx: mpsc::Sender<ContactInfo>,
    latest_slot: Arc<AtomicU64>,
    gossip_port: u16,
    entrypoint: &str,
) -> Result<()> {
    let node = NodeKeypair::new();
    eprintln!("[GOSSIP] identity: {}", node.pubkey());
    let entrypoint = lookup_host(entrypoint)
        .await?
        .next()
        .ok_or_else(|| anyhow::anyhow!("Could not resolve entrypoint"))?;
    eprintln!("[GOSSIP] resolved entrypoint: {entrypoint}");
    let bind_addr = format!("0.0.0.0:{}", gossip_port);
    let transport = Transport::new(&bind_addr).await?;
    eprintln!("[GOSSIP] bound to {bind_addr}");
    let public_ip = fetch_public_ip().await;
    let gossip_addr: SocketAddr = format!("{}:{}", public_ip, gossip_port).parse()?;
    eprintln!("[GOSSIP] our gossip addr: {gossip_addr}");
    // ── Ping/pong handshake ──
    let ping = Ping::new(&node.keypair)?;
    eprintln!("[GOSSIP] sending Ping to {entrypoint}...");
    transport.send(&Protocol::PingMessage(ping).encode_to()?, &entrypoint).await?;
    let mut pong_ok = false;
    for attempt in 1..=10 {
        match tokio::time::timeout(Duration::from_secs(2), transport.recv()).await {
            Ok(Ok((bytes, sender))) => {
                if matches!(Protocol::decode_from(&bytes)?, Protocol::PongMessage(_)) {
                    eprintln!("[GOSSIP] Pong received from {sender} (attempt {attempt})");
                    pong_ok = true;
                    break;
                } else {
                    eprintln!("[GOSSIP] unexpected msg from {sender} during handshake ({} bytes)", bytes.len());
                }
            }
            Ok(Err(e)) => eprintln!("[GOSSIP] recv error (attempt {attempt}): {e}"),
            Err(_) => {
                eprintln!("[GOSSIP] Pong timeout (attempt {attempt}), resending Ping...");
                let ping = Ping::new(&node.keypair)?;
                transport.send(&Protocol::PingMessage(ping).encode_to()?, &entrypoint).await?;
            }
        }
    }
    if !pong_ok {
        eprintln!("[GOSSIP] WARNING: no Pong after 10 attempts, continuing anyway");
    }
    // ── Initial pull request ──
    use solana_sdk::signature::Signable;
    let contact_info = ContactInfo::new(node.pubkey(), timestamp(), gossip_addr, DEVNET_SHRED_VERSION);
    let initial_pull = create_pull_request_message(contact_info, &node.keypair)?;
    if let Ok(Protocol::PullRequest(filter, caller)) = Protocol::decode_from(&initial_pull) {
        eprintln!("[GOSSIP] mask_bits={} keys={} sig_ok={} wallclock={}", filter.mask_bits(), filter.filter.keys.len(), caller.verify(), caller.wallclock());
    }
    eprintln!("[GOSSIP] sending PullRequest ({} bytes) to {entrypoint}", initial_pull.len());
    transport.send(&initial_pull, &entrypoint).await?;
    eprintln!("[GOSSIP] PullRequest sent, waiting 15s for response...");
    for i in 1..=15 {
        match tokio::time::timeout(Duration::from_secs(1), transport.recv()).await {
            Ok(Ok((bytes, sender))) => {
                let tag = u32::from_le_bytes(bytes[..4].try_into().unwrap_or([0xff; 4]));
                eprintln!("[GOSSIP] initial-wait recv {} bytes from {} (tag={})", bytes.len(), sender, tag);
                if let Ok(Protocol::PingMessage(ping)) = Protocol::decode_from(&bytes) {
                    let pong = Pong::new(&ping, &node.keypair)?;
                    transport
                        .send(&Protocol::PongMessage(pong).encode_to()?, &sender)
                        .await?;
                    eprintln!("[GOSSIP] initial-wait replied Pong to {sender}");
                }
            }
            Ok(Err(e)) => eprintln!("[GOSSIP] initial-wait recv error: {e}"),
            Err(_) => eprintln!("[GOSSIP] initial-wait tick {i}/15"),
        }
    }
    eprintln!("[GOSSIP] done waiting, entering main loop");
    let mut crds_table = crds::CrdsTable::new();
    let (gossip_tx, _gossip_rx) = emitter::create_channel();
    let mut known_peers: HashSet<SocketAddr> = HashSet::new();
    known_peers.insert(entrypoint);
    let mut last_pull = Instant::now();
    let mut last_prune = Instant::now();
    let mut pull_round: u64 = 0;
    loop {
        tokio::select! {
            result = transport.recv() => {
                let (bytes, sender) = match result {
                    Ok(v) => {
                        let hex_prefix = v.0.iter().take(16).map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" ");
                        eprintln!("[GOSSIP] raw recv {} bytes from {} hex=[{}]", v.0.len(), v.1, hex_prefix);
                        v
                    }
                    Err(e) => {
                        eprintln!("[GOSSIP] recv error: {e}");
                        continue;
                    }
                };
                    match Protocol::decode_from(&bytes) {
                        Ok(msg) => {
                            match msg {
                                Protocol::PingMessage(ping) => {
                                    eprintln!("[GOSSIP] Ping from {sender}, sending Pong");
                                    let pong = Pong::new(&ping, &node.keypair)?;
                                    transport.send(&Protocol::PongMessage(pong).encode_to()?, &sender).await?;
                                }
                                Protocol::PongMessage(_) => {
                                    eprintln!("[GOSSIP] Pong from {sender}");
                                }
                                msg => {
                                    eprintln!("[GOSSIP] recv {} bytes from {sender}", bytes.len());
                                    let new_peers = Handler::handle(msg, sender, &mut crds_table, &gossip_tx, &transport, &node.keypair).await?;
                                    known_peers.extend(new_peers);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("[GOSSIP] decode error from {sender} ({} bytes): {e}", bytes.len());
                        }
                    }
                }
                _ = sleep(Duration::from_millis(500)) => {
                if last_pull.elapsed() >= Duration::from_secs(10) {
                    if known_peers.is_empty() {
                        known_peers.insert(entrypoint);
                    }
                    let contact_info = ContactInfo::new(node.pubkey(), timestamp(), gossip_addr, DEVNET_SHRED_VERSION);
                    match create_pull_request_messages(contact_info, &node.keypair) {
                        Ok(messages) => {
                            // Full enumeration of all filter partitions to the entrypoint:
                            // a single mask only returns ~1/2^mask_bits of the entrypoint's
                            // CRDS table, so we must ask for every bucket to discover peers.
                            for bytes in &messages {
                                let _ = transport.send(bytes, &entrypoint).await;
                            }
                            eprintln!(
                                "[GOSSIP] sent {} partition PullRequests to entrypoint {entrypoint}",
                                messages.len()
                            );
                            // To other peers, send one rotating partition per round to spread
                            // load while still eventually covering the whole table.
                            let num = messages.len() as u64;
                            if num > 0 {
                                for peer in known_peers.iter().filter(|p| **p != entrypoint) {
                                    let idx = (pull_round % num) as usize;
                                    let _ = transport.send(&messages[idx], peer).await;
                                }
                            }
                            pull_round = pull_round.wrapping_add(1);
                        }
                        Err(e) => eprintln!("[GOSSIP] Failed to create PullRequest: {e}"),
                    }
                    last_pull = Instant::now();
                }
                if last_prune.elapsed() >= Duration::from_secs(15) {
                    crds_table.prune();
                    // Only ever advance the slot. A prune that sees no votes must NOT
                    // reset the last-known good slot to 0, otherwise repair stalls every
                    // time the (rarely-published) Vote entries aren't present.
                    if let Some(slot) = crds_table.get_highest_slot() {
                        if slot > latest_slot.load(Ordering::Relaxed) {
                            latest_slot.store(slot, Ordering::Relaxed);
                        }
                    }
                    let slot = latest_slot.load(Ordering::Relaxed);
                    eprintln!("[GOSSIP] prune: {} entries, highest_slot={}", crds_table.len(), slot);
                    for (_, ci) in crds_table.all_contact_infos() {
                        // Send every discovered ContactInfo to the channel
                        let _ = tx.send(ci.clone()).await;
                    }
                    let infos = crds_table.get_contact_infos();
                    for (_, addr) in &infos {
                        known_peers.insert(*addr);
                    }
                    last_prune = Instant::now();
                }
            }
        }
    }
}