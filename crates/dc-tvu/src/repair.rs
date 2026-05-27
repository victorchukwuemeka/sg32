use anyhow::Result;
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;

// remember we are not even in the turbine tree so
// so no one is sending us shred and for that to happen we even
// need a validators stake you get
// so for the repair protocol we need to send a pull mechanism
// for a specific validator and its shred  and they respond with their
// raw bytes   from their own blockstore .

//windowindex is the 8 variant in the repairprotocol enum
// from agave
const ENUM_TAG_WINDOW_INDEX: u32 = 8;

///our Ed25518 signature are always 64 bytes
const SIG_BYTES: usize = 64;

const PING_BYTES: usize = 4 + 32 + 32 + 64;

pub enum Response {
    Shred { bytes: Vec<u8>, nonce: u32 },
    Ping { token: [u8; 32], pubkey: [u8; 32] },
}

/// Build a signed RepairProtocol::WindowIndex and fire it off.
pub async fn send_repair_request(
    socket: &UdpSocket,
    dest: SocketAddr,
    our_key: &SigningKey,
    validator_pub: &[u8; 32],
    slot: u64,
    shred_idx: u64,
) -> Result<u32> {
    let nonce: u32 = rand::random();

    // Must be within 10 min of validator's clock or they drop with TimeSkew
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;

    //WE PACK THE BYTES MANUALLY
    let mut buf = Vec::with_capacity(160);

    // (1) Enum discriminator — bincode uses u32 LE variant index
    buf.extend_from_slice(&ENUM_TAG_WINDOW_INDEX.to_le_bytes());
    // (2) Signature placeholder — zeros for now, will sign and overwrite
    buf.extend_from_slice(&[0u8; SIG_BYTES]);
    // (3) Sender pubkey — our node identity, from our signing key
    buf.extend_from_slice(&our_key.verifying_key().to_bytes());
    // (4) Recipient pubkey — the validator we're asking
    buf.extend_from_slice(validator_pub);
    // (5) Timestamp — unix microseconds
    buf.extend_from_slice(&timestamp.to_le_bytes());
    // (6) Nonce — random, validator appends this to the response
    buf.extend_from_slice(&nonce.to_le_bytes());
    // (7) Slot
    buf.extend_from_slice(&slot.to_le_bytes());
    // (8) Shred index
    buf.extend_from_slice(&shred_idx.to_le_bytes());
    // ── Sign ──
    // The signature covers bytes 0..4 (enum tag) ++ bytes 68..end (everything
    // after the signature field). This matches Agave's scheme exactly.
    let signable: Vec<u8> = [&buf[..4], &buf[4 + SIG_BYTES..]].concat();
    let signature = our_key.sign(&signable);
    // Place the 64-byte signature at offset 4, overwriting the zeros
    buf[4..4 + SIG_BYTES].copy_from_slice(&signature.to_bytes());

    // ── Send to validator's serve_repair port ──
    socket.send_to(&buf, dest).await?;
    Ok(nonce)
}

/// Extract shred bytes and nonce from a repair response packet.
///
/// Validator sends back: [shred raw bytes] + [nonce (4 bytes, bincode u32 LE)]
///
/// We split off the last 4 bytes to get the nonce for matching, and return
/// the remaining bytes as a raw shred for the pipeline.
///
/// Returns None if the packet is too short (need at least 5 bytes).
pub fn parse_repair_response(packet: &[u8]) -> Option<(&[u8], u32)> {
    if packet.len() < 5 {
        return None;
    }
    let (shred_bytes, nonce_bytes) = packet.split_at(packet.len() - 4);
    let nonce = u32::from_le_bytes(nonce_bytes.try_into().ok()?);
    Some((shred_bytes, nonce))
}

pub fn parse_response(packet: &[u8]) -> Option<Response> {
    if packet.len() == PING_BYTES && packet[..4] == [0, 0, 0, 0] {
        let mut pubkey = [0u8; 32];
        pubkey.copy_from_slice(&packet[4..36]);
        let mut token = [0u8; 32];
        token.copy_from_slice(&packet[36..68]);
        return Some(Response::Ping { token, pubkey });
    }
    if packet.len() < 5 {
        return None;
    }
    let (shred_bytes, nonce_bytes) = packet.split_at(packet.len() - 4);
    let nonce = u32::from_le_bytes(nonce_bytes.try_into().ok()?);
    Some(Response::Shred {
        bytes: shred_bytes.to_vec(),
        nonce,
    })
}

pub fn build_pong(token: &[u8; 32], our_key: &SigningKey) -> Vec<u8> {
    let hash = Sha256::new()
        .chain_update(b"SOLANA_PING_PONG")
        .chain_update(token)
        .finalize();

    let mut buf = Vec::with_capacity(PING_BYTES);
    buf.extend_from_slice(&7u32.to_le_bytes());          // enum tag for Pong
    buf.extend_from_slice(&our_key.verifying_key().to_bytes());  // our pubkey
    buf.extend_from_slice(&hash);                        // SHA256 hash
    let sig = our_key.sign(&hash);                       // sign the hash
    buf.extend_from_slice(&sig.to_bytes());              // signature
    buf
}
