use solana_sdk::hash::Hash;

pub struct DeshredResult {
    pub entries: Vec<Entry>,
    pub transactions: Vec<Vec<u8>>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Entry {
    pub num_hashes: u64,
    pub hash: Hash,
    pub transactions: Vec<Vec<u8>>,
}

// ── Solana `short_vec` compact-u16 ───────────────────────────────────────────
// Lengths inside a transaction are encoded as a variable-length "compact-u16"
// (1-2 bytes, continuation bit 0x80), NOT as bincode's fixed u64.
fn read_shortu16(data: &[u8], pos: &mut usize) -> Option<u16> {
    let mut value: u16 = 0;
    let mut shift: u32 = 0;
    loop {
        let byte = *data.get(*pos)?;
        *pos += 1;
        value |= u16::from(byte & 0x7f).checked_shl(shift)?;
        if byte & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
        if shift > 14 {
            return None;
        }
    }
}

fn read_u64(data: &[u8], pos: &mut usize) -> Option<u64> {
    let v = u64::from_le_bytes(data.get(*pos..*pos + 8)?.try_into().ok()?);
    *pos += 8;
    Some(v)
}

/// Returns the serialized length of one versioned transaction starting at `start`.
///
/// The wire layout is the classic Solana transaction encoding (which wincode
/// preserves): `short_vec` signature count + signatures, then the message
/// (optional version prefix byte, 3-byte header, short_vec account keys,
/// 32-byte recent blockhash, short_vec instructions, and — for versioned
/// messages — short_vec address-table lookups).
fn tx_len(data: &[u8], start: usize) -> Option<usize> {
    let mut p = start;

    let sig_count = read_shortu16(data, &mut p)? as usize;
    p = p.checked_add(sig_count.checked_mul(64)?)?;

    // Message: a legacy message starts with num_required_signatures (< 128),
    // a versioned message starts with 0x80 | version.
    let first = *data.get(p)?;
    let versioned = first & 0x80 != 0;
    if versioned {
        p += 1;
    }

    p = p.checked_add(3)?; // header: required sigs, readonly signed, readonly unsigned

    let num_accounts = read_shortu16(data, &mut p)? as usize;
    p = p.checked_add(num_accounts.checked_mul(32)?)?;

    p = p.checked_add(32)?; // recent blockhash

    let num_ixs = read_shortu16(data, &mut p)? as usize;
    for _ in 0..num_ixs {
        p = p.checked_add(1)?; // program_id_index (u8)
        let acct_idx_len = read_shortu16(data, &mut p)? as usize;
        p = p.checked_add(acct_idx_len)?;
        let data_len = read_shortu16(data, &mut p)? as usize;
        p = p.checked_add(data_len)?;
    }

    if versioned {
        let num_lookups = read_shortu16(data, &mut p)? as usize;
        for _ in 0..num_lookups {
            p = p.checked_add(32)?; // account key
            let writable = read_shortu16(data, &mut p)? as usize;
            p = p.checked_add(writable)?;
            let readonly = read_shortu16(data, &mut p)? as usize;
            p = p.checked_add(readonly)?;
        }
    }

    if p > data.len() {
        return None;
    }
    Some(p - start)
}

/// Deshreds the concatenated data-shred payloads of a FEC set into entries and
/// their transactions.
///
/// Agave serializes the entry stream with `wincode` as a `Vec<Entry>`:
///   u64 entry_count
///   per entry: u64 num_hashes, [u8;32] hash, u64 tx_count, then the txs.
pub fn deshred_into_txs(data_payloads: &[Vec<u8>]) -> Option<DeshredResult> {
    let mut all_data = Vec::new();
    for payload in data_payloads {
        all_data.extend_from_slice(payload);
    }
    if all_data.len() < 8 {
        return None;
    }

    let mut pos = 0usize;
    let num_entries = read_u64(&all_data, &mut pos)? as usize;
    if num_entries > 1_000_000 {
        return None;
    }

    let mut entries: Vec<Entry> = Vec::with_capacity(num_entries.min(4096));
    for _ in 0..num_entries {
        let num_hashes = match read_u64(&all_data, &mut pos) {
            Some(v) => v,
            None => break,
        };
        let hash_bytes: [u8; 32] = match all_data.get(pos..pos + 32).and_then(|s| s.try_into().ok()) {
            Some(h) => h,
            None => break,
        };
        let hash = Hash::new_from_array(hash_bytes);
        pos += 32;

        let tx_count = match read_u64(&all_data, &mut pos) {
            Some(v) if v <= 100_000 => v as usize,
            _ => break,
        };

        let mut transactions: Vec<Vec<u8>> = Vec::with_capacity(tx_count.min(4096));
        let mut ok = true;
        for _ in 0..tx_count {
            match tx_len(&all_data, pos) {
                Some(len) => {
                    transactions.push(all_data[pos..pos + len].to_vec());
                    pos += len;
                }
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            break;
        }
        entries.push(Entry {
            num_hashes,
            hash,
            transactions,
        });
    }

    if entries.is_empty() {
        return None;
    }

    let transactions: Vec<Vec<u8>> = entries
        .iter()
        .flat_map(|entry| entry.transactions.iter().cloned())
        .collect();

    Some(DeshredResult {
        entries,
        transactions,
    })
}
