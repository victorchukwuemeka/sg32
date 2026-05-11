mod contact_info;
mod crds;
mod crds_data;
mod crds_filter;
mod emitter;
mod handler;
mod ip_echo;
mod keypair;
mod legacy_contact_info;
mod ping_pong;
mod protocol;
mod pull_request;
mod short_vec;
mod transport;
mod types;

use anyhow::Result;
use contact_info::ContactInfo;
use emitter::create_channel;
use handler::Handler;
use keypair::NodeKeypair;
use ping_pong::{Ping, Pong};
use protocol::Protocol;
use pull_request::create_pull_request_message;
use solana_sdk::timing::timestamp;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::net::lookup_host;
use tokio::time::sleep;
use transport::Transport;

const DEVNET_ENTRYPOINT: &str = "entrypoint.devnet.solana.com:8001";
const DEVNET_SHRED_VERSION: u16 = 11016;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt::init();

    let node = NodeKeypair::new();
    tracing::info!("Our node identity: {}", node.pubkey());

    let entrypoint = lookup_host(DEVNET_ENTRYPOINT)
        .await?
        .next()
        .ok_or_else(|| anyhow::anyhow!("Could not resolve devnet entrypoint"))?;

    let transport = Transport::new("0.0.0.0:8000").await?;

    let public_ip = reqwest::get("https://api.ipify.org").await?.text().await?;
    let gossip_addr: SocketAddr = format!("{}:8000", public_ip).parse()?;

    // ===== PHASE 1: PING/PONG HANDSHAKE =====

    let ping = Ping::new(&node.keypair)?;
    let ping_bytes = Protocol::PingMessage(ping).encode_to()?;
    transport.send(&ping_bytes, &entrypoint).await?;
    tracing::info!("Sent Ping to {entrypoint}, waiting for Pong...");

    let mut pong_received = false;
    for attempt in 1..=10 {
        match tokio::time::timeout(Duration::from_secs(2), transport.recv()).await {
            Ok(Ok((bytes, sender))) => match Protocol::decode_from(&bytes)? {
                Protocol::PongMessage(_) => {
                    tracing::info!("Got Pong from {sender} (attempt {attempt})");
                    pong_received = true;
                    break;
                }
                other => {
                    tracing::debug!("Got unexpected message during handshake: {other:?}");
                }
            },
            _ => {
                tracing::info!("Pong timeout (attempt {attempt}), resending Ping...");
                let ping = Ping::new(&node.keypair)?;
                let ping_bytes = Protocol::PingMessage(ping).encode_to()?;
                transport.send(&ping_bytes, &entrypoint).await?;
            }
        }
    }

    if !pong_received {
        tracing::warn!("No Pong received after 10 attempts");
    }

    // ===== PHASE 2: SEND INITIAL PULL REQUEST TO ENTRYPOINT =====

    let contact_info = ContactInfo::new(
        node.pubkey(),
        timestamp(),
        gossip_addr,
        DEVNET_SHRED_VERSION,
    );
    let initial_pull = create_pull_request_message(contact_info, &node.keypair)
        .map_err(|e| anyhow::anyhow!("Failed to create pull request: {e}"))?;
    let hex_preview: String = initial_pull[..initial_pull.len().min(64)]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    tracing::info!(
        "PullRequest: {} bytes, hex preview (first 64): {}",
        initial_pull.len(),
        hex_preview
    );
    transport.send(&initial_pull, &entrypoint).await?;
    tracing::info!("Initial PullRequest sent to entrypoint, listening for response...");

    // Listen for a few seconds after sending — the entrypoint may send a Ping back
    for i in 1..=6 {
        match tokio::time::timeout(Duration::from_millis(500), transport.recv()).await {
            Ok(Ok((bytes, sender))) => {
                tracing::info!("GOT PACKET from {sender}: {} bytes", bytes.len());
                match Protocol::decode_from(&bytes) {
                    Ok(Protocol::PingMessage(ping)) => {
                        tracing::info!(">> Got Ping from {sender}, sending Pong");
                        let pong = Pong::new(&ping, &node.keypair)?;
                        transport.send(
                            &Protocol::PongMessage(pong).encode_to()?,
                            &sender,
                        ).await?;
                    }
                    Ok(Protocol::PushMessage(pk, vals)) => {
                        tracing::info!(">> Got PushMessage from {pk}: {} values", vals.len());
                    }
                    Ok(Protocol::PullResponse(pk, vals)) => {
                        tracing::info!(">> Got PullResponse from {pk}: {} values", vals.len());
                    }
                    Ok(other) => {
                        tracing::info!(">> Got {:?}", other);
                    }
                    Err(e) => {
                        let h: String = bytes[..bytes.len().min(32)]
                            .iter().map(|b| format!("{b:02x}"))
                            .collect::<Vec<_>>().join(" ");
                        tracing::warn!(">> decode error: {e}, hex: {h}");
                    }
                }
            }
            Ok(Err(e)) => tracing::warn!("recv error: {e}"),
            Err(_) => tracing::info!("listen window {i}/6 — no packet"),
        }
    }
    tracing::info!("Entering main gossip loop");

    // ===== PHASE 3: MAIN GOSSIP LOOP =====

    let mut crds_table = crds::CrdsTable::new();
    let (gossip_tx, _gossip_rx) = create_channel();
    let mut known_peers: HashSet<SocketAddr> = HashSet::new();
    known_peers.insert(entrypoint);

    let mut last_pull = Instant::now();
    let mut last_prune = Instant::now();

    loop {
        tokio::select! {
            result = transport.recv() => {
                match result {
                    Ok((bytes, sender)) => {
                        match Protocol::decode_from(&bytes) {
                            Ok(Protocol::PingMessage(ping)) => {
                                let pong = Pong::new(&ping, &node.keypair)?;
                                transport.send(
                                    &Protocol::PongMessage(pong).encode_to()?,
                                    &sender,
                                ).await?;
                                tracing::debug!("replied Pong to {sender}");
                            }
                            Ok(Protocol::PongMessage(_)) => {
                                tracing::debug!("Pong from {sender}");
                            }
                            Ok(msg) => {
                                tracing::debug!("recv {} bytes from {sender}", bytes.len());
                                let new_peers = Handler::handle(
                                    msg,
                                    sender,
                                    &mut crds_table,
                                    &gossip_tx,
                                    &transport,
                                    &node.keypair,
                                ).await?;
                                known_peers.extend(new_peers);
                            }
                            Err(e) => {
                                let hex_preview: String = bytes[..bytes.len().min(32)]
                                    .iter()
                                    .map(|b| format!("{b:02x}"))
                                    .collect::<Vec<_>>()
                                    .join(" ");
                                tracing::warn!(
                                    "decode error from {sender} ({} bytes, hex: {}) — {e}",
                                    bytes.len(),
                                    hex_preview,
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("recv error: {e}");
                    }
                }
            }

            _ = sleep(Duration::from_millis(500)) => {
                if last_pull.elapsed() >= Duration::from_secs(5) {
                    if known_peers.is_empty() {
                        known_peers.insert(entrypoint);
                    }

                    let contact_info = ContactInfo::new(
                        node.pubkey(),
                        timestamp(),
                        gossip_addr,
                        DEVNET_SHRED_VERSION,
                    );
                    match create_pull_request_message(contact_info, &node.keypair) {
                        Ok(bytes) => {
                            tracing::info!(
                                "sending PullRequest ({} bytes) to {} peers",
                                bytes.len(),
                                known_peers.len()
                            );
                            for peer in known_peers.iter() {
                                if let Err(e) = transport.send(&bytes, peer).await {
                                    tracing::warn!("pull to {peer} failed: {e}");
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("build pull request failed: {e}");
                        }
                    }
                    last_pull = Instant::now();
                }

                if last_prune.elapsed() >= Duration::from_secs(30) {
                    crds_table.prune();
                    tracing::info!(
                        "CRDS: {} entries, {} peers",
                        crds_table.len(),
                        known_peers.len(),
                    );
                    last_prune = Instant::now();
                }
            }
        }
    }
}
