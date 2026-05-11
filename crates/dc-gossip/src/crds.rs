use crate::crds_data::{CrdsData, CrdsValue};
use crate::emitter::GossipEvent;
use crate::types::ValidatorInfo;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub struct CrdsTable {
    entries: HashMap<Pubkey, CrdsValue>,
}

impl CrdsTable {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Merge a value. Returns true if it was new or newer.
    pub fn merge(&mut self, value: CrdsValue) -> bool {
        let key = value.pubkey();
        let incoming = value.wallclock();

        match self.entries.get(&key) {
            Some(existing) if existing.wallclock() >= incoming => false,
            _ => {
                self.entries.insert(key, value);
                true
            }
        }
    }

    pub fn prune(&mut self) {
        let cutoff = now() - 15 * 60;
        self.entries.retain(|_, v| v.wallclock() > cutoff);
    }

    pub fn get_contact_infos(&self) -> Vec<(Pubkey, SocketAddr)> {
        self.entries
            .iter()
            .filter_map(|(pk, value)| match &value.data {
                CrdsData::ContactInfo(ci) => ci.gossip_addr().map(|addr| (*pk, addr)),
                CrdsData::LegacyContactInfo(lci) => Some((*pk, lci.gossip)),
                _ => None,
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn drain_events(&mut self) -> Vec<GossipEvent> {
        let mut events = vec![];
        for value in self.entries.values() {
            let pk = value.pubkey().to_string();
            if let Some(addr) = match &value.data {
                CrdsData::ContactInfo(ci) => ci.gossip_addr(),
                CrdsData::LegacyContactInfo(lci) => Some(lci.gossip),
                _ => None,
            } {
                events.push(GossipEvent::NewValidators(ValidatorInfo {
                    id: pk,
                    gossip_addr: addr,
                    tvu_addr: None,
                    tpu_addr: None,
                    last_seen: now(),
                    version: value.wallclock(),
                }));
            }
        }
        events
    }
}
