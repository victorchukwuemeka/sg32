/// Custom serialization for compact vector length encoding.
/// Solana uses this to save bytes on the wire.
///
/// Encoding rules:
/// 0-127       → 1 byte
/// 128-16383   → 2 bytes
/// 16384+      → 3 bytes

pub fn encode_len(len: usize) -> Vec<u8> {
    let mut buf = vec![];
    let mut val = len;

    loop {
        // take 7 bits
        let mut byte = (val & 0x7F) as u8;
        val >>= 7;

        // if more bytes coming, set high bit
        if val > 0 {
            byte |= 0x80;
        }

        buf.push(byte);

        if val == 0 {
            break;
        }
    }
    buf
}

pub fn decode_len(bytes: &[u8]) -> (usize, usize) {
    let mut len = 0usize;
    let mut bytes_read = 0;

    for (i, byte) in bytes.iter().enumerate() {
        // take lower 7 bits
        len |= ((byte & 0x7F) as usize) << (i * 7);
        bytes_read += 1;
        // if high bit not set — we're done
        if byte & 0x80 == 0 {
            break;
        }
    }

    (len, bytes_read)
}
