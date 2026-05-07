use serde::{Deserialize, Serialize};
//use solana_sdk::pubkey::Pubkey;
use solana_sdk::{pubkey::Pubkey, timing::timestamp};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{SystemTime, UNIX_EPOCH};

const SOCKET_CACHE_SIZE: usize = 12;
const SOCKET_ADDR_UNSPECIFIED: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), /*port:*/ 0u16);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocketEntry {
    pub key: u8,
    pub index: u8,
    pub offset: u16,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
enum Extension {}

//#[derive(Debug, Clone, Serialize, Deserialize)]
/**
*
* pub struct Version {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
    pub commit: Option<u32>,
    pub feature_set: u32,
    pub client: u16,
}

impl Default for Version {
    fn default() -> Self {
        Self {
            major: 2,
            minor: 0,
            patch: 0,
            commit: None,
            feature_set: 0,
            client: 3, // Agave
        }
    }
}
*
*
*
*/

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactInfo {
    pub pubkey: Pubkey,
    pub wallclock: u64,
    pub outset: u64, // fixed spelling from "unset"
    pub shred_version: u16,
    pub version: solana_version::Version,
    pub addrs: Vec<IpAddr>,
    pub sockets: Vec<SocketEntry>,
    pub extensions: Vec<Extension>,
    pub cache: [SocketAddr; SOCKET_CACHE_SIZE],
}

fn unix_timestamp_micros() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_micros()
        .try_into()
        .expect("unix timestamp micros should fit in u64")
}

impl ContactInfo {
    pub fn new(pubkey: Pubkey, wallclock: u64, gossip: SocketAddr, shred_version: u16) -> Self {
        let ip = gossip.ip();
        let port = gossip.port();

        Self {
            pubkey,
            wallclock,
            outset: unix_timestamp_micros(),
            shred_version,
            version: solana_version::Version::default(),
            addrs: vec![ip],
            sockets: vec![SocketEntry {
                key: 0,   // 0 = gossip
                index: 0, // first IP in addrs
                offset: port,
            }],
            extensions: vec![],
            cache: [SOCKET_ADDR_UNSPECIFIED; SOCKET_CACHE_SIZE],
        }
    }

    pub fn pubkey(&self) -> &Pubkey {
        &self.pubkey
    }

    pub fn sockets(&self) -> &Vec<SocketEntry> {
        &self.sockets
    }

    pub fn default() -> Self {
        Self {
            pubkey: Pubkey::new_unique(),
            wallclock: timestamp(),
            outset: unix_timestamp_micros(),
            shred_version: 0,
            version: solana_version::Version::default(),
            addrs: Vec::<IpAddr>::default(),
            sockets: vec![],
            extensions: Vec::<Extension>::default(),
            cache: [SOCKET_ADDR_UNSPECIFIED; SOCKET_CACHE_SIZE],
        }
    }
}
