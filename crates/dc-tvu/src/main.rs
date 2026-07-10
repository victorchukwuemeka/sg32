use clap::Parser;
use dc_gossip::contact_info::ContactInfo;
use dc_gossip::run_gossip_loop;
use dc_tvu::repair::{build_pong, parse_response, send_repair_request, Response};
use dc_tvu::shred::Shred;
use dc_tvu::shred_header::*;
use rand::RngCore;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use dc_tvu::deshredder;
use dc_tvu::fec_batch::FecBatch;
use dc_tvu::flat_file_store::FlatFileStore;
use dc_tvu::merkle_prover::MerkleTree;
use dc_tvu::reed_solomon::{NUM_CODE_SHREDS, NUM_DATA_SHREDS};
use dc_tvu::ring_buffer::{SlotData, SlotRingBuffer};
use dc_tvu::rpc_server::{self, AppState};
use dc_tvu::stats;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

const PACKET_DATA_SIZE: usize = 1232;

#[derive(Parser)]
#[command(name = "sg32", about = "Solana Light Node — Shred Recovery & Trustless Merkle Proofs")]
struct Args {
    #[arg(long, default_value = "8899")]
    rpc_port: u16,

    #[arg(long, default_value = "8003")]
    repair_port: u16,

    #[arg(long, default_value = "8001")]
    gossip_port: u16,

    #[arg(long, default_value = "data")]
    data_dir: String,

    #[arg(long, default_value = "entrypoint.devnet.solana.com:8001")]
    entrypoint: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let mut secret = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut secret);
    let our_key = ed25519_dalek::SigningKey::from_bytes(&secret);
    println!("Our pubkey: {:?}", our_key.verifying_key().to_bytes());

    let repair_bind = format!("0.0.0.0:{}", args.repair_port);
    let socket = UdpSocket::bind(&repair_bind).await?;
    println!("Listening on {}", repair_bind);

    let latest_slot = Arc::new(AtomicU64::new(0));
    let latest_slot_clone = latest_slot.clone();
    let gossip_port = args.gossip_port;
    let entrypoint = args.entrypoint.clone();
    let (gossip_tx, mut gossip_rx) = mpsc::channel::<ContactInfo>(1000);
    tokio::spawn(async move {
        if let Err(e) = run_gossip_loop(gossip_tx, latest_slot_clone, gossip_port, &entrypoint).await {
            eprintln!("gossip error: {e}");
        }
    });

    let mut batches: HashMap<ErasureSetId, FecBatch> = HashMap::new();
    let ring_buffer = Arc::new(RwLock::new(SlotRingBuffer::new(500)));
    let data_dir = args.data_dir.clone();
    let file_store = Arc::new(RwLock::new(FlatFileStore::new(data_dir.clone().into())?));
    let pipeline_stats = stats::new_shared_stats();

    let state = Arc::new(AppState {
        ring_buffer: ring_buffer.clone(),
        file_store: file_store.clone(),
        stats: pipeline_stats.clone(),
    });
    let rpc_router = rpc_server::router(state);
    let rpc_addr: std::net::SocketAddr = format!("0.0.0.0:{}", args.rpc_port).parse().unwrap();
    println!("RPC server on {}", rpc_addr);
    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(rpc_addr).await.unwrap();
        axum::serve(listener, rpc_router).await.unwrap();
    });
    let mut buf = vec![0u8; PACKET_DATA_SIZE];

    println!("Waiting for gossip peers...");

    let mut validators: HashMap<[u8; 32], SocketAddr> = HashMap::new();
    let mut last_repair_poll = std::time::Instant::now();
    let mut last_print = std::time::Instant::now();
    const REPAIR_POLL_SECS: u64 = 5;

    loop {
        tokio::select! {
            Some(ci) = gossip_rx.recv() => {
                if let Some(addr) = ci.socket_by_key(4) {
                    let pk: [u8; 32] = ci.pubkey().to_bytes();
                    validators.insert(pk, addr);
                    println!("-> Validator {:?} serve_repair={} ({} tracked)",
                        &pk[..4], addr, validators.len());
                }
            }

            _ = tokio::time::sleep(Duration::from_secs(1)) => {
                let slot = latest_slot.load(Ordering::Relaxed);
                if slot > 0 && last_repair_poll.elapsed() >= Duration::from_secs(REPAIR_POLL_SECS) {
                    let count = validators.len();
                    println!("[REPAIR] polling {} validators for slot {}", count, slot);
                    for (&pk, &addr) in &validators {
                        for idx in 0..(NUM_DATA_SHREDS + NUM_CODE_SHREDS) as u64 {
                            if let Err(e) = send_repair_request(&socket, addr, &our_key, &pk, slot, idx).await {
                                eprintln!("repair failed (idx={}): {e}", idx);
                            }
                        }
                    }
                    last_repair_poll = std::time::Instant::now();
                }
            }

            Ok((len, peer)) = socket.recv_from(&mut buf) => {
                let packet = &buf[..len];
                match parse_response(packet) {
                    Some(Response::Ping { token, pubkey }) => {
                        let pong = build_pong(&token, &our_key);
                        let _ = socket.send_to(&pong, peer).await;
                        println!("Ping/Pong from {:?}", &pubkey[..4]);
                    }
                    Some(Response::Shred { bytes, nonce }) => {
                        println!("SHRED! len={} nonce={} from {}", bytes.len(), nonce, peer);
                        if let Some(shred) = Shred::parse_from_bytes(&bytes) {
                            process_shred(shred, &mut batches, &ring_buffer, &file_store, &pipeline_stats, &data_dir).await;
                        } else {
                            println!("shred parse failed from {}", peer);
                        }
                    }
                    None => {
                        if let Some(shred) = Shred::parse_from_bytes(packet) {
                            println!("TURBINE shred len={} from {}", len, peer);
                            process_shred(shred, &mut batches, &ring_buffer, &file_store, &pipeline_stats, &data_dir).await;
                        } else {
                            println!("unknown (len={}) from {}", len, peer);
                        }
                    }
                }
            }
        }

        if last_print.elapsed() >= std::time::Duration::from_secs(10) {
            let slot = latest_slot.load(Ordering::Relaxed);
            println!("[STATUS] latest_slot={} validators={} pending_batches={}",
                slot, validators.len(), batches.len());
            last_print = std::time::Instant::now();
        }
    }
}

async fn process_shred(
    shred: Shred,
    batches: &mut HashMap<ErasureSetId, FecBatch>,
    ring_buffer: &Arc<RwLock<SlotRingBuffer>>,
    file_store: &Arc<RwLock<FlatFileStore>>,
    pipeline_stats: &stats::SharedStats,
    data_dir: &str,
) {
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
    let data_count = batch.data_shreds.iter().filter(|s| s.is_some()).count();
    let code_count = batch.code_shreds.iter().filter(|s| s.is_some()).count();
    {
        let mut s = pipeline_stats.write().await;
        s.current_batch.slot = shred.slot();
        s.current_batch.fec_set_index = batch_id.fec_set_index;
        s.current_batch.data_shreds = data_count;
        s.current_batch.code_shreds = code_count;
        s.current_batch.num_data = batch.num_data;
        s.current_batch.num_code = batch.num_code;
    }
    match &shred {
        Shred::MerkleData { common_header, data_header, data, .. } => {
            let data_index = common_header.index - common_header.fec_set_index;
            batch.add_data_shred(data_index, data.clone());
            if batch.parent_slot == 0 {
                batch.parent_slot = common_header.slot.saturating_sub(data_header.parent_offset as u64);
            }
            if data_index == 0 && data_count == 0 {
                let sv = common_header.shred_variant;
                let variant_name = match sv & 0xF0 {
                    0x90 => "Data",
                    0xB0 => "Data+Resigned",
                    0x60 => "Code",
                    0x70 => "Code+Resigned",
                    _ => "Unknown",
                };
                eprintln!("[SHRED] slot={} index={} fec={} variant=0x{:02x}({}) proof={}",
                    common_header.slot, common_header.index, common_header.fec_set_index,
                    sv, variant_name, sv & 0x0f);
                eprintln!("[DATA_HEADER] parent_offset={} flags=0x{:02x} size={}",
                    data_header.parent_offset, data_header.flags, data_header.size);
                let data_off_print = SIZE_OF_COMMON_HEADER + SIZE_OF_DATA_HEADER; // 88
                let after_data_print = data_header.size as usize + if common_header.shred_variant & 0xF0 == 0xB0 { SIZE_OF_SIGNATURE } else { 0 };
                eprintln!("[DATA_OFF] data_off={} data_header.size={} data.len={} after_data_off={}",
                    data_off_print, data_header.size, data.len(), after_data_print);
            }
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
        if let Some(first_payload) = recovered.first() {
            let hex8 = first_payload.iter().take(8).map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" ");
            eprintln!("[BATCH] slot={} first_payload_len={} first8=[{}]",
                batch.slot, first_payload.len(), hex8);
        }
        if let Some(result) = deshredder::deshred_into_txs(&recovered) {
            let tree = MerkleTree::new(&result.transactions);
            let root = tree.root;
            let slot_data = SlotData {
                slot: batch.slot,
                parent_slot: batch.parent_slot,
                entries: bincode::serialize(&result.entries).unwrap_or_default(),
                num_transactions: result.transactions.len(),
                merkle_root: Some(root),
                merkle_tree: Some(Arc::new(tree)),
            };
            ring_buffer.write().await.put(slot_data);
            let concat: Vec<u8> = recovered.concat();
            let _ = file_store.write().await.save_slot(batch.slot, &concat);
            println!("   → {} txs, root={:?}",
                result.transactions.len(), &root[..4]);
            {
                let mut s = pipeline_stats.write().await;
                s.total_blocks_recovered += 1;
                s.latest_slot = batch.slot;
                s.latest_block_txs = result.transactions.len();
                s.latest_block_root = root;
                s.blocks_in_ring_buffer = ring_buffer.read().await.len();
                s.files_on_disk = std::fs::read_dir(data_dir).map(|e| e.filter_map(|e| e.ok()).filter(|e| e.path().extension().is_some_and(|x| x == "dat")).count()).unwrap_or(0);
            }
        }
        batches.remove(&batch_id);
    }
}
