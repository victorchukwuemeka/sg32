
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contact_info::ContactInfo;
    use crate::crds_data::CrdsData;
    use solana_sdk::pubkey::Pubkey;
    use solana_sdk::signer::keypair::Keypair;
    use std::net::SocketAddr;

    fn test_pubkey() -> Pubkey {
        Pubkey::new_from_array([8u8; 32])
    }

    fn test_contact_value(wallclock: u64, port: u16) -> CrdsValue {
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let ci = ContactInfo::new(Pubkey::new_unique(), wallclock, addr, 7016);
        CrdsValue::unsigned_new_data(CrdsData::ContactInfo(ci))
    }

    fn test_crds_filter() -> CrdsFilter {
        CrdsFilter::new(512, 0)
    }

    #[test]
    fn roundtrip_pull_request() {
        let msg = Protocol::PullRequest(test_crds_filter(), test_contact_value(1000, 8001));
        let bytes = msg.encode_to().unwrap();
        let decoded = Protocol::decode_from(&bytes).unwrap();
        assert!(matches!(decoded, Protocol::PullRequest(_, _)));
    }

    #[test]
    fn roundtrip_pull_response() {
        let msg = Protocol::PullResponse(
            test_pubkey(),
            vec![test_contact_value(1000, 8001), test_contact_value(2000, 8002)],
        );
        let bytes = msg.encode_to().unwrap();
        let decoded = Protocol::decode_from(&bytes).unwrap();
        match decoded {
            Protocol::PullResponse(pk, vals) => {
                assert_eq!(pk, test_pubkey());
                assert_eq!(vals.len(), 2);
            }
            _ => panic!("expected PullResponse"),
        }
    }

    #[test]
    fn roundtrip_push_message() {
        let msg = Protocol::PushMessage(
            test_pubkey(),
            vec![test_contact_value(1000, 8001)],
        );
        let bytes = msg.encode_to().unwrap();
        let decoded = Protocol::decode_from(&bytes).unwrap();
        match decoded {
            Protocol::PushMessage(pk, vals) => {
                assert_eq!(pk, test_pubkey());
                assert_eq!(vals.len(), 1);
            }
            _ => panic!("expected PushMessage"),
        }
    }

    #[test]
    fn roundtrip_ping_message() {
        let keypair = Keypair::new();
        let ping = Ping::new(&keypair).unwrap();
        let msg = Protocol::PingMessage(ping);
        let bytes = msg.encode_to().unwrap();
        let decoded = Protocol::decode_from(&bytes).unwrap();
        assert!(matches!(decoded, Protocol::PingMessage(_)));
    }

    #[test]
    fn roundtrip_pong_message() {
        let keypair = Keypair::new();
        let ping = Ping::new(&keypair).unwrap();
        let pong = Pong::new(&ping, &keypair).unwrap();
        let msg = Protocol::PongMessage(pong);
        let bytes = msg.encode_to().unwrap();
        let decoded = Protocol::decode_from(&bytes).unwrap();
        assert!(matches!(decoded, Protocol::PongMessage(_)));
    }

    #[test]
    fn roundtrip_prune_message() {
        let pk = Pubkey::new_unique();
        let prune = PruneData {
            pubkey: pk,
            prunes: vec![Pubkey::new_unique()],
            signature: solana_sdk::signature::Signature::default(),
            destination: pk,
            wallclock: 1000,
        };
        let msg = Protocol::PruneMessage(pk, prune);
        let bytes = msg.encode_to().unwrap();
        let decoded = Protocol::decode_from(&bytes).unwrap();
        assert!(matches!(decoded, Protocol::PruneMessage(_, _)));
    }

    #[test]
    fn decode_unknown_tag() {
        let bytes = vec![0xff, 0xff, 0xff, 0xff];
        let decoded = Protocol::decode_from(&bytes).unwrap();
        assert!(matches!(decoded, Protocol::Unknown));
    }

    #[test]
    fn decode_pull_response_empty() {
        let msg = Protocol::PullResponse(test_pubkey(), vec![]);
        let bytes = msg.encode_to().unwrap();
        let decoded = Protocol::decode_from(&bytes).unwrap();
        match decoded {
            Protocol::PullResponse(_, vals) => assert!(vals.is_empty()),
            _ => panic!("expected PullResponse"),
        }
    }

    #[test]
    fn decode_push_message_empty() {
        let msg = Protocol::PushMessage(test_pubkey(), vec![]);
        let bytes = msg.encode_to().unwrap();
        let decoded = Protocol::decode_from(&bytes).unwrap();
        match decoded {
            Protocol::PushMessage(_, vals) => assert!(vals.is_empty()),
            _ => panic!("expected PushMessage"),
        }
    }
}

