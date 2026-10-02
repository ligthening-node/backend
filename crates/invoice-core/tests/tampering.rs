//! What a signature does and does not prove, shown by editing invoices and re-encoding them.

use bitcoin::secp256k1::{Message, PublicKey, Secp256k1, SecretKey};
use invoice_core::bech32::{self, char_to_word};
use invoice_core::signature::{SIGNATURE_WORDS, message_hash};
use invoice_core::words::from_bytes;
use invoice_core::{CheckId, DecodeContext, PayeeSource, Status, Verdict, decode};

/// The private key every BOLT11 example is signed with.
const SPEC_KEY: &str = "e126f68f7eafcc8b74f54d269fe206be715000f94dac067d1c04a8ca3b2db734";
const SPEC_NOW: u64 = 1_496_314_658;
const COFFEE: &str = "lnbc2500u1pvjluezsp5zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zygspp5qqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqypqdq5xysxxatsyp3k7enxv4jsxqzpu9qrsgquk0rl77nj30yxdy8j9vdx85fkpmdla2087ne0xh8nhedh8w27kyke0lp53ut353s06fv3qfegext0eh0ymjpf39tuven09sam30g4vgpfna3rh";

fn spec_key() -> (SecretKey, PublicKey) {
    let secret: SecretKey = SPEC_KEY.parse().unwrap();
    let public = secret.public_key(&Secp256k1::new());
    return (secret, public);
}

fn check_status(invoice: &str, ctx: &DecodeContext, id: CheckId) -> Status {
    let decoded = decode(invoice, ctx).unwrap();
    return decoded
        .report
        .checks
        .iter()
        .find(|c| c.id == id)
        .unwrap()
        .status;
}

/// Returns the invoice with its HRP replaced, keeping the old signature and fixing only the checksum.
fn with_hrp(invoice: &str, hrp: &str) -> String {
    let parts = bech32::decode(invoice).unwrap();
    return bech32::encode(hrp, &parts.data);
}

/// Signs `hrp` + `unsigned_words` with the spec key, the same way a real node would.
fn sign(hrp: &str, unsigned_words: &[u8]) -> String {
    let (secret, _) = spec_key();
    let hash = message_hash(hrp, unsigned_words);
    let sig = Secp256k1::new().sign_ecdsa_recoverable(&Message::from_digest(hash), &secret);
    let (recovery_id, compact) = sig.serialize_compact();

    let mut sig_bytes = compact.to_vec();
    sig_bytes.push(recovery_id.to_i32() as u8);
    let mut data = unsigned_words.to_vec();
    data.extend(from_bytes(&sig_bytes));
    return bech32::encode(hrp, &data);
}

fn unsigned_words(invoice: &str) -> Vec<u8> {
    let data = bech32::decode(invoice).unwrap().data;
    return data[..data.len() - SIGNATURE_WORDS].to_vec();
}

fn n_field(key: &PublicKey) -> Vec<u8> {
    let mut field = vec![char_to_word('n').unwrap(), 1, 21]; // length 53 = 1 * 32 + 21
    field.extend(from_bytes(&key.serialize()));
    return field;
}

#[test]
fn changing_the_amount_without_n_just_changes_the_recovered_payee() {
    let (_, spec_payee) = spec_key();
    let tampered = with_hrp(COFFEE, "lnbc9000u");

    let decoded = decode(&tampered, &DecodeContext::at(SPEC_NOW)).unwrap();
    assert_eq!(decoded.invoice.amount_msat, Some(900_000_000));
    assert_eq!(decoded.invoice.payee.source, PayeeSource::Recovered);
    assert_ne!(
        decoded.invoice.payee.pubkey, spec_payee,
        "a different 'payee' signed it"
    );
    assert_eq!(
        decoded.report.verdict,
        Verdict::Payable,
        "without an expected key nothing looks wrong"
    );

    let mut ctx = DecodeContext::at(SPEC_NOW);
    ctx.expected_payee = Some(spec_payee);
    let caught = decode(&tampered, &ctx).unwrap();
    assert_eq!(
        check_status(&tampered, &ctx, CheckId::ExpectedPayee),
        Status::Fail
    );
    assert_eq!(caught.report.verdict, Verdict::NotPayable);
}

#[test]
fn with_n_a_correct_signature_passes_and_tampering_fails() {
    let (_, spec_payee) = spec_key();
    let mut words = unsigned_words(COFFEE);
    words.extend(n_field(&spec_payee));
    let signed = sign("lnbc2500u", &words);
    let ctx = DecodeContext::at(SPEC_NOW);

    let decoded = decode(&signed, &ctx).unwrap();
    assert_eq!(decoded.invoice.payee.source, PayeeSource::Explicit);
    assert_eq!(
        check_status(&signed, &ctx, CheckId::Signature),
        Status::Pass
    );
    assert_eq!(decoded.report.verdict, Verdict::Payable);

    let tampered = with_hrp(&signed, "lnbc9000u");
    assert_eq!(
        check_status(&tampered, &ctx, CheckId::Signature),
        Status::Fail
    );
    assert_eq!(
        decode(&tampered, &ctx).unwrap().report.verdict,
        Verdict::Invalid
    );
}

#[test]
fn n_field_naming_someone_else_fails() {
    let other = SecretKey::from_slice(&[7; 32])
        .unwrap()
        .public_key(&Secp256k1::new());
    let mut words = unsigned_words(COFFEE);
    words.extend(n_field(&other));
    let signed = sign("lnbc2500u", &words);

    let ctx = DecodeContext::at(SPEC_NOW);
    let decoded = decode(&signed, &ctx).unwrap();
    assert_eq!(decoded.invoice.payee.pubkey, other);
    assert_eq!(
        check_status(&signed, &ctx, CheckId::Signature),
        Status::Fail
    );
    assert_eq!(decoded.report.verdict, Verdict::Invalid);
}
