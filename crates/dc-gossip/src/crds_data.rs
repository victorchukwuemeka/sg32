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

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Vote {
    pub from: Pubkey,
    transaction: Transaction,
    pub wallclock: u64,
    slot: Option<Slot>,
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
