use crate::contact_info::ContactInfo;
use crate::crds_data::CrdsValue;
use crate::crds_filter::CrdsFilter;
use crate::protocol::Protocol;
use solana_sdk::signature::Keypair;
use std::sync::LazyLock;
use thiserror::Error;

// Solana's standard UDP packet payload size (MTU-safe):
// 1232 bytes — everything (enum tag + CrdsFilter + CrdsValue) must fit in this.
const PACKET_DATA_SIZE: usize = 1232;

// Precomputed cache: maps target-serialized-size → bloom_max_bytes.
//
// HOW AGAVE DOES IT (crds_gossip_pull.rs):
//   The cache is built once via LazyLock by iterating bloom_max_bytes = 1..PACKET_DATA_SIZE,
//   computing the resulting CrdsFilter's serialized size, and storing the mapping.
//   Forward-fill zeros so any gap maps to the nearest valid bloom size.
//
//   At query time:
//     target_size = PACKET_DATA_SIZE - 4(enum tag) - caller.bincode_serialized_size()
//     bloom_max_bytes = cache[target_size]
//
//   This guarantees the full PullRequest fits exactly in PACKET_DATA_SIZE bytes.
static MAX_BLOOM_BYTES_CACHE: LazyLock<[u16; PACKET_DATA_SIZE + 1]> = LazyLock::new(|| {
    let mut cache = [0u16; PACKET_DATA_SIZE + 1];
    for bloom_bytes in 1..=PACKET_DATA_SIZE {
        let filter = CrdsFilter::new(bloom_bytes, /*num_items=*/ 0);
        if let Ok(size) = bincode::serialized_size(&filter) {
            let idx = size as usize;
            if idx <= PACKET_DATA_SIZE {
                cache[idx] = bloom_bytes as u16;
            }
        }
    }
    // Forward-fill zeros: if target_size lands on a gap, use the last known good size
    let mut last = 0u16;
    for entry in cache.iter_mut() {
        if *entry == 0 {
            *entry = last;
        }
        last = *entry;
    }
    cache
});

// Looks up the optimal bloom_max_bytes so the serialized CrdsFilter fits in PACKET_DATA_SIZE.
fn get_max_bloom_filter_bytes(caller_size: usize) -> usize {
    let target = PACKET_DATA_SIZE
        .checked_sub(4 + caller_size)
        .unwrap_or(0);
    MAX_BLOOM_BYTES_CACHE
        .get(target)
        .copied()
        .map(usize::from)
        .unwrap_or(0)
}

pub fn create_pull_request_message(
    contact_info: ContactInfo,
    keypair: &Keypair,
) -> Result<Vec<u8>, PullRequestErrorMessages> {
    // Validate that we have at least one socket entry
    let socket_ip = contact_info.sockets();
    if socket_ip.is_empty() {
        return Err(PullRequestErrorMessages::NoSocketEntry);
    }

    // Must use modern ContactInfo (CrdsData::ContactInfo, variant 11) for PullRequest.
    // Agave's Protocol::sanitize rejects LegacyContactInfo (variant 0) via
    // its Deprecated wrapper, causing silent deserialization failure.
    let signed_info = CrdsValue::new_contact_info(contact_info, keypair);

    // Debug: print the exact bytes the signature was computed over
    let data_bytes = bincode::serialize(&signed_info.data).unwrap();
    eprintln!(
        "[CRDS_SIGNED] {} bytes: {}",
        data_bytes.len(),
        data_bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
    );

    // 1) Measure the caller's serialized size.
    //    This determines exactly how much room remains for the CrdsFilter.
    let caller_size = bincode::serialized_size(&signed_info)
        .map_err(|_| PullRequestErrorMessages::SerializeFailed)?;
    let caller_size = caller_size as usize;

    // 2) Use the precomputed cache to get the exact bloom_max_bytes that makes
    //    the total PullRequest fit in PACKET_DATA_SIZE (same approach as Agave).
    let bloom_max_bytes = get_max_bloom_filter_bytes(caller_size);

    // 3) Create the CrdsFilter with the exact bloom budget
    let filter = CrdsFilter::new(bloom_max_bytes, /*num_items=*/ 0);

    // Debug: verify the bloom we're sending
    let actual_keys = filter.filter.keys.len();
    let sample = solana_sdk::hash::Hash::new_from_array([0xabu8; 32]);
    let contains = filter.filter.contains(&sample);
    let wire_size = bincode::serialized_size(&Protocol::PullRequest(filter.clone(), signed_info.clone())).unwrap_or(0);
    eprintln!(
        "[PULL_REQ] bloom_max_bytes={} mask_bits={} keys={} contains_sample={} caller_size={} wire_size={}",
        bloom_max_bytes, filter.mask_bits(), actual_keys, contains, caller_size, wire_size
    );
    if contains {
        eprintln!("[PULL_REQ] CRITICAL: empty bloom says contains=true! ALL values will be filtered out!");
    }

    // 4) Encode the pull request — guaranteed to fit in one packet
    Protocol::PullRequest(filter, signed_info)
        .encode_to()
        .map_err(|_| PullRequestErrorMessages::SerializeFailed)
}

#[derive(Error, Debug)]
pub enum PullRequestErrorMessages {
    #[error("No socket adress in contact info")]
    NoSocketEntry,
    #[error("Failed to serialize message")]
    SerializeFailed,
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        solana_sdk::{signer::Signer, timing::timestamp},
        std::net::SocketAddr,
    };

    #[test]
    fn test_create_pull_request_with_no_gossip_addres() {
        let keypair = Keypair::new();
        let contact_info = ContactInfo::default();

        // No gossip socket → should fail
        let pull_request = create_pull_request_message(contact_info, &keypair);

        assert!(pull_request.is_err())
    }

    #[test]
    fn test_create_pull_request() {
        let keypair = Keypair::new();
        let gossip: SocketAddr = "0.0.0.0:8100"
            .parse()
            .expect("Failed create entrypoint socket");
        let contact_info = ContactInfo::new(keypair.pubkey(), timestamp(), gossip, 0);

        // Filter is now sized automatically inside create_pull_request_message
        let pull_request = create_pull_request_message(contact_info, &keypair);

        assert!(pull_request.is_ok())
    }
}
