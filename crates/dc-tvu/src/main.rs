use dc_gossip::contact_info::ContactInfo;
use dc_gossip::run_gossip_loop;
use dc_tvu::repair::{parse_repair_response, send_repair_request};
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

const PACKET_DATA_SIZE: usize = 1232;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut secret = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut secret);
    let our_key = ed25519_dalek::SigningKey::from_bytes(&secret);
    println!("Our pubkey: {:?}", our_key.verifying_key().to_bytes());

    let socket = UdpSocket::bind("0.0.0.0:8003").await?;
    println!("Listening on 0.0.0.0:8003");

    let (gossip_tx, mut gossip_rx) = mpsc::channel::<ContactInfo>(1000);
    tokio::spawn(async move {
        if let Err(e) = run_gossip_loop(gossip_tx).await {
            eprintln!("gossip error: {e}");
        }
    });

    let mut batches: HashMap<ErasureSetId, FecBatch> = HashMap::new();
    let mut ring_buffer = SlotRingBuffer::new(500);
    let mut file_store = FlatFileStore::new("data".into())?;
    let mut buf = vec![0u8; PACKET_DATA_SIZE];

    // TODO: get live slot from gossip Votes or RPC
    let target_slot: u64 = 0;

    println!("Waiting for gossip peers...");

    loop {
        tokio::select! {
            Some(ci) = gossip_rx.recv() => {
                let serve_repair = ci.socket_by_key(4);
                if let Some(addr) = serve_repair {
                    let pk: [u8; 32] = ci.pubkey().to_bytes();
                    println!("Validator: {:?} serve_repair={}", &pk[..4], addr);
                    match send_repair_request(
                        &socket, addr, &our_key, &pk, target_slot, 0,
                    ).await {
                        Ok(nonce) => println!("Repair req sent (nonce={}) to {}", nonce, addr),
                        Err(e) => eprintln!("repair failed: {e}"),
                    }
                }
            }

            Ok((len, peer)) = socket.recv_from(&mut buf) => {
                let packet = &buf[..len];
                let Some((shred_bytes, _nonce)) = parse_repair_response(packet) else {
                    println!("short packet from {}", peer);
                    continue;
                };
                match Shred::parse_from_bytes(shred_bytes) {
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
        }
    }
}
