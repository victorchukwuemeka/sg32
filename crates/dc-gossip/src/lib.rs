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
const DEVNET_ENTRYPOINT: &str = "entrypoint.devnet.solana.com:8001";
const DEVNET_SHRED_VERSION: u16 = 11016;
/// Run the gossip loop in the background, sending every discovered
/// ContactInfo through the channel and updating latest_slot with the
/// highest voted slot from the CRDS table.
pub async fn run_gossip_loop(
    tx: mpsc::Sender<ContactInfo>,
    latest_slot: Arc<AtomicU64>,
) -> Result<()> {
    let node = NodeKeypair::new();
    tracing::info!("Gossip identity: {}", node.pubkey());
    let entrypoint = lookup_host(DEVNET_ENTRYPOINT)
        .await?
        .next()
        .ok_or_else(|| anyhow::anyhow!("Could not resolve devnet entrypoint"))?;
    let transport = Transport::new("0.0.0.0:8000").await?;
    let public_ip = reqwest::get("https://api.ipify.org").await?.text().await?;
    let gossip_addr: SocketAddr = format!("{}:8000", public_ip).parse()?;
    // ── Ping/pong handshake ──
    let ping = Ping::new(&node.keypair)?;
    transport.send(&Protocol::PingMessage(ping).encode_to()?, &entrypoint).await?;
    for _ in 1..=10 {
        match tokio::time::timeout(Duration::from_secs(2), transport.recv()).await {
            Ok(Ok((bytes, _))) => {
                if matches!(Protocol::decode_from(&bytes)?, Protocol::PongMessage(_)) {
                    break;
                }
            }
            _ => {
                let ping = Ping::new(&node.keypair)?;
                transport.send(&Protocol::PingMessage(ping).encode_to()?, &entrypoint).await?;
            }
        }
    }
    // ── Initial pull request ──
    let contact_info = ContactInfo::new(node.pubkey(), timestamp(), gossip_addr, DEVNET_SHRED_VERSION);
    let initial_pull = create_pull_request_message(contact_info, &node.keypair)?;
    transport.send(&initial_pull, &entrypoint).await?;
    // ── Main loop ──
    let mut crds_table = crds::CrdsTable::new();
    let (gossip_tx, _gossip_rx) = emitter::create_channel();
    let mut known_peers: HashSet<SocketAddr> = HashSet::new();
    known_peers.insert(entrypoint);
    let mut last_pull = Instant::now();
    let mut last_prune = Instant::now();
    loop {
        tokio::select! {
            result = transport.recv() => {
                if let Ok((bytes, sender)) = result {
                    if let Ok(msg) = Protocol::decode_from(&bytes) {
                        match msg {
                            Protocol::PingMessage(ping) => {
                                let pong = Pong::new(&ping, &node.keypair)?;
                                transport.send(&Protocol::PongMessage(pong).encode_to()?, &sender).await?;
                            }
                            Protocol::PongMessage(_) => {}
                            msg => {
                                let new_peers = Handler::handle(msg, sender, &mut crds_table, &gossip_tx, &transport, &node.keypair).await?;
                                known_peers.extend(new_peers);
                            }
                        }
                    }
                }
            }
            _ = sleep(Duration::from_millis(500)) => {
                if last_pull.elapsed() >= Duration::from_secs(5) {
                    if known_peers.is_empty() {
                        known_peers.insert(entrypoint);
                    }
                    let contact_info = ContactInfo::new(node.pubkey(), timestamp(), gossip_addr, DEVNET_SHRED_VERSION);
                    if let Ok(bytes) = create_pull_request_message(contact_info, &node.keypair) {
                        for peer in known_peers.iter() {
                            let _ = transport.send(&bytes, peer).await;
                        }
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