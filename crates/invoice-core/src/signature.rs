//! The invoice signature: what is signed, and how the payee key is checked or recovered.

use bitcoin::hashes::{Hash, sha256};
use bitcoin::secp256k1::ecdsa::{RecoverableSignature, RecoveryId, Signature};
use bitcoin::secp256k1::{Message, PublicKey, Secp256k1};

use crate::error::DecodeError;
use crate::words::to_bytes;

pub const SIGNATURE_WORDS: usize = 104;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SigOutcome {
    /// The `n` field is present and the signature verifies against it.
    VerifiedExplicit,
    /// No `n` field; the payee key was recovered from the signature.
    Recovered(PublicKey),
    /// The `n` field is present but the signature does not verify against it.
    ExplicitMismatch {
        /// The signature would verify once normalized, so the only problem is that it is high-S.
        high_s: bool,
        recovered: Option<PublicKey>,
    },
}

/// SHA256(hrp as UTF-8 || data words before the signature, regrouped to bytes and zero-padded).
pub fn message_hash(hrp: &str, data_words: &[u8]) -> [u8; 32] {
    let mut preimage = hrp.as_bytes().to_vec();
    preimage.extend(to_bytes(data_words, true));
    return sha256::Hash::hash(&preimage).to_byte_array();
}

/// Checks the signature against the explicit `n` key, or recovers the key when `n` is absent.
pub fn check(
    hash: [u8; 32],
    sig_words: &[u8],
    explicit: Option<&PublicKey>,
) -> Result<SigOutcome, DecodeError> {
    let bytes = to_bytes(sig_words, false);
    if bytes.len() != 65 {
        return Err(DecodeError::InvalidSignatureEncoding);
    }
    let recovery_id = RecoveryId::from_i32(i32::from(bytes[64]))
        .map_err(|_| DecodeError::InvalidSignatureEncoding)?;
    let recoverable = RecoverableSignature::from_compact(&bytes[..64], recovery_id)
        .map_err(|_| DecodeError::InvalidSignatureEncoding)?;

    let secp = Secp256k1::verification_only();
    let message = Message::from_digest(hash);
    let recovered = secp.recover_ecdsa(&message, &recoverable).ok();

    let Some(key) = explicit else {
        // Recovery accepts both low-S and high-S signatures, as the spec requires.
        return recovered
            .map(SigOutcome::Recovered)
            .ok_or(DecodeError::RecoveryFailed);
    };

    // libsecp256k1 verification only accepts low-S signatures, which is what the spec requires with `n`.
    let signature = recoverable.to_standard();
    if secp.verify_ecdsa(&message, &signature, key).is_ok() {
        return Ok(SigOutcome::VerifiedExplicit);
    }
    let mut normalized: Signature = signature;
    normalized.normalize_s();
    let high_s = normalized != signature && secp.verify_ecdsa(&message, &normalized, key).is_ok();
    return Ok(SigOutcome::ExplicitMismatch { high_s, recovered });
}
