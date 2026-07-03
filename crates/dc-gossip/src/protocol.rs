
use crate::crds_data::CrdsValue;
use crate::crds_filter::CrdsFilter;
use crate::ping_pong::{Ping, Pong};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use solana_sdk::{pubkey::Pubkey, signature::Signature};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PruneData {
    pub pubkey: Pubkey,
    pub prunes: Vec<Pubkey>,
    pub signature: Signature,
    pub destination: Pubkey,
    pub wallclock: u64,
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

