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
    fn max_items(max_bits: f64, false_rate: f64, num_keys: f64) -> f64 {
        let m = max_bits;
        let p = false_rate;
        let k = num_keys;
        (m / (-k / (1f64 - (p.ln() / k).exp()).ln())).ceil()
    }

    fn compute_mask_bits(num_items: f64, max_items: f64) -> u32 {
        ((num_items / max_items).log2().ceil()).max(0.0) as u32
    }

    pub fn mask_bits(&self) -> u32 {
        self.mask_bits
    }

    /// Creates a `CrdsFilter` whose bloom bit-array is limited to `bloom_max_bytes * 8` bits.
    /// `num_items` is the number of entries expected — used to compute mask_bits for
    /// partitioning the filter space (higher mask_bits = more sub-filters, fewer collisions).
    pub fn new(bloom_max_bytes: usize, num_items: usize) -> Self {
        let max_bits = (bloom_max_bytes * 8) as f64;
        let false_rate: f64 = 0.1;
        let num_keys: f64 = 8.0;

        let max_items_val = Self::max_items(max_bits, false_rate, num_keys);

        // Agave's entrypoint (release mode) requires mask_bits >= MIN_PULL_REQUEST_MASK_BITS
        // where MIN_NUM_BLOOM_ITEMS = 65536. Use at least that many items so our mask_bits
        // passes the sanitization check.
        const MIN_NUM_BLOOM_ITEMS: usize = 65536;
        let effective_items = num_items.max(MIN_NUM_BLOOM_ITEMS);

        // mask_bits is computed from num_items (not max_items) to control filter
        // partitioning — higher mask_bits = fewer values per bucket.
        let mask_bits_val = Self::compute_mask_bits(effective_items as f64, max_items_val);

        // Bloom::random must be called with max_items (bloom capacity), NOT with
        // effective_items. Passing 65536 items to a bloom with only 8256 bits yields
        // a single hash function and ~99.96% false-positive rate, causing the
        // entrypoint to think we already have every value and return nothing.
        let filter = Bloom::random(max_items_val as usize, false_rate, max_bits as usize);

        Self {
            filter,
            mask: compute_mask(0, mask_bits_val),
            mask_bits: mask_bits_val,
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

impl Default for CrdsFilter {
    fn default() -> Self {
        let max_bits = MASK_BITS as f64;
        let false_rate: f64 = 0.1;
        let num_keys: f64 = 8.0;
        let max_items = Self::max_items(max_bits, false_rate, num_keys) as usize;
        let num_items: usize = 0;
        let mask_bits = Self::compute_mask_bits(num_items as f64, max_items as f64);

        Self {
            filter: Bloom::random(max_items, false_rate, max_bits as usize),
            mask: compute_mask(0, mask_bits),
            mask_bits,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_filter() {
        let f = CrdsFilter::new(512, 0);
        assert!(f.mask_bits() >= 6, "mask_bits should be >= 6 for release-mode Agave compat");
    }

    #[test]
    fn new_zero_bytes() {
        let f = CrdsFilter::new(1, 0);
        // Must not panic
        assert!(f.mask_bits() >= 6);
    }

    #[test]
    fn mask_bits_scales_with_items() {
        let small = CrdsFilter::new(512, 0);
        let big = CrdsFilter::new(512, 100_000);
        assert!(big.mask_bits() >= small.mask_bits());
    }

    #[test]
    fn larger_bloom_lower_mask_bits() {
        let small_budget = CrdsFilter::new(200, 0);
        let large_budget = CrdsFilter::new(1000, 0);
        assert!(
            large_budget.mask_bits() <= small_budget.mask_bits(),
            "more bloom space should reduce mask_bits (need fewer partitions)"
        );
    }

    #[test]
    fn compute_mask_zero_bits() {
        let mask = compute_mask(0, 0);
        assert_eq!(mask, u64::MAX);
    }

    #[test]
    fn compute_mask_max_bits() {
        let mask = compute_mask(0x1234, 64);
        assert_eq!(mask, 0x1234);
    }

    #[test]
    fn default_creates_filter() {
        let f = CrdsFilter::default();
        assert!(f.mask_bits() <= 32, "default mask_bits should be reasonable");
    }

    #[test]
    fn filter_is_deterministic() {
        let a = CrdsFilter::new(512, 0);
        let b = CrdsFilter::new(512, 0);
        // Same inputs should produce the same mask_bits
        assert_eq!(a.mask_bits(), b.mask_bits());
    }
}
