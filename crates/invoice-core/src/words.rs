//! Conversions between bech32 5-bit words and 8-bit bytes.

/// Regroups 5-bit words into bytes.
///
/// With `pad = false`, leftover bits (writer padding) are dropped: this is how field data is read.
/// With `pad = true`, a final partial byte is zero-padded: this is how the signed message is built.
pub fn to_bytes(words: &[u8], pad: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(words.len() * 5 / 8 + 1);
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for &word in words {
        acc = (acc << 5) | u32::from(word & 0x1f);
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    if pad && bits > 0 {
        out.push((acc << (8 - bits)) as u8);
    }
    return out;
}

/// Regroups bytes into 5-bit words, zero-padding the last word.
pub fn from_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() * 8 / 5 + 1);
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    for &byte in bytes {
        acc = (acc << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(((acc >> bits) & 0x1f) as u8);
        }
    }
    if bits > 0 {
        out.push(((acc << (5 - bits)) & 0x1f) as u8);
    }
    return out;
}

/// Reads words as one big-endian integer. Returns `None` if it does not fit in a `u64`.
pub fn be_int(words: &[u8]) -> Option<u64> {
    let mut value: u64 = 0;
    for &word in words {
        if value >> 59 != 0 {
            return None;
        }
        value = (value << 5) | u64::from(word & 0x1f);
    }
    return Some(value);
}

// === Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_from_spec_example() {
        // "pvjluez" in the bech32 alphabet
        assert_eq!(be_int(&[1, 12, 18, 31, 28, 25, 2]), Some(1_496_314_658));
    }

    #[test]
    fn be_int_overflow() {
        assert_eq!(be_int(&[31; 12]), Some((1u64 << 60) - 1));
        let mut two_pow_64 = vec![16];
        two_pow_64.extend([0; 12]);
        assert_eq!(be_int(&two_pow_64), None);
    }

    #[test]
    fn padding_is_dropped_or_kept() {
        // 52 words = 260 bits = 32 bytes + 4 padding bits
        let words = from_bytes(&[0xab; 32]);
        assert_eq!(words.len(), 52);
        assert_eq!(to_bytes(&words, false), vec![0xab; 32]);
        assert_eq!(to_bytes(&words, true).len(), 33);
    }
}
