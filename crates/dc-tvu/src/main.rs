use dc_tvu::shred::Shred;
use dc_tvu::shred_header::*;
use tokio::net::UdpSocket;

use dc_tvu::fec_batch::FecBatch;
use dc_tvu::reed_solomon::{NUM_CODE_SHREDS, NUM_DATA_SHREDS};
use std::collections::HashMap;

const PACKET_DATA_SIZE: usize = 1232;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:8003").await?;
    println!("Listening on 0.0.0.0:8003");
    let mut buf = vec![0u8; PACKET_DATA_SIZE];

    let mut batches: HashMap<ErasureSetId, FecBatch> = HashMap::new();

    loop {
        let (len, peer) = socket.recv_from(&mut buf).await?;
        let packet = &buf[..len];
        if packet.len() < SIZE_OF_COMMON_HEADER + SIZE_OF_CODING_HEADER {
            println!(
                "the bytes:{} are too short  and  we got it from {}",
                packet.len(),
                peer
            );
        }
        match Shred::parse_from_bytes(packet) {
            Some(shred) => {
                let batch_id = shred.erasure_set_id();
                let num_data = match &shred {
                    Shred::MerkleCode { coding_header, .. } => {
                        coding_header.num_data_shreds as usize
                    }
                    _ => NUM_DATA_SHREDS,
                };
                let num_code = match &shred {
                    Shred::MerkleCode { coding_header, .. } => {
                        coding_header.num_coding_shreds as usize
                    }
                    _ => NUM_CODE_SHREDS,
                };
                let batch = batches.entry(batch_id).or_insert_with(|| {
                    FecBatch::new(batch_id.slot, batch_id.fec_set_index, num_data, num_code)
                });
                match &shred {
                    Shred::MerkleData {
                        common_header,
                        data,
                        ..
                    } => {
                        let data_index = common_header.index - common_header.fec_set_index;
                        batch.add_data_shred(data_index, data.clone());
                    }
                    Shred::MerkleCode {
                        coding_header,
                        code,
                        ..
                    } => {
                        batch.add_code_shred(coding_header.position.into(), code.clone());
                    }
                }
                println!(
                    "slot={} fec={} data={}/{} code={}/{} total={}",
                    shred.slot(),
                    batch_id.fec_set_index,
                    batch.data_shreds.iter().filter(|s| s.is_some()).count(),
                    batch.num_data,
                    batch.code_shreds.iter().filter(|s| s.is_some()).count(),
                    batch.num_code,
                    batch.received_count(),
                );
                if let Some(recovered) = batch.try_recover() {
                    println!(
                        "★★★ RECOVERED {} shreds for slot {} batch {}! ★★★",
                        recovered.len(),
                        batch.slot,
                        batch.fec_set_index
                    );
                    batches.remove(&batch_id);
                }
            }
            None => println!("parse failed from {}", peer),
        }
    }
}
