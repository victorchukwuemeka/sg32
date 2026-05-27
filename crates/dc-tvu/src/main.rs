use dc_gossip::contact_info::ContactInfo;
use dc_gossip::run_gossip_loop;
use dc_tvu::repair::{build_pong, parse_response, send_repair_request, Response};
use dc_tvu::shred::Shred;
use dc_tvu::shred_header::*;
use rand::RngCore;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use dc_tvu::fec_batch::FecBatch;
use dc_tvu::flat_file_store::FlatFileStore;
use dc_tvu::merkle_prover::MerkleTree;
use dc_tvu::reed_solomon::{NUM_CODE_SHREDS, NUM_DATA_SHREDS};
use dc_tvu::ring_buffer::{SlotData, SlotRingBuffer};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

const PACKET_DATA_SIZE: usize = 1232;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut secret = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut secret);
    let our_key = ed25519_dalek::SigningKey::from_bytes(&secret);
    println!("Our pubkey: {:?}", our_key.verifying_key().to_bytes());

    let socket = UdpSocket::bind("0.0.0.0:8003").await?;
    println!("Listening on 0.0.0.0:8003");

    let latest_slot = Arc::new(AtomicU64::new(0));
    let latest_slot_clone = latest_slot.clone();
    let (gossip_tx, mut gossip_rx) = mpsc::channel::<ContactInfo>(1000);
    tokio::spawn(async move {
        if let Err(e) = run_gossip_loop(gossip_tx, latest_slot_clone).await {
            eprintln!("gossip error: {e}");
        }
    });

    let mut batches: HashMap<ErasureSetId, FecBatch> = HashMap::new();
    let mut ring_buffer = SlotRingBuffer::new(500);
    let mut file_store = FlatFileStore::new("data".into())?;
    let mut buf = vec![0u8; PACKET_DATA_SIZE];

    println!("Waiting for gossip peers...");

    let mut last_print = std::time::Instant::now();

    loop {
        tokio::select! {
            Some(ci) = gossip_rx.recv() => {
                let serve_repair = ci.socket_by_key(4);
                if let Some(addr) = serve_repair {
                    let pk: [u8; 32] = ci.pubkey().to_bytes();
                    let slot = latest_slot.load(Ordering::Relaxed);
                    println!("-> Validator {:?} slot={} serve_repair={}",
                        &pk[..4], slot, addr);
                    for idx in 0..NUM_DATA_SHREDS as u64 {
                        match send_repair_request(&socket, addr, &our_key, &pk, slot, idx).await {
                            Ok(nonce) => println!("<- sent req (nonce={}, idx={}) to {}", nonce, idx, addr),
                            Err(e) => eprintln!("repair failed (idx={}): {e}", idx),
                        }
                    }
                }
            }

            Ok((len, peer)) = socket.recv_from(&mut buf) => {
                let packet = &buf[..len];
                match parse_response(packet) {
                    Some(Response::Ping { token, pubkey }) => {
                        let slot = latest_slot.load(Ordering::Relaxed);
                        println!("Ping from {:?} (len={}) — sending Pong (slot={})",
                            &pubkey[..4], len, slot);
                        let pong = build_pong(&token, &our_key);
                        match socket.send_to(&pong, peer).await {
                            Ok(n) => println!("Pong sent ({} bytes) to {}", n, peer),
                            Err(e) => eprintln!("Pong send failed: {e}"),
                        }
                        for idx in 0..NUM_DATA_SHREDS as u64 {
                            match send_repair_request(&socket, peer, &our_key, &pubkey, slot, idx).await {
                                Ok(nonce) => println!("Re-sent req (nonce={}, idx={}) to {}", nonce, idx, peer),
                                Err(e) => eprintln!("re-send failed (idx={}): {e}", idx),
                            }
                        }
                    }
                    Some(Response::Shred { bytes, nonce }) => {
                        println!("SHRED! len={} nonce={} from {}", bytes.len(), nonce, peer);
                        match Shred::parse_from_bytes(&bytes) {
                            Some(shred) => {
                                let batch_id = shred.erasure_set_id();
                                let num_data = match &shred {
                                    Shred::MerkleCode { coding_header, .. } => coding_header.num_data_shreds as usize,
                                    _ => NUM_DATA_SHREDS,
                                };
                                let num_code = match &shred {
                                    Shred::MerkleCode { coding_header, .. } => coding_header.num_coding_shreds as usize,
                                    _ => NUM_CODE_SHREDS,
                                };
                                let batch = batches.entry(batch_id).or_insert_with(|| {
                                    FecBatch::new(batch_id.slot, batch_id.fec_set_index, num_data, num_code)
                                });
                                match &shred {
                                    Shred::MerkleData { common_header, data, .. } => {
                                        let data_index = common_header.index - common_header.fec_set_index;
                                        batch.add_data_shred(data_index, data.clone());
                                    }
                                    Shred::MerkleCode { coding_header, code, .. } => {
                                        batch.add_code_shred(coding_header.position.into(), code.clone());
                                    }
                                }
                                println!("slot={} fec={} data={}/{} code={}/{} total={}",
                                    shred.slot(), batch_id.fec_set_index,
                                    batch.data_shreds.iter().filter(|s| s.is_some()).count(), batch.num_data,
                                    batch.code_shreds.iter().filter(|s| s.is_some()).count(), batch.num_code,
                                    batch.received_count(),
                                );
                                if let Some(recovered) = batch.try_recover() {
                                    println!("★★★ RECOVERED {} shreds! ★★★", recovered.len());
                                    let all_entries = recovered.concat();
                                    let tree = MerkleTree::new(&recovered);
                                    let slot_data = SlotData {
                                        slot: batch.slot, parent_slot: 0,
                                        entries: all_entries.clone(),
                                        num_transactions: recovered.len(),
                                        merkle_root: Some(tree.root),
                                    };
                                    ring_buffer.put(slot_data);
                                    let _ = file_store.save_slot(batch.slot, &all_entries);
                                    batches.remove(&batch_id);
                                }
                            }
                            None => println!("parse failed from {}", peer),
                        }
                    }
                    None => println!("unknown/too-short (len={}) from {}", len, peer),
                }
            }
        }

        if last_print.elapsed() >= std::time::Duration::from_secs(10) {
            let slot = latest_slot.load(Ordering::Relaxed);
            println!("[STATUS] latest_slot={} peers_in_gossip={} pending_batches={}",
                slot, 0, batches.len());
            last_print = std::time::Instant::now();
        }
    }
}
