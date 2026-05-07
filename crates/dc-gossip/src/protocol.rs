use crate::contact_info::ContactInfo;
use crate::crds_data::CrdsValue;
use crate::crds_filter::CrdsFilter;
use crate::ping_pong::{Ping, Pong};
use anyhow::Result;
use bitvec::prelude::*;
use serde::{Deserialize, Serialize};
use solana_bloom::bloom::{Bloom, ConcurrentBloom};
use solana_sdk::{
    hash::Hash,
    pubkey::Pubkey,
    signature::Signature,
    signer::{keypair::Keypair, Signer},
};

const MASK_BITS: u32 = 7427;

/**
* #[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CrdsFilter {
    pub filter: Bloom<Hash>,
    mask: u64,
    mask_bits: u32,
}

impl CrdsFilter {
    pub fn mask_bits(&self) -> u32 {
        self.mask_bits
    }
}

fn compute_mask(seed: u64, mask_bits: u32) -> u64 {
    if mask_bits == 0 {
        return u64::MAX;
    }

    assert!(mask_bits <= 64);
    assert!(u128::from(seed) < (1u128 << mask_bits));

    let prefix = if mask_bits == 64 {
        seed
    } else {
        seed << (64 - mask_bits)
    };
    let suffix = if mask_bits == 64 {
        0
    } else {
        u64::MAX >> mask_bits
    };

    prefix | suffix
}

fn compute_mask_bits(num_items: u32, max_items: u32) -> u32 {
    if num_items <= max_items {
        return 0;
    }

    let ratio = num_items as f64 / max_items as f64;
    ratio.log2().ceil() as u32
}

impl Default for CrdsFilter {
    fn default() -> Self {
        let max_items: u32 = 1287;
        let num_items: u32 = 0;
        let false_rate: f64 = 0.1;
        let mask_bits = compute_mask_bits(num_items, max_items);

        Self {
            filter: Bloom::random(num_items as usize, false_rate, MASK_BITS as usize),
            mask: compute_mask(0, mask_bits),
            mask_bits,
        }
    }
}

*#[derive(Serialize, Deserialize, Clone, Debug)]
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
}
*
*
*
*/

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum CrdsData {
    LegacyContactInfo(Vec<u8>),         // 0 - deprecated, just bytes
    Vote(u8, Vec<u8>),                  // 1
    LowestSlot(u8, Vec<u8>),            // 2
    LegacySnapshotHashes(Vec<u8>),      // 3
    AccountsHashes(Vec<u8>),            // 4
    EpochSlots(u8, Vec<u8>),            // 5
    LegacyVersion(Vec<u8>),             // 6
    Version(Vec<u8>),                   // 7
    NodeInstance(Vec<u8>),              // 8
    DuplicateShred(u16, Vec<u8>),       // 9
    SnapshotHashes(Vec<u8>),            // 10
    ContactInfo(ContactInfo),           // 11 ← our real one
    RestartLastVotedForkSlots(Vec<u8>), // 12
    RestartHeaviestFork(Vec<u8>),       // 13
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequest {
    pub from: Pubkey,
    pub caller: CrdsValue,
    pub known: Vec<String>,
}

//#[derive(Debug, Clone, Serialize, Deserialize)]
/**pub struct PullResponse {
    pub from: Pubkey,
    pub values: Vec<CrdsValue>,
}**/

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum Protocol {
    PullRequest(CrdsFilter, CrdsValue),
    PullResponse(Pubkey, Vec<CrdsValue>),
    PushMessage(Pubkey, Vec<CrdsValue>),
    PingMessage(Ping),
    PongMessage(Pong),
    Unknown,
}

impl Protocol {
    pub fn encode_to(&self) -> Result<Vec<u8>> {
        Ok(bincode::serialize(self)?)
    }

    pub fn decode_from(bytes: &[u8]) -> Result<Self> {
        Ok(bincode::deserialize(bytes)?)
    }
}
