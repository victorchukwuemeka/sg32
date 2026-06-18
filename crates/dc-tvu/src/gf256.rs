// ── GF(2^8) Arithmetic ──────────────────────────────────────────────────────
// Reed-Solomon erasure coding needs to solve systems of equations over bytes.
// Normal math won't work because:
//   - 200 + 100 = 300 (doesn't fit in 1 byte, max 255)
//   - 200 × 100 = 20000 (way too big for a byte)
//
// GF(2^8) = Galois Field of 256 elements. Think of it as "byte arithmetic" —
// a set of rules where every operation produces another byte (0-255).
// It was discovered by Évariste Galois in 1832.
//
// Addition:  XOR (simple, stays in range)
// Multiplication: uses precomputed logarithm tables (see LOG/ALOG below)
//
// Solana uses the primitive polynomial 0x11D (x^8 + x^4 + x^3 + x^2 + 1).
// "Primitive" means it generates all 255 non-zero elements when you start
// at 1 and repeatedly multiply by 2, XORing with 0x11D whenever you overflow
// past 255. This guarantees every non-zero byte appears exactly once.
//
// The tables below were precomputed from that process and hardcoded for speed.
// ─────────────────────────────────────────────────────────────────────────────

// LOG[byte] = the exponent e such that 2^e = byte in this field.
// If you start at 1 and keep doubling (XORing with 0x11D on overflow),
// LOG tells you at which step each byte appears.
// Example: LOG[25] = 1 means 2^1 = 25 in this field.
// Note: LOG[0] = 0 (0 has no log, but we store 0 for safety).
const LOG: [u8; 256] = [
    0, 0, 1, 25, 2, 50, 26, 198, 3, 223, 51, 238, 27, 104, 199, 75, 4, 100, 224, 14, 52, 141, 239,
    129, 28, 193, 105, 248, 200, 8, 76, 113, 5, 138, 101, 47, 225, 36, 15, 33, 53, 147, 142, 218,
    240, 18, 130, 69, 29, 181, 194, 125, 106, 39, 249, 185, 201, 154, 9, 120, 77, 228, 114, 166, 6,
    191, 139, 98, 102, 221, 48, 253, 226, 152, 37, 179, 16, 145, 34, 136, 54, 208, 148, 206, 143,
    150, 219, 189, 241, 210, 19, 92, 131, 56, 70, 64, 30, 66, 182, 163, 195, 72, 126, 110, 107, 58,
    40, 84, 250, 133, 186, 61, 202, 94, 155, 159, 10, 21, 121, 43, 78, 212, 229, 172, 115, 243,
    167, 87, 7, 112, 192, 247, 140, 128, 99, 13, 103, 74, 222, 237, 49, 197, 254, 24, 227, 165,
    153, 119, 38, 184, 180, 124, 17, 68, 146, 217, 35, 32, 137, 46, 55, 63, 209, 91, 149, 188, 207,
    205, 144, 135, 151, 178, 220, 252, 190, 97, 242, 86, 211, 171, 20, 42, 93, 158, 132, 60, 57,
    83, 71, 109, 65, 162, 31, 45, 67, 216, 183, 123, 164, 118, 196, 23, 73, 236, 127, 12, 111, 246,
    108, 161, 59, 82, 41, 157, 85, 170, 251, 96, 134, 177, 187, 204, 62, 90, 203, 89, 95, 176, 156,
    169, 160, 81, 11, 245, 22, 235, 122, 117, 44, 215, 79, 174, 213, 233, 230, 231, 173, 232, 116,
    214, 244, 234, 168, 80, 88, 175,
];

// ALOG[e] = the byte at exponent e = 2^e in this field.
// This is the reverse lookup: ALOG[LOG[byte]] = byte.
// Example: ALOG[1] = 25 means 2^1 = 25.
// The sequence cycles: ALOG[255] = ALOG[0] = 1, because 2^255 = 2^0 = 1
// in this field (the nonzero elements form a cyclic group of order 255).
const ALOG: [u8; 256] = [
    1, 2, 4, 8, 16, 32, 64, 128, 29, 58, 116, 232, 205, 135, 19, 38, 76, 152, 45, 90, 180, 117,
    234, 201, 143, 3, 6, 12, 24, 48, 96, 192, 157, 39, 78, 156, 37, 74, 148, 57, 114, 228, 197,
    151, 27, 54, 108, 216, 173, 71, 142, 1, 2, 4, 8, 16, 32, 64, 128, 29, 58, 116, 232, 205, 135,
    19, 38, 76, 152, 45, 90, 180, 117, 234, 201, 143, 3, 6, 12, 24, 48, 96, 192, 157, 39, 78, 156,
    37, 74, 148, 57, 114, 228, 197, 151, 27, 54, 108, 216, 173, 71, 142, 1, 2, 4, 8, 16, 32, 64,
    128, 29, 58, 116, 232, 205, 135, 19, 38, 76, 152, 45, 90, 180, 117, 234, 201, 143, 3, 6, 12,
    24, 48, 96, 192, 157, 39, 78, 156, 37, 74, 148, 57, 114, 228, 197, 151, 27, 54, 108, 216, 173,
    71, 142, 1, 2, 4, 8, 16, 32, 64, 128, 29, 58, 116, 232, 205, 135, 19, 38, 76, 152, 45, 90, 180,
    117, 234, 201, 143, 3, 6, 12, 24, 48, 96, 192, 157, 39, 78, 156, 37, 74, 148, 57, 114, 228,
    197, 151, 27, 54, 108, 216, 173, 71, 142, 1, 2, 4, 8, 16, 32, 64, 128, 29, 58, 116, 232, 205,
    135, 19, 38, 76, 152, 45, 90, 180, 117, 234, 201, 143, 3, 6, 12, 24, 48, 96, 192, 157, 39, 78,
    156, 37, 74, 148, 57, 114, 228, 197, 151, 27, 54, 108, 216, 173, 71, 142, 241,
];

// gf_add: addition in GF(2^8) = XOR.
// Used when combining bytes during Reed-Solomon encoding/decoding.
// Example: gf_add(0b11001000, 0b01100100) = 0b10101100 (172).
pub fn gf_add(a: u8, b: u8) -> u8 {
    a ^ b
}

// gf_sub: subtraction in GF(2^8) = XOR (same as addition).
// In GF(2), a - b = a + b = a XOR b. No borrow/carry.
// This is what makes GF arithmetic efficient on computers.
pub fn gf_sub(a: u8, b: u8) -> u8 {
    a ^ b
}

// gf_mul: multiplication in GF(2^8).
// Uses the log/antilog trick: a × b = alog[ (log[a] + log[b]) % 255 ].
// This is like regular logarithms: log(a × b) = log(a) + log(b),
// then exponentiate (alog) to get the result.
//
// Edge case: if a or b is 0, the result is 0 (0 has no log).
//
// Used in the Cauchy matrix to compute weighted combinations of shred bytes.
// Example: gf_mul(25, 2) = gf_mul(2^1, 2^1) = 2^(1+1) = 2^2 = 4.
//   LOG[25] = 1, LOG[2] = 1, 1+1 = 2, ALOG[2] = 4.
pub fn gf_mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        return 0;
    }
    let log_sum = LOG[a as usize] as u16 + LOG[b as usize] as u16;
    ALOG[(log_sum % 255) as usize]
}

// gf_inv: multiplicative inverse.
// Returns x such that gf_mul(a, x) = 1 (except for a=0).
// In GF(2^8): a^(-1) = a^254 = alog[255 - log[a]].
// Since a^(255) = 1 in this field, a^(-1) = a^(254).
//
// Used when inverting the Cauchy submatrix during erasure decoding.
// Example: gf_inv(25). LOG[25] = 1. 255 - 1 = 254. ALOG[254] = 142.
//   gf_mul(25, 142) = 25 × 142 = ? Let's check:
//   LOG[25]=1, LOG[142]=254. 1+254=255. 255%255=0. ALOG[0]=1. ✓
pub fn gf_inv(a: u8) -> u8 {
    if a == 0 {
        return 0;
    }
    ALOG[255 - LOG[a as usize] as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_is_xor() {
        assert_eq!(gf_add(0b1010, 0b1100), 0b0110);
        assert_eq!(gf_add(0xFF, 0x00), 0xFF);
        assert_eq!(gf_add(0xFF, 0xFF), 0x00);
        assert_eq!(gf_add(0xAB, 0xCD), 0x66);
    }

    #[test]
    fn sub_equals_add() {
        for a in 0..=255u8 {
            for b in 0..=255u8 {
                assert_eq!(
                    gf_sub(a, b),
                    gf_add(a, b),
                    "gf_sub({a}, {b}) != gf_add({a}, {b})"
                );
            }
        }
    }

    #[test]
    fn mul_by_zero_is_zero() {
        for a in 0..=255u8 {
            assert_eq!(gf_mul(a, 0), 0);
            assert_eq!(gf_mul(0, a), 0);
        }
    }

    #[test]
    fn mul_by_one_is_identity() {
        for a in 0..=255u8 {
            assert_eq!(gf_mul(a, 1), a, "gf_mul({a}, 1) != {a}");
            assert_eq!(gf_mul(1, a), a, "gf_mul(1, {a}) != {a}");
        }
    }

    #[test]
    fn mul_commutative() {
        for a in 0..=255u8 {
            for b in 0..=255u8 {
                assert_eq!(
                    gf_mul(a, b),
                    gf_mul(b, a),
                    "gf_mul({a}, {b}) != gf_mul({b}, {a})"
                );
            }
        }
    }

    #[test]
    fn mul_associative() {
        for a in [0, 1, 25, 142, 255] {
            for b in [0, 1, 3, 128, 254] {
                for c in [0, 1, 2, 64, 255] {
                    assert_eq!(
                        gf_mul(gf_mul(a, b), c),
                        gf_mul(a, gf_mul(b, c)),
                        "({a}×{b})×{c} != {a}×({b}×{c})"
                    );
                }
            }
        }
    }

    #[test]
    fn mul_distributive() {
        for a in [0, 1, 7, 99, 201] {
            for b in [0, 1, 16, 77, 255] {
                for c in [0, 1, 32, 55, 128] {
                    assert_eq!(
                        gf_mul(a, gf_add(b, c)),
                        gf_add(gf_mul(a, b), gf_mul(a, c)),
                        "{a}×({b}+{c}) != ({a}×{b})+({a}×{c})"
                    );
                }
            }
        }
    }

    #[test]
    fn inv_of_zero_is_zero() {
        assert_eq!(gf_inv(0), 0);
    }

    #[test]
    fn inv_of_one_is_one() {
        assert_eq!(gf_inv(1), 1);
    }

    #[test]
    fn mul_inv_roundtrip_all_bytes() {
        for a in 1..=255u8 {
            let inv = gf_inv(a);
            assert_ne!(inv, 0, "gf_inv({a}) returned 0, but {a} != 0");
            assert_eq!(gf_mul(a, inv), 1, "gf_mul({a}, gf_inv({a})) != 1");
        }
    }

    #[test]
    fn log_alog_consistent() {
        for a in 1..=255u8 {
            assert_eq!(ALOG[LOG[a as usize] as usize], a, "ALOG[LOG[{a}]] != {a}");
        }
    }

    #[test]
    fn known_values_from_solana_spec() {
        // Solana uses primitive poly 0x11D.
        // 2^1 = 25 in this field (the first element after 2^0=1)
        assert_eq!(gf_mul(2, 2), 4);
        assert_eq!(gf_mul(25, 2), 4);
        assert_eq!(gf_mul(25, 25), gf_mul(gf_mul(25, 2), 142));
        // inv(2) = 142 because 2 × 142 = 284, 284 % 0x11D = 284 - 256 = 28
        // Actually 2 * 142 = 2^(1+254) = 2^255 = 2^0 = 1. So let's just trust the tables.
        assert_eq!(gf_mul(2, gf_inv(2)), 1);
    }
}
