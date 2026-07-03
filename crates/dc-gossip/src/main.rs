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
use crds_data::{CrdsData, CrdsValue};
use crds_filter::CrdsFilter;
use emitter::create_channel;
use handler::Handler;
use keypair::NodeKeypair;
use ping_pong::{Ping, Pong};
use protocol::Protocol;
use pull_request::create_pull_request_message;
use solana_sdk::signer::Signer;
use solana_sdk::timing::timestamp;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tokio::net::lookup_host;
use tokio::time::sleep;
use solana_sdk::signer::keypair::Keypair;
use transport::Transport;
use tracing_appender::rolling;
use tracing_subscriber::fmt::writer::MakeWriterExt;

const DEVNET_ENTRYPOINT: &str = "entrypoint.devnet.solana.com:8001";
const DEVNET_SHRED_VERSION: u16 = 7016;

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

fn self_test() -> Result<()> {
    let keypair = Keypair::new();
    let gossip: SocketAddr = "1.2.3.4:8001".parse().unwrap();

    println!("=== SELF TEST ===");

    // 1) Create ContactInfo with a fixed wallclock so output is reproducible
    let ci = ContactInfo::new(keypair.pubkey(), 1_000_000, gossip, 11016);

    // Also serialize ContactInfo alone (no CrdsData wrapper) to see its size
    let ci_bytes = bincode::serialize(&ci)?;
    println!("ContactInfo alone: {} bytes", ci_bytes.len());
    println!("Hex: {}", hex_bytes(&ci_bytes));

    // Serialize just the Version field to see its size
    let ver_bytes = bincode::serialize(&ci.version)?;
    println!("Version alone: {} bytes", ver_bytes.len());
    println!("Hex: {}", hex_bytes(&ver_bytes));

    // Serialize just the addrs Vec
    let addrs_bytes = bincode::serialize(&ci.addrs)?;
    println!("addrs Vec: {} bytes", addrs_bytes.len());
    println!("Hex: {}", hex_bytes(&addrs_bytes));

    // Serialize just the sockets Vec
    let socks_bytes = bincode::serialize(&ci.sockets)?;
    println!("sockets Vec: {} bytes", socks_bytes.len());
    println!("Hex: {}", hex_bytes(&socks_bytes));

    // 2) Serialize CrdsData::ContactInfo alone — this is what gets SIGNED
    let data = CrdsData::ContactInfo(ci.clone());
    let crds_bytes = bincode::serialize(&data)?;
    println!("\nCrdsData::ContactInfo: {} bytes (expected: 4+{})", crds_bytes.len(), ci_bytes.len());
    println!("Hex: {}", hex_bytes(&crds_bytes));

    // Breakdown of the CrdsData bytes:
    println!("  [0..4]   CrdsData tag: {:02x?}", &crds_bytes[..4]);
    println!("  [4..]    ContactInfo: {:02x?}", &crds_bytes[4..]);

    // 3) Round-trip check: bincode → deserialize → re-serialize → bytes must match
    let data2: CrdsData = bincode::deserialize(&crds_bytes)?;
    let crds_bytes2 = bincode::serialize(&data2)?;
    if crds_bytes == crds_bytes2 {
        println!("Round-trip: OK (same bytes)");
    } else {
        println!("Round-trip: MISMATCH ({} vs {} bytes)", crds_bytes.len(), crds_bytes2.len());
        println!("  First:  {}", hex_bytes(&crds_bytes));
        println!("  Second: {}", hex_bytes(&crds_bytes2));
    }

    // 4) Create full CrdsValue with signature
    let cv = CrdsValue::new_contact_info(ci.clone(), &keypair);
    let cv_bytes = bincode::serialize(&cv)?;
    println!("\nCrdsValue: {} bytes (sig=64 + data={})", cv_bytes.len(), crds_bytes.len());
    println!("Hex: {}", hex_bytes(&cv_bytes));

    // 5) Self-verify the signature
    let sig_ok = cv.signature.verify(keypair.pubkey().as_ref(), &crds_bytes);
    println!("Signature self-verify: {}", if sig_ok { "OK" } else { "FAIL" });

    // 6) Create the full PullRequest and check it fits
    let filter = CrdsFilter::new(512, 0);
    let pr = Protocol::PullRequest(filter, cv);
    let pr_bytes = bincode::serialize(&pr)?;
    println!("\nProtocol::PullRequest: {} bytes (max packet: 1232)", pr_bytes.len());
    if pr_bytes.len() <= 1232 {
        println!("  Fits ({} bytes spare)", 1232 - pr_bytes.len());
    } else {
        println!("  OVERFLOW by {} bytes!", pr_bytes.len() - 1232);
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let args: Vec<String> = std::env::args().collect();
    if args.contains(&"--self-test".to_string()) {
        return self_test();
    }
    let file_appender = rolling::never("logs", "main-logs.txt");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    let stderr = std::io::stderr.with_max_level(tracing::Level::TRACE);
    tracing_subscriber::fmt()
        .with_writer(stderr.and(non_blocking))
        .with_ansi(false)
        .init();

    let node = NodeKeypair::new();
    tracing::info!("Our node identity: {}", node.pubkey());

    let entrypoint = lookup_host(DEVNET_ENTRYPOINT)
        .await?
        .next()
        .ok_or_else(|| anyhow::anyhow!("Could not resolve devnet entrypoint"))?;

    let transport = Transport::new("0.0.0.0:8001").await?;

    let public_ip = reqwest::get("https://api.ipify.org").await?.text().await?;
    let gossip_addr: SocketAddr = format!("{}:8001", public_ip).parse()?;

    // ===== PHASE 1: PING/PONG HANDSHAKE =====

    let ping = Ping::new(&node.keypair)?;
    let ping_bytes = Protocol::PingMessage(ping).encode_to()?;
    tracing::info!(
        "[TX] PingMessage {} bytes to {entrypoint}",
        ping_bytes.len()
    );
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
    tracing::info!(
        "[TX_PULL] contact_info: pubkey={} version={}.{}.{} wallclock={} gossip={}",
        contact_info.pubkey(),
        contact_info.version.major,
        contact_info.version.minor,
        contact_info.version.patch,
        contact_info.wallclock,
        gossip_addr,
    );
    let initial_pull = create_pull_request_message(contact_info, &node.keypair)
        .map_err(|e| anyhow::anyhow!("Failed to create pull request: {e}"))?;
    let hex_preview: String = initial_pull[..initial_pull.len().min(64)]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    tracing::info!(
        "[TX_PULL] body {} bytes, hex(first 64): {}",
        initial_pull.len(),
        hex_preview,
    );
    transport.send(&initial_pull, &entrypoint).await?;
    tracing::info!("Initial PullRequest sent to entrypoint, listening for response...");

    // Listen for a few seconds after sending — the entrypoint may send a Ping back
    for i in 1..=6 {
        match tokio::time::timeout(Duration::from_millis(500), transport.recv()).await {
            Ok(Ok((bytes, sender))) => {
                let hex32: String = bytes[..bytes.len().min(32)]
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                tracing::info!(
                    "[RX] {} bytes from {sender}, hex(first32): {hex32}",
                    bytes.len()
                );
                match Protocol::decode_from(&bytes) {
                    Ok(Protocol::PingMessage(ping)) => {
                        tracing::info!(">> Got Ping from {sender}, sending Pong");
                        let pong = Pong::new(&ping, &node.keypair)?;
                        transport
                            .send(&Protocol::PongMessage(pong).encode_to()?, &sender)
                            .await?;
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
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<Vec<_>>()
                            .join(" ");
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
                                let type_name = match &msg {
                                    Protocol::PullRequest(_, _) => "PullRequest",
                                    Protocol::PullResponse(_, _) => "PullResponse",
                                    Protocol::PushMessage(_, _) => "PushMessage",
                                    Protocol::PruneMessage(_, _) => "PruneMessage",
                                    Protocol::PingMessage(_) => "PingMessage",
                                    Protocol::PongMessage(_) => "PongMessage",
                                    Protocol::Unknown => "Unknown",
                                };
                                tracing::debug!("[RX] {} {type_name} {} bytes from {sender}", bytes.len(), bytes.len());
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
                                let hex_preview: String = bytes[..bytes.len().min(64)]
                                    .iter()
                                    .map(|b| format!("{b:02x}"))
                                    .collect::<Vec<_>>()
                                    .join(" ");
                                tracing::warn!(
                                    "[RX_FAIL] decode error from {sender} ({} bytes, hex: {}) — {e}",
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
                            let hex64: String = bytes[..bytes.len().min(64)]
                                .iter().map(|b| format!("{b:02x}"))
                                .collect::<Vec<_>>().join(" ");
                            tracing::info!(
                                "[TX] PullRequest {} bytes to {} peers hex(64): {hex64}",
                                bytes.len(),
                                known_peers.len(),
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
                    let infos = crds_table.get_contact_infos();
                    tracing::info!(
                        "CRDS: {} entries, {} gossip peers",
                        crds_table.len(),
                        infos.len(),
                    );
                    if !infos.is_empty() {
                        tracing::info!("  {:>18} | {:>5} | {:45} | {:7} | Gossip | TPUvote | TPU | TPUfwd | TVU | TVU Q | ServeR | ShredVer", "IP Address", "Age(ms)", "Node identifier", "Version");
                        tracing::info!("  {}", "-".repeat(128));
                        for (_, ci) in crds_table.all_contact_infos() {
                            tracing::info!("{}", ci.table_row());
                        }
                        tracing::info!("  Nodes: {}", infos.len());
                    }
                    // Add discovered gossip peers to known_peers
                    for (_, addr) in &infos {
                        known_peers.insert(*addr);
                    }
                    last_prune = Instant::now();
                }
            }
        }
    }
}
