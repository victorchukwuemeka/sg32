use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};

#[derive(Serialize, Deserialize, Debug)]
pub struct IpEchoServerMessage {
    pub tcp_ports: [u16; 4],
    pub udp_ports: [u16; 4],
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IpEchoServerResponse {
    pub address: IpAddr,
    pub shred_version: u16,
}

pub fn get_cluster_info(entrypoint: &SocketAddr) -> Result<IpEchoServerResponse> {
    let mut stream = TcpStream::connect(entrypoint)?;
    let request = IpEchoServerMessage {
        tcp_ports: [0u16; 4],
        udp_ports: [8001, 0, 0, 0],
    };
    let bytes = bincode::serialize(&request)?;
    stream.write_all(&bytes)?;

    let mut response_buf = vec![0u8; 1024];
    let bytes_read = stream.read(&mut response_buf)?;
    let result: IpEchoServerResponse = bincode::deserialize(&response_buf[..bytes_read])?;
    Ok(result)
}
