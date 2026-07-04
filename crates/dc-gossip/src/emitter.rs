use crate::types::{SlotInfo, ValidatorInfo};
use tokio::sync::broadcast;

#[derive(Clone, Debug)]
pub enum GossipEvent {
    NewValidators(ValidatorInfo),
    ValidatorUpdate(ValidatorInfo),
    SlotUpdate(SlotInfo),
    PeerLeft(String),
}

pub type GossipTx = broadcast::Sender<GossipEvent>;
pub type GossipRx = broadcast::Receiver<GossipEvent>;

pub fn create_channel() -> (GossipTx, GossipRx) {
    broadcast::channel(1000)
}
