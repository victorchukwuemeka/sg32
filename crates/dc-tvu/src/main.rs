use tokio::net::UdpSocket;
use dc_tvu::shred::Shred;
use dc_tvu::shred_header::*;

const PACKET_DATA_SIZE:usize = 1232;
const SIZE_OF_CODING_SHRED_HEADER:usize  = 89;
const VARIANT_BIT_DATA:u8 = 0x80;


#[tokio::main]
async fn main()->anyhow::Result<()>{
    let socket = UdpSocket::bind("0.0.0.0:8003").await?;
    println!("Listening on 0.0.0.0:8003");
    let mut buf = vec![0u8; PACKET_DATA_SIZE];

    loop{
        let(len, peer) = socket.recv_from(&mut buf).await?;
        let packet = &buf[..len];
        if packet.len() < 89{
            println!("the bytes:{} are too short  and  we got it from {}" , packet.len(), peer);

        }
        match Shred::parse_from_bytes(packet){
            Some(shred)=>{
                let typ = if shred.shred_type() == ShredType::Data {"DATA"} else {"CODE "};
                   println!(
                    "{} slot={} index={} fec={} from {}",
                    typ,
                    shred.slot(),
                    shred.index(),
                    shred.erasure_set_id().fec_set_index,
                    peer,
                );
            }
            None => println!("parse failed from {}", peer),
        }


    }

}