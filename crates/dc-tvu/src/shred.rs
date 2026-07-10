use crate::shred_header::*;

#[derive(Debug, Clone)]
pub enum Shred {
    MerkleData {
        common_header: ShredCommonHeader,
        data_header: DataShredHeader,
        merkle_root: [u8; 32],
        merkle_proof: Vec<[u8; 20]>,
        data: Vec<u8>,
    },
    MerkleCode {
        common_header: ShredCommonHeader,
        coding_header: CodingShredHeader,
        merkle_root: [u8; 32],
        merkle_proof: Vec<[u8; 20]>,
        code: Vec<u8>,
    },
}

impl Shred {
    pub fn parse_from_bytes(bytes: &[u8]) -> Option<Self> {
        let variant = get_shred_variant(bytes)?;
        let (proof_size, resigned) = match variant {
            ShredVariant::MerkleData {
                proof_size,
                resigned,
            } => (proof_size as usize, resigned),
            ShredVariant::MerkleCode {
                proof_size,
                resigned,
            } => (proof_size as usize, resigned),
        };
        let common_header = ShredCommonHeader::from_bytes(bytes)?;
        match variant {
            ShredVariant::MerkleData { .. } => {
                let data_header: DataShredHeader = bincode::deserialize(
                    bytes
                        .get(SIZE_OF_COMMON_HEADER..SIZE_OF_COMMON_HEADER + SIZE_OF_DATA_HEADER)?,
                )
                .ok()?;
                let data_off = SIZE_OF_COMMON_HEADER + SIZE_OF_DATA_HEADER; // 88
                let data_end = data_header.size as usize;
                let data = if data_end > data_off {
                    bytes.get(data_off..data_end)?.to_vec()
                } else {
                    Vec::new()
                };
                let after_data = data_end + if resigned { SIZE_OF_SIGNATURE } else { 0 };
                let merkle_root = read_arr::<32>(bytes, after_data)?;
                let proof_off = after_data + 32;
                let mut merkle_proof = Vec::with_capacity(proof_size);
                for i in 0..proof_size {
                    let entry = read_arr::<20>(bytes, proof_off + i * 20)?;
                    merkle_proof.push(entry);
                }
                Some(Shred::MerkleData {
                    common_header,
                    data_header,
                    merkle_root,
                    merkle_proof,
                    data,
                })
            }
            ShredVariant::MerkleCode { .. } => {
                let coding_header: CodingShredHeader =
                    bincode::deserialize(bytes.get(
                        SIZE_OF_COMMON_HEADER..SIZE_OF_COMMON_HEADER + SIZE_OF_CODING_HEADER,
                    )?)
                    .ok()?;
                let code_off = SIZE_OF_COMMON_HEADER + SIZE_OF_CODING_HEADER; // 89
                let code_len = SIZE_OF_CODING_SHRED
                    - code_off
                    - SIZE_OF_MERKLE_ROOT
                    - (proof_size * SIZE_OF_MERKLE_PROOF_ENTRY)
                    - if resigned { SIZE_OF_SIGNATURE } else { 0 };
                let code = bytes.get(code_off..code_off + code_len)?.to_vec();
                let after_code = code_off + code_len;
                let merkle_off = after_code + if resigned { SIZE_OF_SIGNATURE } else { 0 };
                let merkle_root = read_arr::<32>(bytes, merkle_off)?;
                let proof_off = merkle_off + 32;
                let mut merkle_proof = Vec::with_capacity(proof_size);
                for i in 0..proof_size {
                    let entry = read_arr::<20>(bytes, proof_off + i * 20)?;
                    merkle_proof.push(entry);
                }
                Some(Shred::MerkleCode {
                    common_header,
                    coding_header,
                    merkle_root,
                    merkle_proof,
                    code,
                })
            }
        }
    }

    pub fn slot(&self) -> u64 {
        match self {
            Shred::MerkleData { common_header, .. } | Shred::MerkleCode { common_header, .. } => {
                common_header.slot
            }
        }
    }

    pub fn index(&self) -> u32 {
        match self {
            Shred::MerkleData { common_header, .. } | Shred::MerkleCode { common_header, .. } => {
                common_header.index
            }
        }
    }

    pub fn shred_type(&self) -> ShredType {
        match self {
            Shred::MerkleData { .. } => ShredType::Data,
            Shred::MerkleCode { .. } => ShredType::Code,
        }
    }

    pub fn erasure_set_id(&self) -> ErasureSetId {
        match self {
            Shred::MerkleData { common_header, .. } | Shred::MerkleCode { common_header, .. } => {
                ErasureSetId {
                    slot: common_header.slot,
                    fec_set_index: common_header.fec_set_index,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_data_shred_correct_layout() {
        // Build a 1203-byte non-resigned data shred (proof_size=6)
        let mut buf = vec![0u8; SIZE_OF_DATA_SHRED];

        // ── Common header (bytes 0..83) ──
        // [0..64]  signature — not verified, fill with 0xAA
        buf[0..64].fill(0xAA);
        // [64]     variant = 0x96 (Data 0x90 | proof_size 6)
        buf[64] = 0x96;
        // [65..73] slot = 42 (u64 LE)
        buf[65..73].copy_from_slice(&42u64.to_le_bytes());
        // [73..77] index = 0 (u32 LE)
        buf[73..77].copy_from_slice(&0u32.to_le_bytes());
        // [77..79] version = 7016 (u16 LE)
        buf[77..79].copy_from_slice(&7016u16.to_le_bytes());
        // [79..83] fec_set_index = 0 (u32 LE)
        buf[79..83].copy_from_slice(&0u32.to_le_bytes());

        // ── Data header (bytes 83..88) ──
        // [83..85] parent_offset = 1 (u16 LE)
        buf[83..85].copy_from_slice(&1u16.to_le_bytes());
        // [85]     flags = 0
        buf[85] = 0;
        // [86..88] size = 1051 (88 + 963 capacity)
        buf[86..88].copy_from_slice(&1051u16.to_le_bytes());

        // ── Data buffer (bytes 88..1051) ──
        // Fill with 0x42 so we can verify exact extraction
        buf[88..1051].fill(0x42);

        // ── Chained Merkle root (bytes 1051..1083) ──
        buf[1051..1083].fill(0xBB);

        // ── Merkle proof (bytes 1083..1203) — 6 entries of 20 bytes each ──
        buf[1083..1203].fill(0xCC);

        // Now parse
        let shred = Shred::parse_from_bytes(&buf).expect("should parse");

        match shred {
            Shred::MerkleData {
                data,
                merkle_root,
                merkle_proof,
                common_header,
                data_header,
            } => {
                assert_eq!(common_header.slot, 42);
                assert_eq!(common_header.index, 0);
                assert_eq!(data_header.size, 1051);

                // Verify data — should be exactly bytes 88..1051 (all 0x42)
                assert_eq!(data.len(), 963);
                assert!(data.iter().all(|&b| b == 0x42));

                // Verify merkle root — should be bytes 1051..1083 (all 0xBB)
                assert_eq!(merkle_root.len(), 32);
                assert!(merkle_root.iter().all(|&b| b == 0xBB));

                // Verify merkle proof — 6 entries of 20 bytes (all 0xCC)
                assert_eq!(merkle_proof.len(), 6);
                for entry in &merkle_proof {
                    assert_eq!(entry.len(), 20);
                    assert!(entry.iter().all(|&b| b == 0xCC));
                }
            }
            _ => panic!("expected MerkleData variant"),
        }
    }
}
