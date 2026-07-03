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
use pull_request::create_pull_request_message;
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
const DEVNET_SHRED_VERSION: u16 = 11016;
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
    let public_ip = match reqwest::get("https://api.ipify.org").await {
        Ok(r) => r.text().await.unwrap_or_else(|_| "unknown".into()),
        Err(e) => {
            eprintln!("[GOSSIP] ipify failed: {e}, using 0.0.0.0");
            "0.0.0.0".into()
        }
    };
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
                if last_pull.elapsed() >= Duration::from_secs(30) {
                    if known_peers.is_empty() {
                        known_peers.insert(entrypoint);
                    }
                    let contact_info = ContactInfo::new(node.pubkey(), timestamp(), gossip_addr, DEVNET_SHRED_VERSION);
                    if let Ok(bytes) = create_pull_request_message(contact_info, &node.keypair) {
                        eprintln!("[GOSSIP] PullRequest size={}", bytes.len());
                        for peer in known_peers.iter() {
                            eprintln!("[GOSSIP] sending PullRequest to {peer}");
                            let _ = transport.send(&bytes, peer).await;
                        }
                    } else {
                        eprintln!("[GOSSIP] Failed to create PullRequest");
                    }
                    last_pull = Instant::now();
                }
                if last_prune.elapsed() >= Duration::from_secs(15) {
                    crds_table.prune();
                    let slot = crds_table.get_highest_slot().unwrap_or(0);
                    eprintln!("[GOSSIP] prune: {} entries, highest_slot={}", crds_table.len(), slot);
                    latest_slot.store(slot, Ordering::Relaxed);
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