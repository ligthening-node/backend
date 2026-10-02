//! BIP-173 bech32, without the 90-character limit that BOLT11 invoices exceed.

use crate::error::DecodeError;

pub const CHARSET: &[u8; 32] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";
const GENERATORS: [u32; 5] = [
    0x3b6a_57b2,
    0x2650_8e6d,
    0x1ea1_19fa,
    0x3d42_33dd,
    0x2a14_62b3,
];
const CHECKSUM_LEN: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bech32 {
    pub hrp: String,
    /// Data words, checksum removed.
    pub data: Vec<u8>,
}

/// Decodes a bech32 string. The input must already be one case; the result is lowercase.
pub fn decode(input: &str) -> Result<Bech32, DecodeError> {
    let has_lower = input.bytes().any(|b| b.is_ascii_lowercase());
    let has_upper = input.bytes().any(|b| b.is_ascii_uppercase());
    if has_lower && has_upper {
        return Err(DecodeError::MixedCase);
    }
    if let Some((pos, ch)) = input.char_indices().find(|(_, c)| !(' '..='~').contains(c)) {
        return Err(DecodeError::InvalidChar { pos, ch });
    }

    let lower = input.to_ascii_lowercase();
    let sep = match lower.rfind('1') {
        Some(sep) if sep > 0 => sep,
        _ => return Err(DecodeError::MissingSeparator),
    };
    let hrp = &lower[..sep];
    let data_str = &lower[sep + 1..];
    if data_str.len() < CHECKSUM_LEN {
        return Err(DecodeError::TooShort);
    }

    let mut words = Vec::with_capacity(data_str.len());
    for (offset, ch) in data_str.char_indices() {
        match char_to_word(ch) {
            Some(value) => words.push(value),
            None => {
                return Err(DecodeError::InvalidChar {
                    pos: sep + 1 + offset,
                    ch,
                });
            }
        }
    }

    if polymod_with_hrp(hrp, &words) != 1 {
        return Err(DecodeError::BadChecksum);
    }
    words.truncate(words.len() - CHECKSUM_LEN);
    return Ok(Bech32 {
        hrp: hrp.to_string(),
        data: words,
    });
}

/// Encodes a lowercase bech32 string. Used by tests to build tampered but well-formed invoices.
pub fn encode(hrp: &str, data: &[u8]) -> String {
    let mut values = data.to_vec();
    values.extend_from_slice(&[0; CHECKSUM_LEN]);
    let polymod = polymod_with_hrp(hrp, &values) ^ 1;

    let mut out = String::with_capacity(hrp.len() + 1 + data.len() + CHECKSUM_LEN);
    out.push_str(hrp);
    out.push('1');
    for &word in data {
        out.push(CHARSET[usize::from(word)] as char);
    }
    for i in 0..CHECKSUM_LEN {
        let word = (polymod >> (5 * (5 - i))) & 0x1f;
        out.push(CHARSET[word as usize] as char);
    }
    return out;
}

/// Converts one bech32 character to its 5-bit value.
pub fn char_to_word(ch: char) -> Option<u8> {
    return CHARSET
        .iter()
        .position(|&c| c as char == ch)
        .map(|v| v as u8);
}

/// Converts a 5-bit value to its bech32 character.
pub fn word_to_char(word: u8) -> char {
    return CHARSET[usize::from(word & 0x1f)] as char;
}

// === Checksum

fn polymod_with_hrp(hrp: &str, words: &[u8]) -> u32 {
    let mut values: Vec<u8> = Vec::with_capacity(hrp.len() * 2 + 1 + words.len());
    values.extend(hrp.bytes().map(|b| b >> 5));
    values.push(0);
    values.extend(hrp.bytes().map(|b| b & 0x1f));
    values.extend_from_slice(words);
    return polymod(&values);
}

fn polymod(values: &[u8]) -> u32 {
    let mut chk: u32 = 1;
    for &value in values {
        let top = chk >> 25;
        chk = ((chk & 0x01ff_ffff) << 5) ^ u32::from(value);
        for (i, generator) in GENERATORS.iter().enumerate() {
            if (top >> i) & 1 == 1 {
                chk ^= generator;
            }
        }
    }
    return chk;
}

// === Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bip173_valid_vectors() {
        for s in [
            "A12UEL5L",
            "a12uel5l",
            "abcdef1qpzry9x8gf2tvdw0s3jn54khce6mua7lmqqqxw",
            "split1checkupstagehandshakeupstreamerranterredcaperred2y9e3w",
            "?1ezyfcl",
        ] {
            assert!(decode(s).is_ok(), "{s}");
        }
    }

    #[test]
    fn bip173_invalid_vectors() {
        assert_eq!(decode("pzry9x0s0muk"), Err(DecodeError::MissingSeparator));
        assert_eq!(decode("1pzry9x0s0muk"), Err(DecodeError::MissingSeparator));
        assert_eq!(decode("A1G7SGD8"), Err(DecodeError::BadChecksum));
        assert_eq!(
            decode("aBcdef1qpzry9x8gf2tvdw0s3jn54khce6mua7lmqqqxw"),
            Err(DecodeError::MixedCase)
        );
        assert!(matches!(
            decode("x1b4n0q5v"),
            Err(DecodeError::InvalidChar { ch: 'b', .. })
        ));
    }

    #[test]
    fn encode_round_trip() {
        let data = vec![0, 1, 2, 31, 30, 15];
        let s = encode("lnbcrt", &data);
        let decoded = decode(&s).unwrap();
        assert_eq!(decoded.hrp, "lnbcrt");
        assert_eq!(decoded.data, data);
    }
}
