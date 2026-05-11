use serde::{Deserialize, Deserializer, Serialize};
use solana_sdk::{pubkey::Pubkey, timing::timestamp};
use solana_serde_varint as serde_varint;
use solana_short_vec as short_vec;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{SystemTime, UNIX_EPOCH};

const SOCKET_CACHE_SIZE: usize = 14;
const SOCKET_ADDR_UNSPECIFIED: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), /*port:*/ 0u16);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocketEntry {
    pub key: u8,
    pub index: u8,
    #[serde(with = "serde_varint")]
    pub offset: u16,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
enum Extension {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Version {
    #[serde(with = "serde_varint")]
    pub major: u16,
    #[serde(with = "serde_varint")]
    pub minor: u16,
    #[serde(with = "serde_varint")]
    pub patch: u16,
    pub commit: u32,
    pub feature_set: u32,
    #[serde(with = "serde_varint")]
    pub client: u16,
}

impl Version {
    pub fn display(&self) -> String {
        format!("{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl Default for Version {
    fn default() -> Self {
        Self {
            major: 2,
            minor: 0,
            patch: 0,
            commit: 0,
            feature_set: 0,
            client: 3, // Agave
        }
    }
}

// Agave uses a custom Deserialize: deserialize without cache field,
// then populate cache from socket entries (cumulative port offsets).
#[derive(Deserialize)]
struct ContactInfoLite {
    pubkey: Pubkey,
    #[serde(with = "serde_varint")]
    wallclock: u64,
    outset: u64,
    shred_version: u16,
    version: Version,
    #[serde(with = "short_vec")]
    addrs: Vec<IpAddr>,
    #[serde(with = "short_vec")]
    sockets: Vec<SocketEntry>,
    #[serde(with = "short_vec")]
    extensions: Vec<Extension>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContactInfo {
    pub pubkey: Pubkey,
    #[serde(with = "serde_varint")]
    pub wallclock: u64,
    pub outset: u64,
    pub shred_version: u16,
    pub version: Version,
    #[serde(with = "short_vec")]
    pub addrs: Vec<IpAddr>,
    #[serde(with = "short_vec")]
    pub sockets: Vec<SocketEntry>,
    #[serde(with = "short_vec")]
    pub extensions: Vec<Extension>,
    #[serde(skip_serializing)]
    pub cache: [SocketAddr; SOCKET_CACHE_SIZE],
}

impl<'de> Deserialize<'de> for ContactInfo {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let lite = ContactInfoLite::deserialize(deserializer)?;
        let mut cache = [SOCKET_ADDR_UNSPECIFIED; SOCKET_CACHE_SIZE];
        let mut port = 0u16;
        for entry in &lite.sockets {
            port = port.wrapping_add(entry.offset);
            if let Some(cached) = cache.get_mut(usize::from(entry.key)) {
                if let Some(addr) = lite.addrs.get(usize::from(entry.index)) {
                    *cached = SocketAddr::new(*addr, port);
                }
            }
        }
        Ok(Self {
            pubkey: lite.pubkey,
            wallclock: lite.wallclock,
            outset: lite.outset,
            shred_version: lite.shred_version,
            version: lite.version,
            addrs: lite.addrs,
            sockets: lite.sockets,
            extensions: lite.extensions,
            cache,
        })
    }
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
            version: Version::default(),
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

    pub fn gossip_addr(&self) -> Option<SocketAddr> {
        self.socket_by_key(0)
    }

    pub fn socket_by_key(&self, key: u8) -> Option<SocketAddr> {
        let mut port = 0u16;
        for entry in &self.sockets {
            port = port.checked_add(entry.offset)?;
            if entry.key == key {
                let ip = self.addrs.get(entry.index as usize)?;
                return Some(SocketAddr::new(*ip, port));
            }
        }
        None
    }

    pub fn socket_addr_or_none(&self, key: u8) -> String {
        self.socket_by_key(key)
            .map(|a| a.port().to_string())
            .unwrap_or_else(|| "none".into())
    }

    pub fn table_row(&self) -> String {
        let age_ms = (unix_timestamp_micros() / 1000).saturating_sub(self.wallclock / 1000);
        format!(
            "  {:>18} | {:>5} | {:45} | {:7} | {:>4} | {:>7} | {:>4} | {:>5} | {:>4} | {:>5} | {:>5} | {:>7}",
            self.addrs.first().map(|a| a.to_string()).unwrap_or_default(),
            age_ms,
            self.pubkey,
            self.version.display(),
            self.socket_addr_or_none(0),
            self.socket_addr_or_none(9),
            self.socket_addr_or_none(5),
            self.socket_addr_or_none(6),
            self.socket_addr_or_none(10),
            self.socket_addr_or_none(11),
            self.socket_addr_or_none(4),
            self.shred_version,
        )
    }

    pub fn header() -> String {
        "  {:>18} | {:>5} | {:45} | {:7} | {:>4} | {:>7} | {:>4} | {:>5} | {:>4} | {:>5} | {:>5} | {:>7}".into()
    }

    pub fn default() -> Self {
        Self {
            pubkey: Pubkey::new_unique(),
            wallclock: timestamp(),
            outset: unix_timestamp_micros(),
            shred_version: 0,
            version: Version::default(),
            addrs: Vec::<IpAddr>::default(),
            sockets: vec![],
            extensions: Vec::<Extension>::default(),
            cache: [SOCKET_ADDR_UNSPECIFIED; SOCKET_CACHE_SIZE],
        }
    }
}
