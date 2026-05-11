use ::{
    serde::{Deserialize, Serialize},
    solana_sdk::pubkey::Pubkey,
    std::net::SocketAddr,
};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct LegacyContactInfo {
    pub id: Pubkey,
    pub gossip: SocketAddr,
    pub tvu: SocketAddr,
    pub tvu_quic: SocketAddr,
    pub serve_repair_quic: SocketAddr,
    pub tpu: SocketAddr,
    pub tpu_forwards: SocketAddr,
    pub tpu_vote: SocketAddr,
    pub rpc: SocketAddr,
    pub rpc_pubsub: SocketAddr,
    pub wallclock: u64,
    pub shred_version: u16,
}

impl LegacyContactInfo {
    pub fn pubkey(&self) -> &Pubkey {
        &self.id
    }

    pub fn new_spy(id: Pubkey, gossip: SocketAddr, shred_version: u16) -> Self {
        let zero: SocketAddr = "0.0.0.0:0".parse().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        Self {
            id,
            gossip,
            tvu: zero,
            tvu_quic: zero,
            serve_repair_quic: zero,
            tpu: zero,
            tpu_forwards: zero,
            tpu_vote: zero,
            rpc: zero,
            rpc_pubsub: zero,
            wallclock: now,
            shred_version,
        }
    }
}
