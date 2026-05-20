use crate::gf256::{gf_add, gf_inv, gf_mul, gf_sub};

pub const NUM_DATA_SHREDS: usize = 32;
pub const NUM_CODE_SHREDS: usize = 32;

// ── Cauchy Matrix ────────────────────────────────────────────────────────────
// A table of weights where C[i][j] = 1 / (x_i + y_j) in GF(2^8).
//
// Each row i is the recipe for code shred i:
//   code_i[col] = Σ_j C[i][j] × data_j[col]
//   (sum and multiply in GF(2^8))
//
// Each row uses distinct x_i and y_j so that x_i ≠ y_j for all i,j,
// guaranteeing x_i + y_j ≠ 0 and thus every entry is invertible.
// The magic property: ANY square submatrix of a Cauchy matrix is invertible.
// This means no matter which N rows you keep (data or code), you can always
// solve the system and recover the originals.

pub fn generate_cauchy_matrix(num_data: usize, num_code: usize) -> Vec<Vec<u8>> {
    let mut matrix = vec![vec![0u8; num_data]; num_code];
    for i in 0..num_code {
        for j in 0..num_data {
            let x_i = i as u8;
            let y_j = (num_data + j) as u8;
            // C[i][j] = 1 / (x_i + y_j)
            matrix[i][j] = gf_inv(gf_add(x_i, y_j));
        }
    }
    matrix
}

// ── Encoder ──────────────────────────────────────────────────────────────────
// Takes 32 data shreds, produces 32 code shreds.
// Each code shred byte is a weighted sum over all 32 data shreds at that column.

pub fn encode(
    data_shreds: &[Vec<u8>],
    cauchy: &[Vec<u8>],
    num_code: usize,
) -> Vec<Vec<u8>> {
    let shred_size = data_shreds[0].len();
    let mut code_shreds = vec![vec![0u8; shred_size]; num_code];

    for i in 0..num_code {
        for col in 0..shred_size {
            let mut sum = 0u8;
            for j in 0..data_shreds.len() {
                sum = gf_add(sum, gf_mul(data_shreds[j][col], cauchy[i][j]));
            }
            code_shreds[i][col] = sum;
        }
    }

    code_shreds
}

// ── Decoder ──────────────────────────────────────────────────────────────────
// Recovers the original 32 data shreds from any 32 received shreds.
//
// `received`: 32 shreds we received (a mix of data and code)
// `row_indices`: identifies each received shred:
//    - index 0..num_data       → data shred at that position
//    - index num_data..        → code shred at (index - num_data)
// `cauchy`: full Cauchy matrix from encoding

pub fn decode(
    received: &[Vec<u8>],
    row_indices: &[usize],
    cauchy: &[Vec<u8>],
    num_data: usize,
) -> Option<Vec<Vec<u8>>> {
    assert_eq!(received.len(), num_data);
    assert_eq!(row_indices.len(), num_data);

    let shred_size = received[0].len();

    // Step 1: Build the N×N matrix from the rows we received.
    let mut matrix = vec![vec![0u8; num_data]; num_data];
    for (r, &idx) in row_indices.iter().enumerate() {
        if idx < num_data {
            // Received a data shred → row is a unit vector (1 at its index).
            matrix[r][idx] = 1;
        } else {
            // Received a code shred → row is its Cauchy weights.
            let code_idx = idx - num_data;
            matrix[r].copy_from_slice(&cauchy[code_idx]);
        }
    }

    // Step 2: Invert the matrix in GF(2^8).
    let inverted = gf_matrix_invert(&mut matrix)?;

    // Step 3: Multiply inverted matrix by received shred data,
    //         column by column, to recover all 32 original data shreds.
    let mut recovered = vec![vec![0u8; shred_size]; num_data];
    for i in 0..num_data {
        for col in 0..shred_size {
            let mut sum = 0u8;
            for k in 0..num_data {
                sum = gf_add(sum, gf_mul(inverted[i][k], received[k][col]));
            }
            recovered[i][col] = sum;
        }
    }

    Some(recovered)
}

// ── Matrix Inversion (GF(2^8)) ───────────────────────────────────────────────
// Gaussian elimination on an augmented matrix [A | I] → [I | A⁻¹].
// All arithmetic uses GF(2^8) operations via our gf256 module.

fn gf_matrix_invert(matrix: &mut [Vec<u8>]) -> Option<Vec<Vec<u8>>> {
    let n = matrix.len();
    let mut aug = vec![vec![0u8; 2 * n]; n];
    for i in 0..n {
        for j in 0..n {
            aug[i][j] = matrix[i][j];
        }
        aug[i][n + i] = 1;
    }

    for col in 0..n {
        let pivot = (col..n).find(|&row| aug[row][col] != 0)?;
        aug.swap(col, pivot);

        let inv = gf_inv(aug[col][col]);
        for j in 0..2 * n {
            aug[col][j] = gf_mul(aug[col][j], inv);
        }

        for row in 0..n {
            if row != col && aug[row][col] != 0 {
                let factor = aug[row][col];
                for j in 0..2 * n {
                    aug[row][j] = gf_sub(aug[row][j], gf_mul(factor, aug[col][j]));
                }
            }
        }
    }

    let mut inv = vec![vec![0u8; n]; n];
    for i in 0..n {
        for j in 0..n {
            inv[i][j] = aug[i][n + j];
        }
    }

    Some(inv)
}
