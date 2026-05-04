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
use crds::CrdsTable;
//use emitter::create_channel;
use crate::ping_pong::{Ping, Pong};
use ip_echo::get_cluster_info;
use keypair::NodeKeypair;
use solana_bloom::bloom::Bloom;
use solana_sdk::hash::Hash;
use std::net::SocketAddr;
use tokio::net::lookup_host;
use transport::Transport;

const DEVNET_ENTRYPOINT: &str = "entrypoint.devnet.solana.com:8001";
const DEVNET_SHRED_VERSION: u16 = 11016;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt::init();

    // who we are first from i mean our node
    let node = NodeKeypair::new();
    tracing::info!("our node identity: {}", node.pubkey());

    // our entrypoint to the devnet.
    let entrypoint = lookup_host(DEVNET_ENTRYPOINT)
        .await?
        .next()
        .ok_or_else(|| anyhow::anyhow!("could not resolve devnet entrypoint"))?;

    //binding our udp socket
    let transport = Transport::new("0.0.0.0:8001").await?;
    //let mut table = CrdsTable::new();
    //let (tx, _rx) = create_channel();
    //let ping = ping::Ping::new(&node.keypair)?;
    //let pin_msg = protocol::Protocol::PingMessage(ping);
    //let ping_bytes = protocol::Protocol::encode_to(&pin_msg)?;

    //transport.send(&ping_bytes, &entrypoint).await?;
    //tracing::info!("Ping sent to devnet entrypoint");

    //let gossip_addr: SocketAddr = "102.89.22.11:8001".parse()?;
    let public_ip = reqwest::get("https://api.ipify.org").await?.text().await?;
    let gossip_addr: SocketAddr = format!("{}:8001", public_ip).parse()?;
    let info = ContactInfo::new(node.pubkey(), gossip_addr, 0u16);

    let caller = protocol::CrdsValue::new_contact_info(info, &node.keypair);
    let bloom: Bloom<Hash> = Bloom::random(1024, 0.1, 1024);

    let filter = protocol::CrdsFilter {
        filter: bloom,
        mask: !0u64,
        mask_bits: 0,
    };

    let pull_request = protocol::Protocol::PullRequest(filter, caller);
    let pull_bytes = pull_request.encode_to()?;
    transport.send(&pull_bytes, &entrypoint).await?;
    tracing::info!("PullRequest sent to devnet");

    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(5), transport.recv()).await {
            Ok(Ok((bytes, sender))) => {
                tracing::info!("GOT PACKET from {} — {} bytes ", sender, bytes.len(),);
                match protocol::Protocol::decode_from(&bytes) {
                    Ok(protocol::Protocol::PingMessage(ping)) => {
                        tracing::info!("Got Ping from {} — sending Pong", sender);
                        tracing::info!("Ping from {} — sending Pong", sender);
                        let pong = ping_pong::Pong::new(&ping, &node.keypair)?;
                        let pong_msg = protocol::Protocol::PongMessage(pong);
                        let pong_bytes = pong_msg.encode_to()?;
                        transport.send(&pong_bytes, &sender).await?;
                        tracing::info!("Pong sent to {}", sender);
                    }
                    Ok(protocol::Protocol::PullResponse(pubkey, values)) => {
                        tracing::info!("PullResponse from {} — {} values 🔥", pubkey, values.len());
                    }
                    Ok(protocol::Protocol::PushMessage(pubkey, values)) => {
                        tracing::info!("PushMessage from {} — {} values 🔥", pubkey, values.len());
                    }
                    Ok(other) => {
                        tracing::info!("other message: {:?}", other);
                    }
                    Err(_) => {
                        // can't decode yet — print raw bytes
                        tracing::info!("raw bytes: {:?}", &bytes[..bytes.len().min(16)]);
                    }
                }
            }
            Ok(Err(e)) => tracing::error!("recv error: {}", e),
            Err(_) => {
                // every 1s resend PullRequest
                transport.send(&pull_bytes, &entrypoint).await?;
                tracing::info!("PullRequest resent");
            }
        }
    }
}
