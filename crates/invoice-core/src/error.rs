use serde::Serialize;
use thiserror::Error;

/// Fatal errors: the input cannot be read as a BOLT11 invoice at all.
///
/// Positions are character offsets into `Decoded::normalized` (the trimmed, lowercased input
/// without a `lightning:` prefix), so a UI can underline the exact spot.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum DecodeError {
    #[error("no bech32 separator '1' found")]
    MissingSeparator,

    #[error("invalid character {ch:?} at position {pos}")]
    InvalidChar { pos: usize, ch: char },

    #[error("mixed upper and lower case is not allowed")]
    MixedCase,

    #[error("bech32 checksum failed")]
    BadChecksum,

    #[error("unknown currency prefix {prefix:?}")]
    UnknownCurrency { prefix: String },

    #[error("invalid amount {amount:?}: {reason}")]
    InvalidAmount { amount: String, reason: String },

    #[error("invoice is too short to hold a timestamp and a signature")]
    TooShort,

    #[error("tagged field '{tag}' at position {pos} runs past the end of the data")]
    FieldOverrun { tag: char, pos: usize },

    #[error("signature is not a valid compact secp256k1 signature")]
    InvalidSignatureEncoding,

    #[error("could not recover a public key from the signature")]
    RecoveryFailed,
}
