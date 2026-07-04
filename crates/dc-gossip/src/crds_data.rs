use crate::contact_info::ContactInfo;
use crate::legacy_contact_info::LegacyContactInfo;
use solana_sdk::{
    clock::Slot,
    hash::Hash,
    pubkey::Pubkey,
    signature::{Signable, Signature},
    signer::{keypair::Keypair, Signer},
    transaction::Transaction,
};
use ::{
    bincode::serialize,
    bv::BitVec,
    serde::{Deserialize, Serialize},
    std::{borrow::Borrow, borrow::Cow, collections::BTreeSet},
};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum CrdsData {
    LegacyContactInfo(LegacyContactInfo), // 0 - deprecated, just bytes
    Vote(VoteIndex, Vote),                // 1
    LowestSlot(u8, LowestSlot),           // 2
    LegacySnapshotHashes(LegacySnapshotHashes), // 3
    AccountsHashes(AccountsHashes),       // 4
    EpochSlots(EpochSlotsIndex, EpochSlots), // 5
    LegacyVersion(LegacyVersion),         // 6
    Version(Version),                     // 7
    NodeInstance(NodeInstance),           // 8
    DuplicateShred(DuplicateShredIndex, DuplicateShred), // 9
    SnapshotHashes(SnapshotHashes),       // 10
    ContactInfo(ContactInfo),             // 11 ← our real one
    RestartLastVotedForkSlots(RestartLastVotedForkSlots), // 12
    RestartHeaviestFork(RestartHeaviestFork), // 13
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RestartHeaviestFork {
    pub from: Pubkey,
    pub wallclock: u64,
    pub last_slot: Slot,
    pub last_slot_hash: Hash,
    pub observed_stake: u64,
    pub shred_version: u16,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RestartLastVotedForkSlots {
    pub from: Pubkey,
    pub wallclock: u64,
    offsets: SlotsOffsets,
    pub last_voted_slot: Slot,
    pub last_voted_hash: Hash,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
enum SlotsOffsets {
    RunLengthEncoding(RunLengthEncoding),
    RawOffsets(RawOffsets),
}

#[derive(Deserialize, Serialize, Clone, Debug)]
struct RunLengthEncoding(Vec<u16>);

#[derive(Deserialize, Serialize, Clone, Debug)]
struct RawOffsets(BitVec<u8>);

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct SnapshotHashes {
    pub from: Pubkey,
    pub full: (Slot, Hash),
    pub incremental: Vec<(Slot, Hash)>,
    pub wallclock: u64,
}

pub type DuplicateShredIndex = u16;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct DuplicateShred {
    pub from: Pubkey,
    pub wallclock: u64,
    pub slot: Slot,
    _unused: u32,
    _unused_shred_type: ShredType,
    num_chunks: u8,
    chunk_index: u8,
    chunk: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
enum ShredType {
    Data = 0b1010_0101,
    Code = 0b0101_1010,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NodeInstance {
    pub pubkey: Pubkey,
    pub wallclock: u64,
    pub timestamp: u64,
    pub token: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Version {
    pub pubkey: Pubkey,
    pub wallclock: u64,
    pub version: solana_version::LegacyVersion2,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LegacyVersion {
    pub pubkey: Pubkey,
    pub wallclock: u64,
    pub version: solana_version::LegacyVersion1,
}

type EpochSlotsIndex = u8;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EpochSlots {
    pub from: Pubkey,
    pub slots: Vec<CompressedSlot>,
    pub wallclock: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum CompressedSlot {
    Flate2(Flate2),
    Uncompressed(Uncompressed),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Flate2 {
    pub first_slot: Slot,
    pub num: usize,
    pub compressed: Vec<u8>,
}

type LegacySnapshotHashes = AccountsHashes;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AccountsHashes {
    pub from: Pubkey,
    pub hashes: Vec<(Slot, Hash)>,
    pub wallclock: u64,
}

type VoteIndex = u8;

#[derive(Serialize, Clone, Debug)]
pub struct Vote {
    pub from: Pubkey,
    transaction: Transaction,
    pub wallclock: u64,
    #[serde(skip_serializing)]
    pub slot: Option<Slot>,
}

impl<'de> Deserialize<'de> for Vote {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use solana_vote_interface::instruction::VoteInstruction;

        #[derive(Deserialize)]
        struct VoteHelper {
            from: Pubkey,
            transaction: Transaction,
            pub wallclock: u64,
        }
        let helper = VoteHelper::deserialize(deserializer)?;

        // Parse the vote transaction to fill the cached slot field.
        let slot = extract_slot_from_vote_tx(&helper.transaction);

        Ok(Vote {
            from: helper.from,
            transaction: helper.transaction,
            wallclock: helper.wallclock,
            slot,
        })
    }
}

/// Parse a vote transaction and return the last voted slot.
fn extract_slot_from_vote_tx(tx: &Transaction) -> Option<Slot> {
    use solana_vote_interface::{instruction::VoteInstruction, program::check_id};

    let message = tx.message();
    let first_ix = message.instructions.first()?;
    let program_id = message
        .account_keys
        .get(usize::from(first_ix.program_id_index))?;

    if !check_id(program_id) {
        return None;
    }

    let ix_data = &first_ix.data;
    let vote_ix: VoteInstruction = bincode::deserialize(ix_data).ok()?;

    match vote_ix {
        VoteInstruction::Vote(v) | VoteInstruction::VoteSwitch(v, _) => v.last_voted_slot(),
        VoteInstruction::UpdateVoteState(s)
        | VoteInstruction::UpdateVoteStateSwitch(s, _)
        | VoteInstruction::CompactUpdateVoteState(s)
        | VoteInstruction::CompactUpdateVoteStateSwitch(s, _) => s.last_voted_slot(),
        VoteInstruction::TowerSync(s) | VoteInstruction::TowerSyncSwitch(s, _) => {
            s.last_voted_slot()
        }
        _ => None,
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LowestSlot {
    pub from: Pubkey,
    root: Slot,
    pub lowest: Slot,
    slots: BTreeSet<Slot>,
    stash: Vec<EpochIncompleteSlots>,
    pub wallclock: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EpochIncompleteSlots {
    first: Slot,
    compression: CompressionType,
    compressed_list: Vec<u8>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum CompressionType {
    Uncompressed,
    Gzip,
    Bzip2,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Uncompressed {
    pub first_slot: Slot,
    pub num: usize,
    pub slots: BitVec<u8>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CrdsValue {
    pub signature: Signature,
    pub data: CrdsData,
    #[serde(skip)]
    pub hash: Hash,
}

impl CrdsValue {
    pub fn new_contact_info(info: ContactInfo, keypair: &Keypair) -> Self {
        let data = CrdsData::ContactInfo(info);
        let bytes = bincode::serialize(&data).unwrap();
        let signature = keypair.sign_message(&bytes);
        let hash = solana_sdk::hash::hash(&bytes);
        Self {
            signature,
            data,
            hash,
        }
    }

    pub fn new_legacy_contact_info(info: LegacyContactInfo, keypair: &Keypair) -> Self {
        let data = CrdsData::LegacyContactInfo(info);
        let bytes = bincode::serialize(&data).unwrap();
        let signature = keypair.sign_message(&bytes);
        let hash = solana_sdk::hash::hash(&bytes);
        Self {
            signature,
            data,
            hash,
        }
    }

    pub fn pubkey(&self) -> Pubkey {
        match &self.data {
            CrdsData::LegacyContactInfo(contact_info) => *contact_info.pubkey(),
            CrdsData::Vote(_, vote) => vote.from,
            CrdsData::LowestSlot(_, slots) => slots.from,
            CrdsData::LegacySnapshotHashes(hash) => hash.from,
            CrdsData::AccountsHashes(hash) => hash.from,
            CrdsData::EpochSlots(_, p) => p.from,
            CrdsData::LegacyVersion(version) => version.pubkey,
            CrdsData::Version(version) => version.pubkey,
            CrdsData::NodeInstance(node) => node.pubkey,
            CrdsData::DuplicateShred(_, shred) => shred.from,
            CrdsData::SnapshotHashes(hash) => hash.from,
            CrdsData::ContactInfo(node) => *node.pubkey(),
            CrdsData::RestartLastVotedForkSlots(slots) => slots.from,
            CrdsData::RestartHeaviestFork(fork) => fork.from,
        }
    }

    pub fn unsigned_new_data(data: CrdsData) -> Self {
        Self {
            signature: Signature::default(),
            data,
            hash: Hash::default(),
        }
    }

    pub fn signed_new_data(data: CrdsData, keypair: &Keypair) -> Self {
        let mut value = Self::unsigned_new_data(data);
        value.sign(keypair);
        value
    }
}

impl CrdsValue {
    pub fn wallclock(&self) -> u64 {
        match &self.data {
            CrdsData::LegacyContactInfo(ci) => ci.wallclock,
            CrdsData::Vote(_, v) => v.wallclock,
            CrdsData::LowestSlot(_, s) => s.wallclock,
            CrdsData::LegacySnapshotHashes(h) => h.wallclock,
            CrdsData::AccountsHashes(h) => h.wallclock,
            CrdsData::EpochSlots(_, e) => e.wallclock,
            CrdsData::LegacyVersion(v) => v.wallclock,
            CrdsData::Version(v) => v.wallclock,
            CrdsData::NodeInstance(n) => n.wallclock,
            CrdsData::DuplicateShred(_, s) => s.wallclock,
            CrdsData::SnapshotHashes(h) => h.wallclock,
            CrdsData::ContactInfo(ci) => ci.wallclock,
            CrdsData::RestartLastVotedForkSlots(s) => s.wallclock,
            CrdsData::RestartHeaviestFork(f) => f.wallclock,
        }
    }
}

impl Signable for CrdsValue {
    fn pubkey(&self) -> Pubkey {
        self.pubkey()
    }

    fn signable_data(&self) -> Cow<[u8]> {
        Cow::Owned(serialize(&self.data).expect("failed to serialize CrdsData"))
    }

    fn get_signature(&self) -> Signature {
        self.signature
    }

    fn set_signature(&mut self, signature: Signature) {
        self.signature = signature
    }

    fn verify(&self) -> bool {
        self.get_signature()
            .verify(self.pubkey().as_ref(), self.signable_data().borrow())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contact_info::ContactInfo;
    use crate::legacy_contact_info::LegacyContactInfo;
    use bincode::{deserialize, serialize};
    use std::net::SocketAddr;

    fn pk() -> Pubkey {
        Pubkey::new_from_array([42u8; 32])
    }

    #[test]
    fn roundtrip_contact_info() {
        let ci = ContactInfo::new(pk(), 1000, "127.0.0.1:8001".parse().unwrap(), 7016);
        let data = CrdsData::ContactInfo(ci.clone());
        let bytes = serialize(&data).unwrap();
        let data2: CrdsData = deserialize(&bytes).unwrap();
        match data2 {
            CrdsData::ContactInfo(ci2) => {
                assert_eq!(*ci2.pubkey(), *ci.pubkey());
                assert_eq!(ci2.wallclock, ci.wallclock);
            }
            _ => panic!("expected ContactInfo"),
        }
    }

    #[test]
    fn roundtrip_legacy_contact_info() {
        let gossip: SocketAddr = "127.0.0.1:8001".parse().unwrap();
        let lci = LegacyContactInfo {
            id: pk(),
            gossip,
            tvu: "0.0.0.0:0".parse().unwrap(),
            tvu_quic: "0.0.0.0:0".parse().unwrap(),
            serve_repair_quic: "0.0.0.0:0".parse().unwrap(),
            tpu: "0.0.0.0:0".parse().unwrap(),
            tpu_forwards: "0.0.0.0:0".parse().unwrap(),
            tpu_vote: "0.0.0.0:0".parse().unwrap(),
            rpc: "0.0.0.0:0".parse().unwrap(),
            rpc_pubsub: "0.0.0.0:0".parse().unwrap(),
            wallclock: 1000,
            shred_version: 7016,
        };
        let data = CrdsData::LegacyContactInfo(lci);
        let bytes = serialize(&data).unwrap();
        let data2: CrdsData = deserialize(&bytes).unwrap();
        match data2 {
            CrdsData::LegacyContactInfo(lci2) => assert_eq!(lci2.id, pk()),
            _ => panic!("expected LegacyContactInfo"),
        }
    }

    #[test]
    fn roundtrip_node_instance() {
        let node = NodeInstance { pubkey: pk(), wallclock: 1000, timestamp: 500, token: 99 };
        let data = CrdsData::NodeInstance(node);
        let bytes = serialize(&data).unwrap();
        let data2: CrdsData = deserialize(&bytes).unwrap();
        match data2 {
            CrdsData::NodeInstance(n) => {
                assert_eq!(n.pubkey, pk());
                assert_eq!(n.wallclock, 1000);
                assert_eq!(n.token, 99);
            }
            _ => panic!("expected NodeInstance"),
        }
    }

    #[test]
    fn roundtrip_version() {
        let v = Version { pubkey: pk(), wallclock: 1000, version: solana_version::LegacyVersion2::default() };
        let data = CrdsData::Version(v);
        let bytes = serialize(&data).unwrap();
        let _: CrdsData = deserialize(&bytes).unwrap();
    }

    #[test]
    fn roundtrip_legacy_version() {
        // Use serde to construct a default LegacyVersion1 via deserialization
        let default_bytes = bincode::serialize(&solana_version::LegacyVersion2::default()).unwrap();
        let version: solana_version::LegacyVersion1 = bincode::deserialize(&default_bytes).unwrap();
        let lv = LegacyVersion { pubkey: pk(), wallclock: 1000, version };
        let data = CrdsData::LegacyVersion(lv);
        let bytes = serialize(&data).unwrap();
        let _: CrdsData = deserialize(&bytes).unwrap();
    }

    #[test]
    fn roundtrip_epoch_slots() {
        let slots = vec![
            CompressedSlot::Uncompressed(Uncompressed {
                first_slot: 10,
                num: 5,
                slots: BitVec::new(),
            }),
        ];
        let es = EpochSlots { from: pk(), slots, wallclock: 1000 };
        let data = CrdsData::EpochSlots(0, es);
        let bytes = serialize(&data).unwrap();
        let data2: CrdsData = deserialize(&bytes).unwrap();
        match data2 {
            CrdsData::EpochSlots(idx, _) => assert_eq!(idx, 0),
            _ => panic!("expected EpochSlots"),
        }
    }

    #[test]
    fn roundtrip_accounts_hashes() {
        let ah = AccountsHashes { from: pk(), hashes: vec![(100, Hash::default())], wallclock: 1000 };
        let data = CrdsData::AccountsHashes(ah);
        let bytes = serialize(&data).unwrap();
        let _: CrdsData = deserialize(&bytes).unwrap();
    }

    #[test]
    fn roundtrip_legacy_snapshot_hashes() {
        let lsh = AccountsHashes { from: pk(), hashes: vec![], wallclock: 1000 };
        let data = CrdsData::LegacySnapshotHashes(lsh);
        let bytes = serialize(&data).unwrap();
        let _: CrdsData = deserialize(&bytes).unwrap();
    }

    #[test]
    fn roundtrip_snapshot_hashes() {
        let sh = SnapshotHashes {
            from: pk(),
            full: (100, Hash::default()),
            incremental: vec![(50, Hash::default())],
            wallclock: 1000,
        };
        let data = CrdsData::SnapshotHashes(sh);
        let bytes = serialize(&data).unwrap();
        let _: CrdsData = deserialize(&bytes).unwrap();
    }

    #[test]
    fn roundtrip_restart_heaviest_fork() {
        let fork = RestartHeaviestFork {
            from: pk(),
            wallclock: 1000,
            last_slot: 42,
            last_slot_hash: Hash::default(),
            observed_stake: 100_000,
            shred_version: 7016,
        };
        let data = CrdsData::RestartHeaviestFork(fork);
        let bytes = serialize(&data).unwrap();
        let data2: CrdsData = deserialize(&bytes).unwrap();
        match data2 {
            CrdsData::RestartHeaviestFork(f) => {
                assert_eq!(f.from, pk());
                assert_eq!(f.last_slot, 42);
            }
            _ => panic!("expected RestartHeaviestFork"),
        }
    }

    #[test]
    fn crds_value_pubkey_contact_info() {
        let ci = ContactInfo::new(pk(), 1000, "127.0.0.1:8001".parse().unwrap(), 7016);
        let val = CrdsValue::unsigned_new_data(CrdsData::ContactInfo(ci));
        assert_eq!(val.pubkey(), pk());
    }

    #[test]
    fn crds_value_pubkey_legacy_contact() {
        let lci = LegacyContactInfo {
            id: pk(), gossip: "127.0.0.1:8001".parse().unwrap(),
            tvu: "0.0.0.0:0".parse().unwrap(), tvu_quic: "0.0.0.0:0".parse().unwrap(),
            serve_repair_quic: "0.0.0.0:0".parse().unwrap(), tpu: "0.0.0.0:0".parse().unwrap(),
            tpu_forwards: "0.0.0.0:0".parse().unwrap(), tpu_vote: "0.0.0.0:0".parse().unwrap(),
            rpc: "0.0.0.0:0".parse().unwrap(), rpc_pubsub: "0.0.0.0:0".parse().unwrap(),
            wallclock: 1000, shred_version: 7016,
        };
        let val = CrdsValue::unsigned_new_data(CrdsData::LegacyContactInfo(lci));
        assert_eq!(val.pubkey(), pk());
    }

    #[test]
    fn crds_value_wallclock() {
        let ci = ContactInfo::new(pk(), 7777, "127.0.0.1:8001".parse().unwrap(), 7016);
        let val = CrdsValue::unsigned_new_data(CrdsData::ContactInfo(ci));
        assert_eq!(val.wallclock(), 7777);
    }

    #[test]
    fn crds_value_wallclock_node_instance() {
        let node = NodeInstance { pubkey: pk(), wallclock: 8888, timestamp: 0, token: 0 };
        let val = CrdsValue::unsigned_new_data(CrdsData::NodeInstance(node));
        assert_eq!(val.wallclock(), 8888);
    }
}
