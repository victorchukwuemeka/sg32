use crate::contact_info::ContactInfo;
use serde::{Deserialize, Serialize};
use solana_bloom::bloom::{Bloom, ConcurrentBloom};
use solana_sdk::{
    hash::Hash,
    pubkey::Pubkey,
    signature::Signature,
    signer::{keypair::Keypair, Signer},
};

const MASK_BITS: u32 = 7427;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
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

// NOTE about bloom sizing:
//   HOW AGAVE DOES IT:
//   Agave's `get_max_bloom_filter_bytes(caller)` computes the exact byte budget for the bloom:
//     let available = PACKET_DATA_SIZE - 4(enum tag) - caller.bincode_serialized_size()
//     let bloom_max_bytes = precomputed_cache[available]
//   The cache maps serialized-CrdsFilter-size → max-bloom-bytes by iterating all possible
//   bloom sizes (1..PACKET_DATA_SIZE) and recording what serialized size each produces.
//   This guarantees the full Protocol::PullRequest(CrdsFilter, CrdsValue) fits in one packet.
//
//   SIMPLIFIED APPROACH (used here):
//   We estimate: CrdsFilter wire size ≈ 8(bincode vec prefix) + bloom_bytes + 8(mask) + 4(mask_bits)
//   The caller picks a bloom_max_bytes that leaves room for that overhead.
//   See pull_request.rs for where the budget is computed.
impl CrdsFilter {
    /// Creates a `CrdsFilter` whose bloom bit-array is limited to `bloom_max_bytes * 8` bits.
    /// `num_items` is the number of entries expected — used to compute mask_bits for
    /// partitioning the filter space (higher mask_bits = more sub-filters, fewer collisions).
    pub fn new(bloom_max_bytes: usize, num_items: usize) -> Self {
        // Convert the byte budget to a bit budget
        let max_bits = (bloom_max_bytes * 8) as f64;
        let false_rate: f64 = 0.1;
        let num_keys: f64 = 8.0;

        // Standard bloom filter capacity formula:
        //   max_items = max_bits / (-num_keys / ln(1 - p^(1/num_keys)))
        // where p = false positive rate, k = number of hash functions
        let ln_p = false_rate.ln();
        let inner = 1.0f64 - (ln_p / num_keys).exp();
        let max_items = (max_bits / (-num_keys / inner.ln())).ceil() as usize;

        // Determine mask_bits: how many bits of the hash to use for partitioning
        let num_items_u32 = num_items.min(max_items) as u32;
        let max_items_u32 = max_items as u32;
        let mask_bits = compute_mask_bits(num_items_u32, max_items_u32);

        Self {
            filter: Bloom::random(max_items, false_rate, max_bits as usize),
            mask: compute_mask(0, mask_bits),
            mask_bits,
        }
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
