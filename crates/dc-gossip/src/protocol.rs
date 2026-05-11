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
pub struct PruneData {
    pub pubkey: Pubkey,
    pub prunes: Vec<Pubkey>,
    pub signature: Signature,
    pub destination: Pubkey,
    pub wallclock: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequest {
    pub from: Pubkey,
    pub caller: CrdsValue,
    pub known: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullResponse {
    pub from: Pubkey,
    pub values: Vec<CrdsValue>,
}


#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum Protocol {
    PullRequest(CrdsFilter, CrdsValue),
    PullResponse(Pubkey, Vec<CrdsValue>),
    PushMessage(Pubkey, Vec<CrdsValue>),
    PruneMessage(Pubkey, PruneData),
    PingMessage(Ping),
    PongMessage(Pong),
    Unknown,
}

impl Protocol {
    pub fn encode_to(&self) -> Result<Vec<u8>> {
        Ok(bincode::serialize(self)?)
    }

    pub fn decode_from(bytes: &[u8]) -> Result<Self> {
        // First try the fast path — full bincode deserialize
        if let Ok(msg) = bincode::deserialize(bytes) {
            return Ok(msg);
        }

        // Slow path: manually parse, skipping bad CrdsValues in Vec variants
        let mut cursor = std::io::Cursor::new(bytes);

        // Read Protocol discriminant
        let tag: u32 = bincode::deserialize_from(&mut cursor)?;

        match tag {
            0 => {
                // PullRequest(CrdsFilter, CrdsValue) — no fallback, both are required
                cursor.set_position(0);
                Ok(bincode::deserialize(bytes)?)
            }
            1 | 2 => {
                // PullResponse(Pubkey, Vec<CrdsValue>) or PushMessage(Pubkey, Vec<CrdsValue>)
                let from: Pubkey = bincode::deserialize_from(&mut cursor)?;
                let count: u64 = bincode::deserialize_from(&mut cursor)?;

                let mut values = Vec::new();
                for _ in 0..count {
                    let start = cursor.position() as usize;
                    // Try to deserialize one CrdsValue
                    let remaining = &bytes[start..];
                    match bincode::deserialize::<CrdsValue>(remaining) {
                        Ok(val) => {
                            // Advance cursor past this value
                            let consumed = bincode::serialized_size(&val).unwrap_or(0) as usize;
                            // If we can't determine the size, try to find it by scanning
                            cursor.set_position((start + consumed) as u64);
                            values.push(val);
                        }
                        Err(_) => {
                            // Skip past this CrdsValue by scanning for next valid start
                            // Strategy: advance byte-by-byte trying to parse a Signature (64 bytes + CrdsData)
                            // Simpler: just skip 64 bytes (signature) + try to parse CrdsData
                            if let Ok(sig) = bincode::deserialize::<Signature>(remaining) {
                                let sig_size = bincode::serialized_size(&sig).unwrap_or(64) as usize;
                                let after_sig = &bytes[start + sig_size..];
                                if let Ok(crds_data) = bincode::deserialize::<crate::crds_data::CrdsData>(after_sig) {
                                    let data_size = bincode::serialized_size(&crds_data).unwrap_or(0) as usize;
                                    cursor.set_position((start + sig_size + data_size) as u64);
                                } else {
                                    // Give up: advance by estimated minimum CrdsValue size (64 bytes)
                                    cursor.set_position((start + 64) as u64);
                                }
                            } else {
                                cursor.set_position((start + 64) as u64);
                            }
                            // If cursor hasn't advanced, break to avoid infinite loop
                            if cursor.position() as usize == start {
                                cursor.set_position(bytes.len() as u64);
                                break;
                            }
                        }
                    }
                }

                if tag == 1 {
                    Ok(Protocol::PullResponse(from, values))
                } else {
                    Ok(Protocol::PushMessage(from, values))
                }
            }
            3 => {
                cursor.set_position(0);
                Ok(bincode::deserialize(bytes)?)
            }
            4 => {
                cursor.set_position(0);
                Ok(bincode::deserialize(bytes)?)
            }
            5 => {
                cursor.set_position(0);
                Ok(bincode::deserialize(bytes)?)
            }
            _ => Ok(Protocol::Unknown),
        }
    }
}

