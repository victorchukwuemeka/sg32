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
        let proof_size = match variant {
            ShredVariant::MerkleData { proof_size, .. } => proof_size as usize,
            ShredVariant::MerkleCode { proof_size, .. } => proof_size as usize,
        };
        let common_header = ShredCommonHeader::from_bytes(bytes)?;
        match variant {
            ShredVariant::MerkleData { .. } => {
                let data_header: DataShredHeader = bincode::deserialize(
                    bytes
                        .get(SIZE_OF_COMMON_HEADER..SIZE_OF_COMMON_HEADER + SIZE_OF_DATA_HEADER)?,
                )
                .ok()?;
                let merkle_off = SIZE_OF_COMMON_HEADER + SIZE_OF_DATA_HEADER;
                let merkle_root = read_arr::<32>(bytes, merkle_off)?;
                let proof_off = merkle_off + 32;
                let mut merkle_proof = Vec::with_capacity(proof_size);
                for i in 0..proof_size {
                    let entry = read_arr::<20>(bytes, proof_off + i * 20)?;
                    merkle_proof.push(entry);
                }
                let data_off = proof_off + proof_size * 20;
                let data = bytes.get(data_off..)?.to_vec();
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
                let merkle_off = SIZE_OF_COMMON_HEADER + SIZE_OF_CODING_HEADER;
                let merkle_root = read_arr::<32>(bytes, merkle_off)?;
                let proof_off = merkle_off + 32;
                let mut merkle_proof = Vec::with_capacity(proof_size);
                for i in 0..proof_size {
                    let entry = read_arr::<20>(bytes, proof_off + i * 20)?;
                    merkle_proof.push(entry);
                }
                let code_off = proof_off + proof_size * 20;
                let code = bytes.get(code_off..)?.to_vec();
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
            Shred::MerkleData { common_header, .. }
            | Shred::MerkleCode { common_header, .. } => common_header.slot,
        }
    }

    pub fn index(&self) -> u32 {
        match self {
            Shred::MerkleData { common_header, .. }
            | Shred::MerkleCode { common_header, .. } => common_header.index,
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
            Shred::MerkleData { common_header, .. }
            | Shred::MerkleCode { common_header, .. } => ErasureSetId {
                slot: common_header.slot,
                fec_set_index: common_header.fec_set_index,
            },
        }
    }

}
