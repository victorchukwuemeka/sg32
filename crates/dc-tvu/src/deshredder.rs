use bincode;
use solana_sdk::hash::Hash;
use solana_sdk::transaction::VersionedTransaction;
use std::io::Cursor;

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

    if let Some(first) = data_payloads.first() {
        let hex_first_payload = first.iter().take(100).map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" ");
        eprintln!("[DESHRED] payload 0 (len={}) first 100 bytes: [{}]", first.len(), hex_first_payload);
    }
    let hex_first = all_data.iter().take(64).map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" ");
    eprintln!("[DESHRED] {} payloads, total={} bytes, first_bytes=[{}]",
        data_payloads.len(), all_data.len(), hex_first);

    let mut reader = Cursor::new(&all_data);
    let mut entries = Vec::new();
    loop {
        let pos = reader.position() as usize;
        if pos >= all_data.len() {
            break;
        }
        match bincode::deserialize_from::<_, Entry>(&mut reader) {
            Ok(entry) => {
                if entry.num_hashes == 0 && entry.hash == Hash::default() && entry.transactions.is_empty() {
                    break;
                }
                entries.push(entry);
            }
            Err(_) => break,
        }
    }

    if entries.is_empty() {
        return None;
    }

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
