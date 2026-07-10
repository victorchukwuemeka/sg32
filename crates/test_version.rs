fn main() {
    let v = solana_version::Version::default();
    let bytes = bincode::serialize(&v).unwrap();
    println!("solana_version::Version: {} bytes", bytes.len());
    println!("Hex: {}", bytes.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" "));
    
    let lv2 = solana_version::LegacyVersion2::default();
    let lv2_bytes = bincode::serialize(&lv2).unwrap();
    println!("\nLegacyVersion2: {} bytes", lv2_bytes.len());
    println!("Hex: {}", lv2_bytes.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" "));
    
    println!("\nVersion: major={}, minor={}, patch={}, commit=0x{:08x}, feature_set=0x{:08x}",
        v.major, v.minor, v.patch, v.commit, v.feature_set);
}
