use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const HEADER_LENGTH: usize = 4;
const IP_ECHO_SERVER_RESPONSE_LENGTH: usize = HEADER_LENGTH + 23;
const MAX_PORT_COUNT_PER_MESSAGE: usize = 4;

#[derive(Serialize, Deserialize, Default, Debug)]
pub struct IpEchoServerMessage {
    tcp_ports: [u16; MAX_PORT_COUNT_PER_MESSAGE],
    udp_ports: [u16; MAX_PORT_COUNT_PER_MESSAGE],
}

impl IpEchoServerMessage {
    pub fn new(tcp_ports: &[u16], udp_ports: &[u16]) -> Self {
        let mut msg = Self::default();
        msg.tcp_ports[..tcp_ports.len()].copy_from_slice(tcp_ports);
        msg.udp_ports[..udp_ports.len()].copy_from_slice(udp_ports);
        msg
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IpEchoServerResponse {
    pub address: IpAddr,
    pub shred_version: Option<u16>,
}

pub async fn get_cluster_info(entrypoint: &SocketAddr) -> Result<IpEchoServerResponse> {
    let mut stream = TcpStream::connect(entrypoint).await?;

    let msg = IpEchoServerMessage::new(&[], &[8000]);

    // 4 null bytes + serialized msg + newline
    let mut bytes = vec![0u8; HEADER_LENGTH];
    bytes.extend_from_slice(&bincode::serialize(&msg)?);
    bytes.push(b'\n');

    stream.write_all(&bytes).await?;
    stream.flush().await?;

    // read response
    let mut response = vec![0u8; IP_ECHO_SERVER_RESPONSE_LENGTH];
    stream.read_exact(&mut response).await?;

    // verify header
    if &response[..HEADER_LENGTH] != &[0u8; HEADER_LENGTH] {
        anyhow::bail!("invalid response header");
    }

    let resp: IpEchoServerResponse = bincode::deserialize(
        &response[HEADER_LENGTH..]
    )?;

    Ok(resp)
}


