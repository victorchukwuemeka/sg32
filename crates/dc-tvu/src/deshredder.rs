use crate::shred::Shred;
use crate::shred_header::*;

pub fn deshred(shreds: &[Shred]) -> Option<Vec<Vec<u8>>> {
    let mut data_shreds: Vec<&Shred> = shreds
        .iter()
        .filter(|s| s.shred_type() == ShredType::Data)
        .collect();
    data_shreds.sort_by_key(|s| s.index());
    let mut all_data = Vec::new();
    for shred in &data_shreds {
        let payload = match shred {
            Shred::MerkleData { data, .. } => data,
            _ => continue,
        };
        all_data.extend_from_slice(payload);
    }
    while all_data.last() == Some(&0) {
        all_data.pop();
    }

    Some(vec![all_data])
}
