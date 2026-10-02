//! Public output types.
//!
//! JSON conventions (shared by the CLI and the web UI):
//! - millisatoshi amounts are strings, since u64 can exceed JavaScript's 2^53
//! - hashes and public keys are lowercase hex strings
//! - enums with data are `{ "kind": ..., "value": ... }`
//! - field names are snake_case

use std::fmt;

use bitcoin::secp256k1::PublicKey;
use serde::{Deserialize, Serialize, Serializer};

// === Top level

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Decoded {
    /// The input after trimming, stripping `lightning:` and lowercasing. `RawField` offsets index into this.
    pub normalized: String,
    pub invoice: DecodedInvoice,
    pub report: ValidationReport,
    pub anatomy: Vec<RawField>,
}

/// Everything that is not inside the invoice: the current time and the caller's policy.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct DecodeContext {
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub now_unix: u64,
    pub expected_network: Option<Network>,
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub expected_payee: Option<PublicKey>,
    pub description_preimage: Option<String>,
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    #[serde(default, with = "opt_u64_string")]
    pub max_amount_msat: Option<u64>,
}

impl DecodeContext {
    pub fn at(now_unix: u64) -> Self {
        return DecodeContext {
            now_unix,
            ..DecodeContext::default()
        };
    }
}

// === Invoice facts

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct DecodedInvoice {
    pub network: Network,
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    #[serde(with = "opt_u64_string")]
    pub amount_msat: Option<u64>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub timestamp: u64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub expiry_secs: u64,
    pub expiry_is_default: bool,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub expires_at: u64,
    pub description: Option<Description>,
    pub payment_hash: Option<Hash32>,
    pub payment_secret: Option<Hash32>,
    pub payee: Payee,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub min_final_cltv_expiry: u64,
    pub route_hints: Vec<RouteHint>,
    pub fallbacks: Vec<Fallback>,
    pub features: Features,
    pub metadata: Option<HexBytes>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum Network {
    Bitcoin,
    Testnet,
    Signet,
    Regtest,
}

impl Network {
    pub fn to_bitcoin(self) -> bitcoin::Network {
        return match self {
            Network::Bitcoin => bitcoin::Network::Bitcoin,
            Network::Testnet => bitcoin::Network::Testnet,
            Network::Signet => bitcoin::Network::Signet,
            Network::Regtest => bitcoin::Network::Regtest,
        };
    }
}

impl fmt::Display for Network {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Network::Bitcoin => "mainnet",
            Network::Testnet => "testnet",
            Network::Signet => "signet",
            Network::Regtest => "regtest",
        };
        return f.write_str(name);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Description {
    Direct(String),
    Hash(Hash32),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Payee {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub pubkey: PublicKey,
    pub source: PayeeSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum PayeeSource {
    /// Taken from the `n` field and checked against the signature.
    Explicit,
    /// Recovered from the signature because there is no `n` field.
    Recovered,
}

/// One private route: hops in order from a public node towards the payee.
pub type RouteHint = Vec<RouteHop>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct RouteHop {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub pubkey: PublicKey,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    #[serde(serialize_with = "scid_string")]
    pub short_channel_id: u64,
    pub fee_base_msat: u32,
    pub fee_proportional_millionths: u32,
    pub cltv_expiry_delta: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Fallback {
    /// 0 to 16 is a segwit version, 17 is P2PKH, 18 is P2SH.
    pub version: u8,
    pub program: HexBytes,
    pub address: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Features {
    /// Set bit positions, lowest first.
    pub bits: Vec<u16>,
    pub known: Vec<KnownFeature>,
    /// Unknown even bits: "it's ok to be odd", so these make the invoice unpayable.
    pub unknown_required: Vec<u16>,
    pub unknown_optional: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct KnownFeature {
    pub bit: u16,
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub name: &'static str,
    pub required: bool,
}

/// Formats a short channel id as `block x tx_index x output`.
pub fn format_scid(scid: u64) -> String {
    return format!(
        "{}x{}x{}",
        scid >> 40,
        (scid >> 16) & 0xff_ffff,
        scid & 0xffff
    );
}

// === Validation report

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ValidationReport {
    pub verdict: Verdict,
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Payable,
    /// Well-formed and authentic, but fails a time, network, feature or policy check.
    NotPayable,
    /// Broken signature or missing required fields.
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Check {
    pub id: CheckId,
    pub status: Status,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum CheckId {
    Signature,
    ExpectedPayee,
    Expiry,
    Network,
    Amount,
    DescriptionHash,
    RequiredFields,
    Features,
    FieldEncoding,
    UnknownFields,
}

impl CheckId {
    /// Failing one of these means the invoice itself is broken, not just unpayable for us.
    pub fn is_structural(self) -> bool {
        return matches!(self, CheckId::Signature | CheckId::RequiredFields);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pass,
    Info,
    Warn,
    Fail,
    Skipped,
}

// === Anatomy

/// One labelled slice of the normalized invoice string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct RawField {
    pub kind: SegmentKind,
    /// The bech32 type character for tagged fields.
    pub tag: Option<char>,
    /// Character offset into `Decoded::normalized`.
    pub start: usize,
    pub raw: String,
    /// Data length in 5-bit words, for tagged fields.
    pub len_words: Option<u16>,
    pub status: FieldStatus,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum SegmentKind {
    Hrp,
    Separator,
    Timestamp,
    TaggedField,
    Signature,
    Checksum,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum FieldStatus {
    Parsed,
    /// A later copy of a field that was already read; the first one wins.
    Duplicate,
    /// Known tag with the wrong fixed length, ignored as the spec examples require.
    SkippedBadLength,
    /// Known tag whose contents could not be read (bad UTF-8, invalid key, unknown fallback version).
    Invalid,
    Unknown,
}

// === Serde helpers

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
pub struct Hash32(pub [u8; 32]);

impl fmt::Display for Hash32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        return f.write_str(&to_hex(&self.0));
    }
}

impl Serialize for Hash32 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        return serializer.serialize_str(&to_hex(&self.0));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(type = "string"))]
pub struct HexBytes(pub Vec<u8>);

impl fmt::Display for HexBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        return f.write_str(&to_hex(&self.0));
    }
}

impl Serialize for HexBytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        return serializer.serialize_str(&to_hex(&self.0));
    }
}

pub fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(DIGITS[usize::from(b >> 4)] as char);
        out.push(DIGITS[usize::from(b & 0x0f)] as char);
    }
    return out;
}

fn scid_string<S: Serializer>(scid: &u64, serializer: S) -> Result<S::Ok, S::Error> {
    return serializer.serialize_str(&format_scid(*scid));
}

mod opt_u64_string {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<S: Serializer>(value: &Option<u64>, serializer: S) -> Result<S::Ok, S::Error> {
        return match value {
            Some(v) => serializer.serialize_str(&v.to_string()),
            None => serializer.serialize_none(),
        };
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<u64>, D::Error> {
        let value: Option<String> = Option::deserialize(deserializer)?;
        return value
            .map(|s| s.parse::<u64>().map_err(D::Error::custom))
            .transpose();
    }
}
