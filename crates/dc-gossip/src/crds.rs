use crate::contact_info::ContactInfo;
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

    pub fn all_contact_infos(&self) -> Vec<(Pubkey, &ContactInfo)> {
        self.entries
            .iter()
            .filter_map(|(pk, value)| match &value.data {
                CrdsData::ContactInfo(ci) => Some((*pk, ci)),
                _ => None,
            })
            .collect()
    }

    /// Return the highest slot seen across all votes and restart records.
    pub fn get_highest_slot(&self) -> Option<solana_sdk::clock::Slot> {
        let result = self.entries.values().filter_map(|value| match &value.data {
            CrdsData::Vote(_, vote) => vote.slot,
            CrdsData::RestartLastVotedForkSlots(s) => Some(s.last_voted_slot),
            CrdsData::RestartHeaviestFork(f) => Some(f.last_slot),
            _ => None,
        }).max();
        let vote_count = self.entries.values().filter(|v| matches!(&v.data, CrdsData::Vote(_, _))).count();
        let total = self.entries.len();
        eprintln!("[CRDS] get_highest_slot() = {:?} (votes={}, total={})", result, vote_count, total);
        result
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contact_info::ContactInfo;
    use crate::crds_data::{CrdsData, NodeInstance, RestartHeaviestFork};
    use crate::legacy_contact_info::LegacyContactInfo;
    use solana_sdk::pubkey::Pubkey;
    use std::net::SocketAddr;

    fn make_contact_value(pubkey: Pubkey, wallclock: u64, gossip_port: u16) -> CrdsValue {
        let addr: SocketAddr = format!("127.0.0.1:{gossip_port}").parse().unwrap();
        let ci = ContactInfo::new(pubkey, wallclock, addr, 7016);
        CrdsValue::unsigned_new_data(CrdsData::ContactInfo(ci))
    }

    fn make_legacy_contact_value(pubkey: Pubkey, wallclock: u64, gossip_port: u16) -> CrdsValue {
        let gossip: SocketAddr = format!("127.0.0.1:{gossip_port}").parse().unwrap();
        let lci = LegacyContactInfo {
            id: pubkey,
            gossip,
            tvu: "0.0.0.0:0".parse().unwrap(),
            tvu_quic: "0.0.0.0:0".parse().unwrap(),
            serve_repair_quic: "0.0.0.0:0".parse().unwrap(),
            tpu: "0.0.0.0:0".parse().unwrap(),
            tpu_forwards: "0.0.0.0:0".parse().unwrap(),
            tpu_vote: "0.0.0.0:0".parse().unwrap(),
            rpc: "0.0.0.0:0".parse().unwrap(),
            rpc_pubsub: "0.0.0.0:0".parse().unwrap(),
            wallclock,
            shred_version: 7016,
        };
        CrdsValue::unsigned_new_data(CrdsData::LegacyContactInfo(lci))
    }

    fn make_node_instance(from: Pubkey, wallclock: u64) -> CrdsValue {
        CrdsValue::unsigned_new_data(CrdsData::NodeInstance(NodeInstance {
            pubkey: from,
            wallclock,
            timestamp: 0,
            token: 0,
        }))
    }

    fn make_restart_fork(from: Pubkey, wallclock: u64, last_slot: u64) -> CrdsValue {
        CrdsValue::unsigned_new_data(CrdsData::RestartHeaviestFork(RestartHeaviestFork {
            from,
            wallclock,
            last_slot,
            last_slot_hash: solana_sdk::hash::Hash::default(),
            observed_stake: 0,
            shred_version: 7016,
        }))
    }

    #[test]
    fn merge_new_inserts() {
        let mut table = CrdsTable::new();
        let pk = Pubkey::new_unique();
        let val = make_contact_value(pk, 1000, 8001);
        assert!(table.merge(val));
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn merge_older_skipped() {
        let mut table = CrdsTable::new();
        let pk = Pubkey::new_unique();
        let newer = make_contact_value(pk, 2000, 8001);
        let older = make_contact_value(pk, 1000, 8001);
        assert!(table.merge(newer));
        assert!(!table.merge(older));
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn merge_newer_replaces() {
        let mut table = CrdsTable::new();
        let pk = Pubkey::new_unique();
        let older = make_contact_value(pk, 1000, 8001);
        let newer = make_contact_value(pk, 2000, 8002);
        assert!(table.merge(older));
        assert!(table.merge(newer));
        assert_eq!(table.len(), 1);
        let infos = table.get_contact_infos();
        assert_eq!(infos[0].1.port(), 8002);
    }

    #[test]
    fn merge_same_wallclock_skips() {
        let mut table = CrdsTable::new();
        let pk = Pubkey::new_unique();
        let first = make_contact_value(pk, 1000, 8001);
        let second = make_contact_value(pk, 1000, 8002);
        assert!(table.merge(first));
        assert!(!table.merge(second));
        let infos = table.get_contact_infos();
        assert_eq!(infos[0].1.port(), 8001); // first wins
    }

    #[test]
    fn merge_two_pubkeys() {
        let mut table = CrdsTable::new();
        let pk_a = Pubkey::new_unique();
        let pk_b = Pubkey::new_unique();
        assert!(table.merge(make_contact_value(pk_a, 1000, 8001)));
        assert!(table.merge(make_contact_value(pk_b, 1000, 8002)));
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn get_contact_infos_modern() {
        let mut table = CrdsTable::new();
        let pk = Pubkey::new_unique();
        table.merge(make_contact_value(pk, 1000, 8001));
        let infos = table.get_contact_infos();
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].0, pk);
        assert_eq!(infos[0].1.port(), 8001);
    }

    #[test]
    fn get_contact_infos_legacy() {
        let mut table = CrdsTable::new();
        let pk = Pubkey::new_unique();
        table.merge(make_legacy_contact_value(pk, 1000, 8001));
        let infos = table.get_contact_infos();
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].0, pk);
    }

    #[test]
    fn get_contact_infos_skips_non_contact() {
        let mut table = CrdsTable::new();
        let pk = Pubkey::new_unique();
        table.merge(make_node_instance(pk, 1000));
        let infos = table.get_contact_infos();
        assert!(infos.is_empty());
    }

    #[test]
    fn all_contact_infos_modern_only() {
        let mut table = CrdsTable::new();
        let pk_m = Pubkey::new_unique();
        let pk_l = Pubkey::new_unique();
        table.merge(make_contact_value(pk_m, 1000, 8001));
        table.merge(make_legacy_contact_value(pk_l, 1000, 8002));
        let all = table.all_contact_infos();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, pk_m);
    }

    #[test]
    fn prune_removes_stale() {
        let mut table = CrdsTable::new();
        let pk = Pubkey::new_unique();
        let stale_wallclock = now() - 20 * 60; // 20 min ago
        table.merge(make_contact_value(pk, stale_wallclock, 8001));
        table.prune();
        assert_eq!(table.len(), 0);
    }

    #[test]
    fn prune_keeps_fresh() {
        let mut table = CrdsTable::new();
        let pk = Pubkey::new_unique();
        let fresh_wallclock = now() - 60; // 1 min ago
        table.merge(make_contact_value(pk, fresh_wallclock, 8001));
        table.prune();
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn prune_mixed() {
        let mut table = CrdsTable::new();
        let pk_stale = Pubkey::new_unique();
        let pk_fresh = Pubkey::new_unique();
        table.merge(make_contact_value(pk_stale, now() - 20 * 60, 8001));
        table.merge(make_contact_value(pk_fresh, now() - 60, 8002));
        table.prune();
        assert_eq!(table.len(), 1);
        let infos = table.get_contact_infos();
        assert_eq!(infos[0].0, pk_fresh);
    }

    #[test]
    fn get_highest_slot_from_restart_fork() {
        let mut table = CrdsTable::new();
        table.merge(make_restart_fork(Pubkey::new_unique(), 1000, 42));
        table.merge(make_restart_fork(Pubkey::new_unique(), 1000, 99));
        let slot = table.get_highest_slot();
        assert_eq!(slot, Some(99));
    }

    #[test]
    fn get_highest_slot_none() {
        let mut table = CrdsTable::new();
        let pk = Pubkey::new_unique();
        table.merge(make_contact_value(pk, 1000, 8001));
        let slot = table.get_highest_slot();
        assert_eq!(slot, None);
    }

    #[test]
    fn drain_events_has_entries() {
        let mut table = CrdsTable::new();
        let pk_m = Pubkey::new_unique();
        let pk_l = Pubkey::new_unique();
        table.merge(make_contact_value(pk_m, 1000, 8001));
        table.merge(make_legacy_contact_value(pk_l, 1000, 8002));
        let events = table.drain_events();
        assert_eq!(events.len(), 2);
        for event in &events {
            match event {
                GossipEvent::NewValidators(info) => {
                    assert!(!info.id.is_empty());
                }
                _ => panic!("expected NewValidators, got {event:?}"),
            }
        }
    }
}
