use anyhow::Result;
use rand::Rng;
use serde::{Deserialize, Serialize};
use solana_sdk::hash::Hash;
use solana_sdk::{pubkey::Pubkey, signature::Signature, signer::keypair::Keypair, signer::Signer};

const PING_TOKEN_SIZE: usize = 32;
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Ping {
    pub from: Pubkey,
    pub token: [u8; PING_TOKEN_SIZE],
    pub signature: Signature,
}

impl Ping {
    pub fn new(key: &Keypair) -> Result<Self> {
        let mut token = [0u8; PING_TOKEN_SIZE];
        rand::rng().fill_bytes(&mut token);
        let signature = key.sign_message(&token);

        Ok(Self {
            from: key.pubkey(),
            token,
            signature,
        })
    }
}

#[derive(Serialize, Debug, Clone, Deserialize)]
pub struct Pong {
    pub from: Pubkey,
    pub hash: Hash,
    pub signature: Signature,
}

impl Pong {
    pub fn new(ping: &Ping, keypair: &Keypair) -> Result<Self> {
        let mut buf = vec![];
        buf.extend_from_slice(b"SOLANA_PING_PONG");
        buf.extend_from_slice(&ping.token);
        let hash = solana_sdk::hash::hash(&buf);
        let signature = keypair.sign_message(hash.as_ref());
        Ok(Self {
            from: keypair.pubkey(),
            hash,
            signature,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::signer::Signer;

    #[test]
    fn ping_new_creates_valid_ping() {
        let keypair = Keypair::new();
        let ping = Ping::new(&keypair).unwrap();
        assert_eq!(ping.from, keypair.pubkey());
        assert_ne!(ping.token, [0u8; 32]);
        // signature should verify against the token
        assert!(ping.signature.verify(ping.from.as_ref(), &ping.token));
    }

    #[test]
    fn ping_token_is_random() {
        let keypair = Keypair::new();
        let ping1 = Ping::new(&keypair).unwrap();
        let ping2 = Ping::new(&keypair).unwrap();
        assert_ne!(ping1.token, ping2.token);
    }

    #[test]
    fn ping_serialization_roundtrip() {
        let keypair = Keypair::new();
        let ping = Ping::new(&keypair).unwrap();
        let bytes = bincode::serialize(&ping).unwrap();
        let deserialized: Ping = bincode::deserialize(&bytes).unwrap();
        assert_eq!(deserialized.from, ping.from);
        assert_eq!(deserialized.token, ping.token);
        assert_eq!(deserialized.signature, ping.signature);
    }

    #[test]
    fn pong_from_ping_creates_valid_pong() {
        let keypair = Keypair::new();
        let ping = Ping::new(&keypair).unwrap();
        let pong = Pong::new(&ping, &keypair).unwrap();
        assert_eq!(pong.from, keypair.pubkey());
        // Pong hash = hash("SOLANA_PING_PONG" + ping.token)
        let mut expected = vec![];
        expected.extend_from_slice(b"SOLANA_PING_PONG");
        expected.extend_from_slice(&ping.token);
        let expected_hash = solana_sdk::hash::hash(&expected);
        assert_eq!(pong.hash, expected_hash);
        assert!(pong.signature.verify(pong.from.as_ref(), pong.hash.as_ref()));
    }

    #[test]
    fn pong_deterministic_hash() {
        let keypair = Keypair::new();
        let ping = Ping::new(&keypair).unwrap();
        let pong1 = Pong::new(&ping, &keypair).unwrap();
        let pong2 = Pong::new(&ping, &keypair).unwrap();
        assert_eq!(pong1.hash, pong2.hash);
    }

    #[test]
    fn pong_serialization_roundtrip() {
        let keypair = Keypair::new();
        let ping = Ping::new(&keypair).unwrap();
        let pong = Pong::new(&ping, &keypair).unwrap();
        let bytes = bincode::serialize(&pong).unwrap();
        let deserialized: Pong = bincode::deserialize(&bytes).unwrap();
        assert_eq!(deserialized.from, pong.from);
        assert_eq!(deserialized.hash, pong.hash);
        assert_eq!(deserialized.signature, pong.signature);
    }
}
