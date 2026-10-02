//! Hand-written BOLT11 invoice decoder and validator.
//!
//! The core has no I/O and never reads the clock: the current time and any payment policy come in
//! through [`DecodeContext`], so results are deterministic and the crate runs unchanged in WASM.

pub mod bech32;
pub mod checks;
pub mod decoded;
pub mod error;
pub mod fields;
pub mod hrp;
pub mod signature;
pub mod words;

pub use decoded::*;
pub use error::DecodeError;

use crate::signature::{SIGNATURE_WORDS, SigOutcome};

const TIMESTAMP_WORDS: usize = 7;
const CHECKSUM_CHARS: usize = 6;
const DEFAULT_EXPIRY_SECS: u64 = 3600;
const DEFAULT_MIN_FINAL_CLTV: u64 = 18;

/// Decodes and validates a BOLT11 invoice.
///
/// Returns `Err` only when the input cannot be read as an invoice. An invoice that is readable but
/// expired, mis-signed or otherwise unusable returns `Ok`, with the problems in `report`.
pub fn decode(input: &str, ctx: &DecodeContext) -> Result<Decoded, DecodeError> {
    let cleaned = clean_input(input);
    let parts = bech32::decode(cleaned)?;
    let normalized = cleaned.to_ascii_lowercase();

    let data = &parts.data;
    if data.len() < TIMESTAMP_WORDS + SIGNATURE_WORDS {
        return Err(DecodeError::TooShort);
    }
    let hrp = hrp::parse(&parts.hrp)?;
    let sig_start = data.len() - SIGNATURE_WORDS;
    let timestamp = words::be_int(&data[..TIMESTAMP_WORDS]).ok_or(DecodeError::TooShort)?;

    let data_pos = parts.hrp.len() + 1;
    let fields = fields::parse(
        &data[TIMESTAMP_WORDS..sig_start],
        data_pos + TIMESTAMP_WORDS,
        hrp.network,
    )?;

    let hash = signature::message_hash(&parts.hrp, &data[..sig_start]);
    let sig = signature::check(hash, &data[sig_start..], fields.payee.as_ref())?;
    let payee = match (&sig, fields.payee) {
        (SigOutcome::Recovered(key), _) => Payee {
            pubkey: *key,
            source: PayeeSource::Recovered,
        },
        (_, Some(key)) => Payee {
            pubkey: key,
            source: PayeeSource::Explicit,
        },
        (_, None) => return Err(DecodeError::RecoveryFailed),
    };

    let description = match (&fields.description, fields.description_hash) {
        (Some(text), _) => Some(Description::Direct(text.clone())),
        (None, Some(hash)) => Some(Description::Hash(hash)),
        (None, None) => None,
    };
    let expiry_secs = fields.expiry.unwrap_or(DEFAULT_EXPIRY_SECS);

    let invoice = DecodedInvoice {
        network: hrp.network,
        amount_msat: hrp.amount_msat,
        timestamp,
        expiry_secs,
        expiry_is_default: fields.expiry.is_none(),
        expires_at: timestamp.saturating_add(expiry_secs),
        description,
        payment_hash: fields.payment_hash,
        payment_secret: fields.payment_secret,
        payee,
        min_final_cltv_expiry: fields.min_final_cltv.unwrap_or(DEFAULT_MIN_FINAL_CLTV),
        route_hints: fields.route_hints.clone(),
        fallbacks: fields.fallbacks.clone(),
        features: fields.features.clone(),
        metadata: fields.metadata.clone(),
    };

    let report = checks::run(&invoice, &fields, &sig, ctx);
    let anatomy = anatomy(&normalized, parts.hrp.len(), sig_start, fields.raw);
    return Ok(Decoded {
        normalized,
        invoice,
        report,
        anatomy,
    });
}

/// Trims whitespace and strips a `lightning:` URI prefix. `DecodeError` positions index into this.
pub fn clean_input(input: &str) -> &str {
    const PREFIX: &str = "lightning:";
    let input = input.trim();
    // `get` returns None inside a multibyte character, where plain slicing would panic.
    match input.get(..PREFIX.len()) {
        Some(head) if head.eq_ignore_ascii_case(PREFIX) => return &input[PREFIX.len()..],
        _ => return input,
    }
}

/// Lays out every segment of the invoice string in order, for the learning view.
fn anatomy(
    normalized: &str,
    hrp_len: usize,
    sig_start: usize,
    tagged: Vec<RawField>,
) -> Vec<RawField> {
    let data_pos = hrp_len + 1;
    let sig_pos = data_pos + sig_start;
    let checksum_pos = normalized.len() - CHECKSUM_CHARS;
    let segment = |kind: SegmentKind, start: usize, end: usize| RawField {
        kind,
        tag: None,
        start,
        raw: normalized[start..end].to_string(),
        len_words: None,
        status: FieldStatus::Parsed,
        note: None,
    };

    let mut out = Vec::with_capacity(tagged.len() + 5);
    out.push(segment(SegmentKind::Hrp, 0, hrp_len));
    out.push(segment(SegmentKind::Separator, hrp_len, data_pos));
    out.push(segment(
        SegmentKind::Timestamp,
        data_pos,
        data_pos + TIMESTAMP_WORDS,
    ));
    out.extend(tagged);
    out.push(segment(SegmentKind::Signature, sig_pos, checksum_pos));
    out.push(segment(
        SegmentKind::Checksum,
        checksum_pos,
        normalized.len(),
    ));
    return out;
}
