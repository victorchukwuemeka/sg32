use bincode;
use solana_sdk::hash::Hash;
use solana_transaction::versioned::VersionedTransaction;

pub struct DeshredResult {
    pub entries: Vec<Entry>,
    pub transactions: Vec<Vec<u8>>,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct Entry {
    pub num_hashes: u64,
    pub hash: Hash,
    pub transactions: Vec<VersionedTransaction>,
}

pub fn deshred_into_txs(data_payloads: &[Vec<u8>]) -> Option<DeshredResult> {
    let mut all_data = Vec::new();
    for payload in data_payloads {
        all_data.extend_from_slice(payload);
    }

    while all_data.last() == Some(&0) {
        all_data.pop();
    }

    let entries: Vec<Entry> = bincode::deserialize(&all_data).ok()?;

    let transactions: Vec<Vec<u8>> = entries
        .iter()
        .flat_map(|entry| &entry.transactions)
        .map(|tx| bincode::serialize(tx).unwrap_or_default())
        .collect();

    Some(DeshredResult {
        entries,
        transactions,
    })
}

pub fn deshred_into_txs_from_shreds(shred_data: &[Vec<u8>]) -> Option<DeshredResult> {
    deshred_into_txs(shred_data)
}
