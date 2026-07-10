use crate::crds::CrdsTable;
use crate::emitter::GossipTx;
use crate::ping_pong::{Ping, Pong};
use crate::protocol::Protocol;
use crate::transport::Transport;
use anyhow::Result;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signer::keypair::Keypair;
use solana_sdk::signer::Signer;
use std::collections::HashSet;
use std::net::SocketAddr;

pub struct Handler;

impl Handler {
    /// Process an incoming gossip message.
    /// Returns newly discovered gossip addresses.
    pub async fn handle(
        msg: Protocol,
        sender: SocketAddr,
        table: &mut CrdsTable,
        tx: &GossipTx,
        transport: &Transport,
        keypair: &Keypair,
    ) -> Result<HashSet<SocketAddr>> {
        let mut new_peers = HashSet::new();

        match msg {
            Protocol::PushMessage(_, values) | Protocol::PullResponse(_, values) => {
                eprintln!("[HANDLER] got {} values from {:?}", values.len(), sender);
                for value in values {
                    let pk = value.pubkey();
                    let data_type = std::mem::discriminant(&value.data);
                    let vote_slot = if let crate::crds_data::CrdsData::Vote(_, v) = &value.data {
                        Some(v.slot)
                    } else { None };
                    if vote_slot.is_some() {
                        // eprintln!("[HANDLER] received Vote with slot={:?}", vote_slot);
                    }
                    if table.merge(value) {
                        tracing::info!("new/updated entry from {pk}");
                    }
                }

                for info in table.get_contact_infos() {
                    if info.0 != keypair.pubkey() {
                        new_peers.insert(info.1);
                    }
                }
            }

            Protocol::PingMessage(ping) => {
                let pong = Pong::new(&ping, keypair)?;
                transport
                    .send(
                        &Protocol::PongMessage(pong).encode_to()?,
                        &sender,
                    )
                    .await?;
                tracing::debug!("replied Pong to {sender}");
            }

            Protocol::PruneMessage(pubkey, _) => {
                tracing::debug!("Prune from {pubkey}");
            }

            _ => {}
        }

        // Emit events
        for event in table.drain_events() {
            let _ = tx.send(event);
        }

        Ok(new_peers)
    }
}
