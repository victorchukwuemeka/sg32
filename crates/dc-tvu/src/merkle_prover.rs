use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct MerkleTree {
    pub root: [u8; 32],
    pub leaves: Vec<[u8; 32]>,
    layers: Vec<Vec<[u8; 32]>>,
}

impl MerkleTree {
    pub fn new(entries: &[Vec<u8>]) -> Self {
        let leaves: Vec<[u8; 32]> = entries.iter().map(|e| Sha256::digest(e).into()).collect();

        let mut layers = vec![leaves.clone()];
        let mut current = leaves.clone();

        while current.len() > 1 {
            let mut next = Vec::with_capacity((current.len() + 1) / 2);
            for pair in current.chunks(2) {
                let mut hasher = Sha256::new();
                hasher.update(&pair[0]);
                if pair.len() > 1 {
                    hasher.update(&pair[1]);
                } else {
                    hasher.update(&pair[0]);
                }
                next.push(hasher.finalize().into());
            }
            layers.push(next.clone());
            current = next;
        }

        let root = if current.is_empty() {
            [0u8; 32]
        } else {
            current[0]
        };

        Self {
            root,
            leaves,
            layers,
        }
    }

    pub fn prove(&self, index: usize) -> Option<Vec<[u8; 32]>> {
        if index >= self.leaves.len() {
            return None;
        }

        let mut proof = Vec::new();
        let mut idx = index;

        for layer in &self.layers[..self.layers.len() - 1] {
            let sibling = if idx % 2 == 0 {
                if idx + 1 < layer.len() {
                    layer[idx + 1]
                } else {
                    layer[idx]
                }
            } else {
                layer[idx - 1]
            };
            proof.push(sibling);
            idx /= 2;
        }

        Some(proof)
    }

    pub fn verify(root: &[u8; 32], leaf: &[u8; 32], proof: &[[u8; 32]], index: usize) -> bool {
        let mut hash = *leaf;
        let mut idx = index;

        for sibling in proof {
            let mut hasher = Sha256::new();
            if idx % 2 == 0 {
                hasher.update(&hash);
                hasher.update(sibling);
            } else {
                hasher.update(sibling);
                hasher.update(&hash);
            }
            hash = hasher.finalize().into();
            idx /= 2;
        }

        &hash == root
    }
}
