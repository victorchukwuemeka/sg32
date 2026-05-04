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
    serde::{Deserialize, Serialize},
    std::{borrow::Cow, collections::BTreeSet},
};

type IndexVote = u8;

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
pub enum CompressionType {}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum CrdsData {
    LegacyContactInfo(LegacyContactInfo), // 0 - deprecated, just bytes
    Vote(IndexVote, Vote),                // 1
    LowestSlot(u8, Vec<u8>),              // 2
    LegacySnapshotHashes(Vec<u8>),        // 3
    AccountsHashes(Vec<u8>),              // 4
    EpochSlots(u8, Vec<u8>),              // 5
    LegacyVersion(Vec<u8>),               // 6
    Version(Vec<u8>),                     // 7
    NodeInstance(Vec<u8>),                // 8
    DuplicateShred(u16, Vec<u8>),         // 9
    SnapshotHashes(Vec<u8>),              // 10
    ContactInfo(ContactInfo),             // 11 ← our real one
    RestartLastVotedForkSlots(Vec<u8>),   // 12
    RestartHeaviestFork(Vec<u8>),         // 13
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CrdsValue {
    pub signature: Signature,
    pub data: CrdsData,
    #[serde(skip_serializing)]
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

    fn unsigned_new_data(data: CrdsData) -> Self {
        Self {
            signature: Signature::default(),
            data,
            hash: Hash::default(),
        }
    }

    fn signed_new_data(data: CrdsData, keypair: &Keypair) -> Self {
        let mut value = Self::unsigned_new_data(data);
        value.sign(keypair);
        value
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
