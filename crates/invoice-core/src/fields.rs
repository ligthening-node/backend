//! Tagged fields: `type (1 word) | data_length (2 words) | data (data_length words)`.

use bitcoin::hashes::Hash;
use bitcoin::secp256k1::PublicKey;
use bitcoin::{Address, PubkeyHash, ScriptBuf, ScriptHash, WitnessProgram, WitnessVersion};

use crate::bech32::word_to_char;
use crate::decoded::{
    Fallback, Features, FieldStatus, Hash32, HexBytes, KnownFeature, Network, RawField, RouteHint,
    RouteHop, SegmentKind,
};
use crate::error::DecodeError;
use crate::words::{be_int, to_bytes};

const HASH_WORDS: usize = 52;
const PUBKEY_WORDS: usize = 53;
const ROUTE_HOP_BYTES: usize = 51;

/// Known BOLT 9 features that may appear in an invoice, by even (required) bit.
const KNOWN_FEATURES: [(u16, &str); 4] = [
    (8, "var_onion_optin"),
    (14, "payment_secret"),
    (16, "basic_mpp"),
    (48, "option_payment_metadata"),
];

/// Everything read from the tagged fields. Counts include only well-formed copies of each field.
#[derive(Debug, Default)]
pub struct Fields {
    pub payment_hash: Option<Hash32>,
    pub payment_hash_count: usize,
    pub payment_secret: Option<Hash32>,
    pub payment_secret_count: usize,
    pub description: Option<String>,
    pub description_count: usize,
    pub description_hash: Option<Hash32>,
    pub description_hash_count: usize,
    pub payee: Option<PublicKey>,
    pub expiry: Option<u64>,
    pub min_final_cltv: Option<u64>,
    pub route_hints: Vec<RouteHint>,
    pub fallbacks: Vec<Fallback>,
    pub features: Features,
    pub features_seen: bool,
    pub metadata: Option<HexBytes>,
    /// Tags of `x`, `c` or `9` fields that start with a zero word.
    pub non_minimal: Vec<char>,
    pub raw: Vec<RawField>,
}

/// Parses the words between the timestamp and the signature.
/// `base_pos` is the character offset of `words[0]` in the normalized invoice.
pub fn parse(words: &[u8], base_pos: usize, network: Network) -> Result<Fields, DecodeError> {
    let mut fields = Fields::default();
    let mut i = 0;
    while i < words.len() {
        let tag = word_to_char(words[i]);
        let pos = base_pos + i;
        if i + 3 > words.len() {
            return Err(DecodeError::FieldOverrun { tag, pos });
        }
        let len = usize::from(words[i + 1]) * 32 + usize::from(words[i + 2]);
        let end = i + 3 + len;
        if end > words.len() {
            return Err(DecodeError::FieldOverrun { tag, pos });
        }

        let data = &words[i + 3..end];
        let (status, note) = fields.read(tag, data, network);
        fields.raw.push(RawField {
            kind: SegmentKind::TaggedField,
            tag: Some(tag),
            start: pos,
            raw: words[i..end].iter().map(|&w| word_to_char(w)).collect(),
            len_words: Some(len as u16),
            status,
            note,
        });
        i = end;
    }
    return Ok(fields);
}

impl Fields {
    fn read(&mut self, tag: char, data: &[u8], network: Network) -> (FieldStatus, Option<String>) {
        return match tag {
            'p' => read_hash(data, &mut self.payment_hash, &mut self.payment_hash_count),
            's' => read_hash(
                data,
                &mut self.payment_secret,
                &mut self.payment_secret_count,
            ),
            'h' => read_hash(
                data,
                &mut self.description_hash,
                &mut self.description_hash_count,
            ),
            'd' => match String::from_utf8(to_bytes(data, false)) {
                Ok(text) => {
                    self.description_count += 1;
                    first_wins(&mut self.description, text)
                }
                Err(_) => (
                    FieldStatus::Invalid,
                    Some("description is not valid UTF-8".into()),
                ),
            },
            'n' => {
                if data.len() != PUBKEY_WORDS {
                    return bad_length(PUBKEY_WORDS, data.len());
                }
                match PublicKey::from_slice(&to_bytes(data, false)) {
                    Ok(key) => first_wins(&mut self.payee, key),
                    Err(_) => (
                        FieldStatus::Invalid,
                        Some("not a valid secp256k1 public key".into()),
                    ),
                }
            }
            'x' => self.read_int(tag, data, |f| &mut f.expiry),
            'c' => self.read_int(tag, data, |f| &mut f.min_final_cltv),
            '9' => {
                if data.first() == Some(&0) {
                    self.non_minimal.push(tag);
                }
                if self.features_seen {
                    return (
                        FieldStatus::Duplicate,
                        Some("an earlier copy of this field is used".into()),
                    );
                }
                self.features_seen = true;
                self.features = parse_features(data);
                (FieldStatus::Parsed, None)
            }
            'm' => first_wins(&mut self.metadata, HexBytes(to_bytes(data, false))),
            'r' => match parse_route_hint(&to_bytes(data, false)) {
                Some(hint) => {
                    self.route_hints.push(hint);
                    (FieldStatus::Parsed, None)
                }
                None => (
                    FieldStatus::Invalid,
                    Some("route hint is not a list of 51-byte hops".into()),
                ),
            },
            'f' => match parse_fallback(data, network) {
                Ok(fallback) => {
                    self.fallbacks.push(fallback);
                    (FieldStatus::Parsed, None)
                }
                Err(reason) => (FieldStatus::Invalid, Some(reason)),
            },
            _ => (
                FieldStatus::Unknown,
                Some("unknown field type, skipped".into()),
            ),
        };
    }

    fn read_int(
        &mut self,
        tag: char,
        data: &[u8],
        slot: fn(&mut Fields) -> &mut Option<u64>,
    ) -> (FieldStatus, Option<String>) {
        let Some(value) = be_int(data) else {
            return (
                FieldStatus::Invalid,
                Some("value does not fit in 64 bits".into()),
            );
        };
        if data.first() == Some(&0) {
            self.non_minimal.push(tag);
        }
        return first_wins(slot(self), value);
    }
}

// === Field readers

fn first_wins<T>(slot: &mut Option<T>, value: T) -> (FieldStatus, Option<String>) {
    if slot.is_some() {
        return (
            FieldStatus::Duplicate,
            Some("an earlier copy of this field is used".into()),
        );
    }
    *slot = Some(value);
    return (FieldStatus::Parsed, None);
}

fn bad_length(expected: usize, got: usize) -> (FieldStatus, Option<String>) {
    let note = format!("expected {expected} words, got {got}; ignored");
    return (FieldStatus::SkippedBadLength, Some(note));
}

fn read_hash(
    data: &[u8],
    slot: &mut Option<Hash32>,
    count: &mut usize,
) -> (FieldStatus, Option<String>) {
    if data.len() != HASH_WORDS {
        return bad_length(HASH_WORDS, data.len());
    }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&to_bytes(data, false));
    *count += 1;
    return first_wins(slot, Hash32(hash));
}

/// Feature bits are big-endian over the words: bit 0 is the lowest bit of the last word.
pub fn parse_features(data: &[u8]) -> Features {
    let mut bits = Vec::new();
    for (j, &word) in data.iter().rev().enumerate() {
        for b in 0..5 {
            if (word >> b) & 1 == 1 {
                bits.push((j * 5 + b) as u16);
            }
        }
    }

    let mut features = Features {
        bits: bits.clone(),
        ..Features::default()
    };
    for bit in bits {
        let even = bit & !1;
        match KNOWN_FEATURES.iter().find(|(known, _)| *known == even) {
            Some((_, name)) => features.known.push(KnownFeature {
                bit,
                name,
                required: bit == even,
            }),
            None if bit == even => features.unknown_required.push(bit),
            None => features.unknown_optional.push(bit),
        }
    }
    return features;
}

fn parse_route_hint(bytes: &[u8]) -> Option<RouteHint> {
    if bytes.is_empty() || bytes.len() % ROUTE_HOP_BYTES != 0 {
        return None;
    }
    let mut hops = Vec::with_capacity(bytes.len() / ROUTE_HOP_BYTES);
    for hop in bytes.chunks_exact(ROUTE_HOP_BYTES) {
        hops.push(RouteHop {
            pubkey: PublicKey::from_slice(&hop[0..33]).ok()?,
            short_channel_id: u64::from_be_bytes(hop[33..41].try_into().ok()?),
            fee_base_msat: u32::from_be_bytes(hop[41..45].try_into().ok()?),
            fee_proportional_millionths: u32::from_be_bytes(hop[45..49].try_into().ok()?),
            cltv_expiry_delta: u16::from_be_bytes(hop[49..51].try_into().ok()?),
        });
    }
    return Some(hops);
}

fn parse_fallback(data: &[u8], network: Network) -> Result<Fallback, String> {
    let Some((&version, rest)) = data.split_first() else {
        return Err("empty fallback field".into());
    };
    let program = to_bytes(rest, false);

    let script = match version {
        17 => {
            let hash: [u8; 20] = program
                .as_slice()
                .try_into()
                .map_err(|_| "P2PKH needs 20 bytes")?;
            ScriptBuf::new_p2pkh(&PubkeyHash::from_byte_array(hash))
        }
        18 => {
            let hash: [u8; 20] = program
                .as_slice()
                .try_into()
                .map_err(|_| "P2SH needs 20 bytes")?;
            ScriptBuf::new_p2sh(&ScriptHash::from_byte_array(hash))
        }
        0..=16 => {
            let witness_version = WitnessVersion::try_from(version).map_err(|e| e.to_string())?;
            let witness_program =
                WitnessProgram::new(witness_version, &program).map_err(|e| e.to_string())?;
            ScriptBuf::new_witness_program(&witness_program)
        }
        _ => return Err(format!("unknown fallback version {version}, skipped")),
    };

    let address = Address::from_script(&script, network.to_bitcoin()).map_err(|e| e.to_string())?;
    return Ok(Fallback {
        version,
        program: HexBytes(program),
        address: address.to_string(),
    });
}

// === Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_bits_8_14_99() {
        // "q5sqqqqqqqqqqqqqqqqsgq" from the spec: 9 field with features 8, 14 and 99
        let data: Vec<u8> = "sqqqqqqqqqqqqqqqqsgq"
            .chars()
            .map(|c| crate::bech32::char_to_word(c).unwrap())
            .collect();
        let features = parse_features(&data);
        assert_eq!(features.bits, vec![8, 14, 99]);
        assert_eq!(features.unknown_optional, vec![99]);
        assert!(features.unknown_required.is_empty());
    }

    #[test]
    fn scid_format() {
        let scid = (589_390u64 << 40) | (3312u64 << 16) | 1;
        assert_eq!(crate::decoded::format_scid(scid), "589390x3312x1");
    }
}
