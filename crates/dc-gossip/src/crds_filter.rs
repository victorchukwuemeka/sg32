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
