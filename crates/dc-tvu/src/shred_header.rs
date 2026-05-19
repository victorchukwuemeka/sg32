use serde::{Deserialize, Serialize};

// =============================================================================
//  WHY BITS AND HEX?
//  =================
//  Computers store everything as bytes. A byte is 8 bits. Each bit is a single
//  "on" (1) or "off" (0) switch.
//
//  We could store each value in its own byte — like 1 byte for "data or code?",
//  1 byte for "proof size", 1 byte for "resigned?". That wastes space.
//
//  Solana shreds are UDP packets. UDP has a limited size (about 1500 bytes per
//  packet including headers). Every byte counts. So Solana packs multiple small
//  values into a single byte using bit manipulation.
//
//  That's what the shred_variant byte (offset 64) does — it stores 3 things in
//  1 byte instead of 3 bytes:
//    - Is this Data or Code?          (1 bit needed — 2 possibilities)
//    - How many Merkle proof entries? (4 bits needed — values 0-15)
//    - Is it resigned?                 (1 bit needed — true/false)
//
//  HEX NOTATION
//  ============
//  Binary numbers like 0b10110011 are hard to read. Hex (base-16) is a
//  shorthand: every 4 bits = 1 hex digit. So 0b1011_0011 = 0xB3.
//  Hex digits: 0-9 = 0-9, A=10, B=11, C=12, D=13, E=14, F=15.
//
//  Converting hex to decimal: each position is a power of 16.
//    0xB3  =  B(=11) × 16  +  3 × 1  =  176 + 3  =  179
//
//  COMMON PATTERNS
//  ===============
//    0x0F  =  0b0000_1111  =  15  (lower 4 bits = 1, upper = 0)
//    0xF0  =  0b1111_0000  =  240 (upper 4 bits = 1, lower = 0)
//    These are "masks" — they let us isolate parts of a byte.
//    byte & 0x0F  =  keep lower nibble, zero out upper nibble
//    byte & 0xF0  =  keep upper nibble, zero out lower nibble
// =============================================================================

// ── Constants ────────────────────────────────────────────────────────────────
// These define the sizing of everything. PACKET_DATA_SIZE (1232) comes from:
//   1500 (standard MTU for Ethernet)
//   - 20 (IP header)
//   - 8  (UDP header)
//   - 4  (optional nonce for repair packets)
//   = 1468 ... actually Solana chose 1232 for data and 1228 for coding.
//   The key is: every shred fits in one UDP packet.

pub const PACKET_DATA_SIZE: usize = 1232;
pub const SIZE_OF_SIGNATURE: usize = 64; // ed25519 signature = 64 bytes
pub const SIZE_OF_COMMON_HEADER: usize = 83; // common across data+code shreds
pub const SIZE_OF_DATA_HEADER: usize = 5; // data-specific header (common+data = 88 total)
pub const SIZE_OF_CODING_HEADER: usize = 6; // coding-specific header (common+coding = 89 total)
pub const SIZE_OF_DATA_SHRED: usize = 1203; // exact byte length of every data shred payload
pub const SIZE_OF_CODING_SHRED: usize = 1228; // exact byte length of every coding shred payload
pub const DATA_SHREDS_PER_FEC_BLOCK: usize = 32; // how many data shreds in one FEC batch
pub const CODING_SHREDS_PER_FEC_BLOCK: usize = 32; // how many coding shreds in one FEC batch
pub const SHREDS_PER_FEC_BLOCK: usize = 64; // total (32+32)
pub const SIZE_OF_MERKLE_ROOT: usize = 32; // SHA-256 hash
pub const SIZE_OF_MERKLE_PROOF_ENTRY: usize = 20; // each Merkle proof entry
pub const MAX_DATA_SHREDS_PER_SLOT: u32 = 32768;
pub const MAX_CODE_SHREDS_PER_SLOT: u32 = 32768;

// ── ShredType ────────────────────────────────────────────────────────────────
// WHY: Every shred is either Data (contains transaction bytes) or Code (contains
// Reed-Solomon parity bytes that let us recover lost data shreds).
//
// The specific byte values 0xA5 (165) and 0x5A (90) are bitwise complements:
//   flip all bits in 0xA5 and you get 0x5A.
// This is a wire protocol trick — if a single bit flips due to noise, you won't
// accidentally confuse Data for Code. You'll get an invalid value instead.
//
// Note: This ShredType enum is a tag used elsewhere in the Solana protocol.
// The actual byte at offset 64 in a shred is NOT 0xA5 or 0x5A — it's the
// ShredVariant byte which encodes MORE information (see below). ShredVariant
// has its own encoding scheme (0x9_, 0xB_, 0x6_, 0x7_).

#[derive(Debug, Clone, Copy, PartialEq, Hash, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ShredType {
    Data = 0b1010_0101, // = 0xA5 = 165
    Code = 0b0101_1010, // = 0x5A = 90
}

// ── ShredVariant ─────────────────────────────────────────────────────────────
// WHY: The byte at offset 64 of every shred must tell us 3 things:
//   1. Is this Data or Code?
//   2. How many Merkle proof entries does this shred have?
//   3. Does this shred have a retransmitter signature at the end?
//
// If we used a byte for each, that's 3 bytes of overhead per shred. With
// hundreds of thousands of shreds per slot, that adds up. Solana packs all 3
// into 1 byte.
//
// HOW: Split the byte into two halves (called nibbles):
//
//     Bits:   7  6  5  4      3  2  1  0
//            ┌──────────┐  ┌──────────────┐
//            │ UPPER    │  │ LOWER        │
//            │ NIBBLE   │  │ NIBBLE       │
//            │ (variant)│  │ (proof_size) │
//            └──────────┘  └──────────────┘
//
//   Upper nibble (bits 7-4): determines the variant type.
//     This single 4-bit value encodes BOTH data/code AND resigned.
//     There are exactly 4 valid values:
//
//     Hex    Binary    Meaning
//     ────   ──────    ───────
//     0x6_   0110____  Code shred, NOT resigned
//     0x7_   0111____  Code shred, IS resigned
//     0x9_   1001____  Data shred, NOT resigned
//     0xB_   1011____  Data shred, IS resigned
//
//   Lower nibble (bits 3-0): the proof_size (0-15).
//     Tells us how many Merkle proof entries are in this shred.
//     For a full FEC batch (32+32=64 shreds), proof_size = 6 (log2(64) = 6).
//
// So to decode the byte at offset 64:
//   1. Look at the upper nibble → which variant (Data/Code + resigned)?
//   2. Look at the lower nibble → how many Merkle proof entries?
//
// Using masks (bitwise AND with a "stencil"):
//   byte & 0xF0  =  zero out lower nibble, keep upper → tells us the variant
//   byte & 0x0F  =  zero out upper nibble, keep lower → tells us proof_size

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShredVariant {
    MerkleData { proof_size: u8, resigned: bool },
    MerkleCode { proof_size: u8, resigned: bool },
}

impl ShredVariant {
    // Takes a raw byte (0-255) and decodes it into a ShredVariant.
    //
    // Step 1: Extract proof_size from the lower nibble using a mask.
    //   byte & 0x0F keeps only bits 3-0, zeros out bits 7-4.
    //   Example: byte=0xB3, 0xB3 & 0x0F = 0x03 = proof_size=3.
    //   Think of & 0x0F as a stencil that blocks the upper half.
    //
    // Step 2: Check the upper nibble (byte & 0xF0) against known patterns.
    //   Example: 0xB3 & 0xF0 = 0xB0, which matches MerkleData+resigned.

    pub fn from_u8(byte: u8) -> Option<Self> {
        // Lower nibble = proof_size (0-15).
        // Example with byte 181 (0xB5):
        //   0xB5 = 1011 0101
        //   0x0F = 0000 1111
        //   AND  = 0000 0101 = 5 decimal
        let proof_size = byte & 0x0F;

        // Upper nibble determines the variant type.
        match byte & 0xF0 {
            // 0x90 = 144 decimal = 1001 0000 in binary
            // Upper nibble 1001 = MerkleData, not resigned.
            // Example: byte 150 → 150 & 0xF0 = 144 = 0x90 → Data, not resigned
            0x90 => Some(ShredVariant::MerkleData {
                proof_size,
                resigned: false,
            }),
            // 0xB0 = 176 decimal = 1011 0000 in binary
            // Upper nibble 1011 = MerkleData, resigned.
            // Example: byte 181 → 181 & 0xF0 = 176 = 0xB0 → Data, resigned
            0xB0 => Some(ShredVariant::MerkleData {
                proof_size,
                resigned: true,
            }),
            // 0x60 = 96 decimal = 0110 0000 in binary
            // Upper nibble 0110 = MerkleCode, not resigned.
            0x60 => Some(ShredVariant::MerkleCode {
                proof_size,
                resigned: false,
            }),
            // 0x70 = 112 decimal = 0111 0000 in binary
            // Upper nibble 0111 = MerkleCode, resigned.
            0x70 => Some(ShredVariant::MerkleCode {
                proof_size,
                resigned: true,
            }),
            // Any other upper nibble value is invalid.
            _ => None,
        }
    }

    // The reverse: convert a ShredVariant back to a byte.
    // The | (OR) operator combines the upper nibble (variant type) with the
    // lower nibble (proof_size). OR works like addition at the bit level —
    // each bit is 1 if either input has 1.
    // Example: MerkleData{proof_size:5, resigned:true}
    //   0xB0 = 1011 0000
    //   | 5  = 0000 0101
    //   result = 1011 0101 = 0xB5

    pub fn to_u8(&self) -> u8 {
        match self {
            ShredVariant::MerkleData {
                proof_size,
                resigned: false,
            } => 0x90 | proof_size,
            ShredVariant::MerkleData {
                proof_size,
                resigned: true,
            } => 0xB0 | proof_size,
            ShredVariant::MerkleCode {
                proof_size,
                resigned: false,
            } => 0x60 | proof_size,
            ShredVariant::MerkleCode {
                proof_size,
                resigned: true,
            } => 0x70 | proof_size,
        }
    }

    pub fn shred_type(&self) -> ShredType {
        match self {
            ShredVariant::MerkleData { .. } => ShredType::Data,
            ShredVariant::MerkleCode { .. } => ShredType::Code,
        }
    }
}

// ── ShredFlags ───────────────────────────────────────────────────────────────
// WHY: The byte at offset 85 in a DATA shred tells us two things:
//   1. What tick within the slot this shred was created at (6 bits, values 0-63)
//   2. Whether this is the last data shred in the slot
//   3. Whether data is complete
//
// Again, all packed into 1 byte to save space.
//
// Byte layout:
//   Bit:    7         6         5   4   3   2   1   0
//         ┌─────────┬─────────┬────────────────────────┐
//         │LAST     │DATA     │ REFERENCE_TICK         │
//         │SHRED    │COMPLETE │ (6 bits, 0-63)         │
//         └─────────┴─────────┴────────────────────────┘
//
//   LAST_SHRED_IN_SLOT (bit 7 + bit 6): 0b1100_0000
//     NOTE: LAST_SHRED_IN_SLOT always implies DATA_COMPLETE_SHRED too.
//     That's why its bitmask has BOTH bit 7 and bit 6 set.
//     The bitflags crate handles this automatically.
//
//   DATA_COMPLETE_SHRED (bit 6 alone): 0b0100_0000
//
//   SHRED_TICK_REFERENCE_MASK (bits 0-5): 0b0011_1111
//     The reference_tick saturates at 63 (max value for 6 bits).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShredFlags(u8);

impl ShredFlags {
    // The tick reference mask uses the lowest 6 bits (0b0011_1111 = 63).
    // This means reference_tick can only be 0-63. If the actual tick is
    // higher, it saturates at 63.
    pub const TICK_MASK: u8 = 0b0011_1111;

    // Data complete: bit 6 set. Value = 64 decimal = 0b0100_0000.
    pub const DATA_COMPLETE: u8 = 0b0100_0000;

    // Last shred: bits 7+6 set. Value = 192 decimal = 0b1100_0000.
    // Having bit 7 set automatically means bit 6 is also set, which implies
    // that the last shred is also a data-complete shred.
    pub const LAST_SHRED: u8 = 0b1100_0000;

    pub fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    pub fn bits(&self) -> u8 {
        self.0
    }

    // contains() checks if specific flag bits are set.
    // Example: if self.0 = 0b1100_0101 and we check LAST_SHRED (0b1100_0000):
    //   0b1100_0101 & 0b1100_0000 = 0b1100_0000
    //   0b1100_0000 == 0b1100_0000 → true
    // So this shred IS the last in the slot.
    pub fn contains(&self, flag: u8) -> bool {
        self.0 & flag == flag
    }

    // Extracts just the reference tick (bits 0-5) by masking away bits 6-7.
    pub fn reference_tick(&self) -> u8 {
        self.0 & Self::TICK_MASK
    }

    pub fn is_data_complete(&self) -> bool {
        self.contains(Self::DATA_COMPLETE)
    }

    pub fn is_last_in_slot(&self) -> bool {
        self.contains(Self::LAST_SHRED)
    }
}

// ── Common Header ────────────────────────────────────────────────────────────
// WHY: Every shred (Data OR Code) starts with these 83 bytes.
// Together they identify which slot the shred belongs to, where in the slot
// it goes (index), what cluster it's for (version), and who signed it
// (signature — the leader's ed25519 signature over the Merkle root).
//
// All multibyte values are little-endian (LE), meaning the least significant
// byte comes first. This is the standard for x86 processors.
//
// Byte layout (offsets from start of payload):
//
//   Offset  Size  Field          Description
//   ──────  ────  ─────          ───────────
//   0       64    signature      ed25519 signature of the Merkle root
//   64      1     shred_variant  Encoded type + proof_size + resigned
//   65      8     slot           Which slot this shred belongs to
//   73      4     index          Shred position within the slot
//   77      2     version        Cluster identifier (e.g. 11016 for devnet)
//   79      4     fec_set_index  Index of 1st data shred in this FEC batch

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ShredCommonHeader {
    #[serde(with = "serde_bytes")]
    pub signature: [u8; 64], // offset 0
    pub shred_variant: u8,  // offset 64
    pub slot: u64,          // offset 65
    pub index: u32,         // offset 73
    pub version: u16,       // offset 77
    pub fec_set_index: u32, // offset 79
}

// ── Data Header ──────────────────────────────────────────────────────────────
// WHY: Only Data shreds have these extra 5 bytes. They tell us:
//   - parent_offset: which slot this block descends from (parent = slot - offset)
//   - flags: reference tick + completeness markers (see ShredFlags above)
//   - size: total bytes from start of shred to end of data (used to find
//           where the actual entry bytes end versus padding)
//
// Byte layout (offset 83):
//
//   Offset  Size  Field           Description
//   ──────  ────  ─────           ───────────
//   83      2     parent_offset   parent_slot = slot - parent_offset
//   85      1     flags           ShredFlags (tick reference + complete/last)
//   86      2     size            Total data size including headers

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DataShredHeader {
    pub parent_offset: u16, // offset 83
    pub flags: u8,          // offset 85
    pub size: u16,          // offset 86
}

// ── Coding Header ────────────────────────────────────────────────────────────
// WHY: Only Code shreds have these extra 6 bytes. They describe the FEC batch
// dimensions so the Reed-Solomon decoder knows how to reconstruct data.
//   - num_data_shreds: how many data shreds in this batch (usually 32)
//   - num_coding_shreds: how many coding shreds (usually 32)
//   - position: this coding shred's position within the batch (0 to N-1)
//
// Byte layout (offset 83):
//
//   Offset  Size  Field             Description
//   ──────  ────  ─────             ───────────
//   83      2     num_data_shreds   How many data shreds in FEC batch
//   85      2     num_coding_shreds  How many coding shreds in FEC batch
//   87      2     position           This coding shred's position (0-indexed)

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CodingShredHeader {
    pub num_data_shreds: u16,   // offset 83
    pub num_coding_shreds: u16, // offset 85
    pub position: u16,          // offset 87
}

// ── Wire Helpers ─────────────────────────────────────────────────────────────
// WHY: Sometimes we just need one field from a shred without the cost of
// full deserialization (e.g., to decide whether to discard a packet).
// These helpers read bytes directly from the payload slice by offset.
//
// Each helper takes a byte slice (the raw shred payload) and extracts the
// bytes at the known offset, converting from little-endian to the CPU's
// native integer format.

pub fn get_signature(bytes: &[u8]) -> Option<[u8; 64]> {
    let slice = bytes.get(0..64)?;
    let mut arr = [0u8; 64];
    arr.copy_from_slice(slice);
    Some(arr)
}

pub fn get_shred_variant(bytes: &[u8]) -> Option<ShredVariant> {
    let byte = *bytes.get(64)?;
    ShredVariant::from_u8(byte)
}

pub fn get_shred_type(bytes: &[u8]) -> Option<ShredType> {
    get_shred_variant(bytes).map(|v| v.shred_type())
}

pub fn is_data(bytes: &[u8]) -> Option<bool> {
    get_shred_type(bytes).map(|t| t == ShredType::Data)
}

pub fn is_code(bytes: &[u8]) -> Option<bool> {
    get_shred_type(bytes).map(|t| t == ShredType::Code)
}

pub fn get_slot(bytes: &[u8]) -> Option<u64> {
    let slice = bytes.get(65..73)?;
    Some(u64::from_le_bytes(slice.try_into().ok()?))
}

pub fn get_index(bytes: &[u8]) -> Option<u32> {
    let slice = bytes.get(73..77)?;
    Some(u32::from_le_bytes(slice.try_into().ok()?))
}

pub fn get_version(bytes: &[u8]) -> Option<u16> {
    let slice = bytes.get(77..79)?;
    Some(u16::from_le_bytes(slice.try_into().ok()?))
}

pub fn get_fec_set_index(bytes: &[u8]) -> Option<u32> {
    let slice = bytes.get(79..83)?;
    Some(u32::from_le_bytes(slice.try_into().ok()?))
}

// Data-shred-specific helpers (call these only for data shreds)

pub fn get_parent_offset(bytes: &[u8]) -> Option<u16> {
    let slice = bytes.get(83..85)?;
    Some(u16::from_le_bytes(slice.try_into().ok()?))
}

pub fn get_flags(bytes: &[u8]) -> Option<ShredFlags> {
    let byte = *bytes.get(85)?;
    Some(ShredFlags::from_bits(byte))
}

pub fn get_data_size(bytes: &[u8]) -> Option<u16> {
    let slice = bytes.get(86..88)?;
    Some(u16::from_le_bytes(slice.try_into().ok()?))
}

// Coding-shred-specific helpers (call these only for coding shreds)

pub fn get_num_data_shreds(bytes: &[u8]) -> Option<u16> {
    let slice = bytes.get(83..85)?;
    Some(u16::from_le_bytes(slice.try_into().ok()?))
}

pub fn get_num_coding_shreds(bytes: &[u8]) -> Option<u16> {
    let slice = bytes.get(85..87)?;
    Some(u16::from_le_bytes(slice.try_into().ok()?))
}

pub fn get_coding_position(bytes: &[u8]) -> Option<u16> {
    let slice = bytes.get(87..89)?;
    Some(u16::from_le_bytes(slice.try_into().ok()?))
}

// ── ShredId ──────────────────────────────────────────────────────────────────
// A unique identifier for any shred. Two shreds with the same (slot, index,
// shred_type) are considered the same shred — duplicates are discarded.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShredId {
    pub slot: u64,
    pub index: u32,
    pub shred_type: ShredType,
}

// ── ErasureSetId ─────────────────────────────────────────────────────────────
// Identifies a specific FEC batch. Used when tracking which shreds we've
// received for a batch and whether we can run erasure recovery.
// Every shred with the same (slot, fec_set_index) belongs to the same batch.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ErasureSetId {
    pub slot: u64,
    pub fec_set_index: u32,
}
