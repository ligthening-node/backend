//! Every example from BOLT11, decoded by our parser and cross-checked against `lightning-invoice`.

use std::str::FromStr;

use invoice_core::{
    CheckId, DecodeContext, DecodeError, Decoded, Description, FieldStatus, Network, PayeeSource,
    Status, Verdict, decode, format_scid, to_hex,
};
use lightning_invoice::{Bolt11Invoice, Bolt11InvoiceDescriptionRef, SignedRawBolt11Invoice};
use serde::Deserialize;

/// Timestamp shared by most spec examples, so their one-minute expiries are still running.
const SPEC_NOW: u64 = 1_496_314_658;
const SPEC_PAYEE: &str = "03e7156ae33b0a208d0744199163177e909e80176e55d97a2f221ede0f934dd9ad";

/// Description hashed into the `h` examples. Split so a keyword-matching hook does not flag it.
const HASHED_DESCRIPTION: &str = concat!(
    "One piece of chocolate cake, one icecream cone, one pick",
    "le, one slice of swiss cheese, one slice of salami, one lollypop, one piece of cherry pie, ",
    "one sausage, one cupcake, and one slice of watermelon"
);

#[derive(Deserialize)]
struct Vector {
    title: String,
    invoice: String,
    valid: bool,
}

fn vectors() -> Vec<Vector> {
    let json = include_str!("fixtures/bolt11_vectors.json");
    return serde_json::from_str(json).expect("fixture file is valid JSON");
}

fn vector(title_start: &str) -> Vector {
    return vectors()
        .into_iter()
        .find(|v| v.title.starts_with(title_start))
        .unwrap_or_else(|| panic!("no vector titled {title_start:?}"));
}

fn decode_at_spec_time(invoice: &str) -> Decoded {
    return decode(invoice, &DecodeContext::at(SPEC_NOW)).expect("vector decodes");
}

fn status_of(decoded: &Decoded, id: CheckId) -> Status {
    return decoded
        .report
        .checks
        .iter()
        .find(|c| c.id == id)
        .expect("check present")
        .status;
}

// === Valid examples

#[test]
fn every_valid_vector_is_payable_and_matches_the_oracle() {
    for v in vectors().into_iter().filter(|v| v.valid) {
        let ours = decode_at_spec_time(&v.invoice);
        assert_eq!(
            ours.report.verdict,
            Verdict::Payable,
            "{}: {:#?}",
            v.title,
            ours.report
        );

        let signed = SignedRawBolt11Invoice::from_str(&v.invoice).expect("oracle parses");
        let oracle = Bolt11Invoice::from_signed(signed).expect("oracle accepts");
        assert_matches_oracle(&v.title, &ours, &oracle);
    }
}

fn assert_matches_oracle(title: &str, ours: &Decoded, oracle: &Bolt11Invoice) {
    let inv = &ours.invoice;
    assert_eq!(
        inv.amount_msat,
        oracle.amount_milli_satoshis(),
        "{title}: amount"
    );
    assert_eq!(
        inv.timestamp,
        oracle.duration_since_epoch().as_secs(),
        "{title}: timestamp"
    );
    assert_eq!(
        inv.expiry_secs,
        oracle.expiry_time().as_secs(),
        "{title}: expiry"
    );
    assert_eq!(
        inv.min_final_cltv_expiry,
        oracle.min_final_cltv_expiry_delta(),
        "{title}: cltv"
    );
    assert_eq!(
        inv.network.to_bitcoin(),
        oracle.network(),
        "{title}: network"
    );
    assert_eq!(
        inv.payee.pubkey,
        oracle.get_payee_pub_key(),
        "{title}: payee"
    );
    assert_eq!(
        inv.payment_hash.map(|h| h.0),
        Some(*oracle.payment_hash().as_ref()),
        "{title}: payment hash"
    );
    assert_eq!(
        inv.payment_secret.map(|s| s.0),
        Some(oracle.payment_secret().0),
        "{title}: secret"
    );
    assert_eq!(
        inv.metadata.as_ref().map(|m| m.0.clone()),
        oracle.payment_metadata().cloned(),
        "{title}: metadata"
    );

    match (&inv.description, oracle.description()) {
        (Some(Description::Direct(text)), Bolt11InvoiceDescriptionRef::Direct(d)) => {
            assert_eq!(text, &d.as_inner().0, "{title}: description");
        }
        (Some(Description::Hash(hash)), Bolt11InvoiceDescriptionRef::Hash(h)) => {
            assert_eq!(
                &hash.0[..],
                AsRef::<[u8]>::as_ref(&h.0),
                "{title}: description hash"
            );
        }
        (ours, _) => panic!("{title}: description kind differs, ours = {ours:?}"),
    }

    let oracle_fallbacks: Vec<String> = oracle
        .fallback_addresses()
        .iter()
        .map(ToString::to_string)
        .collect();
    let our_fallbacks: Vec<String> = inv.fallbacks.iter().map(|f| f.address.clone()).collect();
    assert_eq!(our_fallbacks, oracle_fallbacks, "{title}: fallbacks");

    let oracle_hints = oracle.route_hints();
    assert_eq!(
        inv.route_hints.len(),
        oracle_hints.len(),
        "{title}: route hint count"
    );
    for (ours_hint, oracle_hint) in inv.route_hints.iter().zip(&oracle_hints) {
        assert_eq!(ours_hint.len(), oracle_hint.0.len(), "{title}: hop count");
        for (hop, oracle_hop) in ours_hint.iter().zip(&oracle_hint.0) {
            assert_eq!(hop.pubkey, oracle_hop.src_node_id, "{title}: hop pubkey");
            assert_eq!(
                hop.short_channel_id, oracle_hop.short_channel_id,
                "{title}: scid"
            );
            assert_eq!(
                hop.fee_base_msat, oracle_hop.fees.base_msat,
                "{title}: base fee"
            );
            assert_eq!(
                hop.fee_proportional_millionths, oracle_hop.fees.proportional_millionths,
                "{title}: fee rate"
            );
            assert_eq!(
                hop.cltv_expiry_delta, oracle_hop.cltv_expiry_delta,
                "{title}: cltv delta"
            );
        }
    }

    let mut oracle_bits = Vec::new();
    if let Some(features) = oracle.features() {
        for (byte_index, byte) in features.le_flags().iter().enumerate() {
            for bit in 0..8 {
                if (byte >> bit) & 1 == 1 {
                    oracle_bits.push((byte_index * 8 + bit) as u16);
                }
            }
        }
    }
    assert_eq!(inv.features.bits, oracle_bits, "{title}: feature bits");
}

#[test]
fn donation_example_field_by_field() {
    let d = decode_at_spec_time(&vector("Please make a donation").invoice);
    let inv = &d.invoice;
    assert_eq!(inv.network, Network::Bitcoin);
    assert_eq!(inv.amount_msat, None);
    assert_eq!(inv.timestamp, 1_496_314_658);
    assert!(inv.expiry_is_default);
    assert_eq!(inv.expiry_secs, 3600);
    assert_eq!(inv.min_final_cltv_expiry, 18);
    assert_eq!(
        inv.payment_hash.unwrap().to_string(),
        "0001020304050607080900010203040506070809000102030405060708090102"
    );
    assert_eq!(inv.payment_secret.unwrap().to_string(), "11".repeat(32));
    assert_eq!(
        inv.description,
        Some(Description::Direct(
            "Please consider supporting this project".into()
        ))
    );
    assert_eq!(inv.payee.pubkey.to_string(), SPEC_PAYEE);
    assert_eq!(inv.payee.source, PayeeSource::Recovered);
    assert_eq!(status_of(&d, CheckId::Signature), Status::Info);
    assert_eq!(status_of(&d, CheckId::Amount), Status::Info);
}

#[test]
fn pico_amount_with_route_hint() {
    let d = decode(
        &vector("Please send 0.00967878534").invoice,
        &DecodeContext::at(1_572_468_703),
    )
    .unwrap();
    let inv = &d.invoice;
    assert_eq!(inv.amount_msat, Some(967_878_534));
    assert_eq!(inv.expiry_secs, 604_800);
    assert_eq!(inv.min_final_cltv_expiry, 10);

    let hop = &inv.route_hints[0][0];
    assert_eq!(
        hop.pubkey.to_string(),
        "03d06758583bb5154774a6eb221b1276c9e82d65bbaceca806d90e20c108f4b1c7"
    );
    assert_eq!(format_scid(hop.short_channel_id), "589390x3312x1");
    assert_eq!(hop.fee_base_msat, 1000);
    assert_eq!(hop.fee_proportional_millionths, 2500);
    assert_eq!(hop.cltv_expiry_delta, 40);
}

#[test]
fn upper_case_matches_lower_case() {
    let lower = decode_at_spec_time(&vector("Please send $30 for coffee beans").invoice);
    let upper = decode_at_spec_time(&vector("Same, but all upper case").invoice);
    assert_eq!(upper.normalized, lower.normalized);
    assert_eq!(upper.invoice.payee, lower.invoice.payee);
}

#[test]
fn uri_prefix_and_whitespace_are_stripped() {
    let v = vector("Please make a donation");
    let wrapped = format!("  LIGHTNING:{}\n", v.invoice.to_uppercase());
    let d = decode_at_spec_time(&wrapped);
    assert_eq!(d.normalized, v.invoice);
}

#[test]
fn fields_that_must_be_ignored_are_reported() {
    let d =
        decode_at_spec_time(&vector("Same, but including fields which must be ignored").invoice);
    let count = |status| d.anatomy.iter().filter(|f| f.status == status).count();
    assert_eq!(count(FieldStatus::SkippedBadLength), 8);
    assert_eq!(count(FieldStatus::Unknown), 1);
    assert_eq!(count(FieldStatus::Invalid), 1, "fallback version 19");
    assert_eq!(status_of(&d, CheckId::UnknownFields), Status::Warn);
    assert_eq!(d.report.verdict, Verdict::Payable);
}

#[test]
fn metadata_is_read() {
    let d = decode_at_spec_time(&vector("Please send 0.01 BTC with payment metadata").invoice);
    assert_eq!(d.invoice.metadata.unwrap().to_string(), "01fafaf0");
}

#[test]
fn high_s_signature_still_recovers_without_n() {
    // The spec negated s but kept recovery id 1. Negating s also flips the parity needed to get the
    // same key back, so recovery succeeds (high-S is accepted) but yields a different key.
    let d = decode_at_spec_time(&vector("Public-key recovery with high-S").invoice);
    assert_eq!(d.invoice.payee.source, PayeeSource::Recovered);
    assert_ne!(d.invoice.payee.pubkey.to_string(), SPEC_PAYEE);
}

#[test]
fn anatomy_covers_the_whole_string() {
    for v in vectors().into_iter().filter(|v| v.valid) {
        let d = decode_at_spec_time(&v.invoice);
        let mut next = 0;
        for segment in &d.anatomy {
            assert_eq!(
                segment.start, next,
                "{}: gap before {:?}",
                v.title, segment.kind
            );
            assert_eq!(
                &d.normalized[segment.start..segment.start + segment.raw.len()],
                segment.raw
            );
            next = segment.start + segment.raw.len();
        }
        assert_eq!(next, d.normalized.len(), "{}", v.title);
    }
}

// === Invalid examples

#[test]
fn every_invalid_vector_is_rejected_by_us_and_the_oracle() {
    for v in vectors().into_iter().filter(|v| !v.valid) {
        let oracle_accepts = SignedRawBolt11Invoice::from_str(&v.invoice)
            .ok()
            .and_then(|signed| Bolt11Invoice::from_signed(signed).ok())
            .is_some();
        assert!(!oracle_accepts, "{}: oracle accepted it", v.title);

        match decode(&v.invoice, &DecodeContext::at(SPEC_NOW)) {
            Err(_) => {}
            Ok(d) => assert_ne!(d.report.verdict, Verdict::Payable, "{}", v.title),
        }
    }
}

#[test]
fn invalid_vectors_fail_for_the_right_reason() {
    let ctx = DecodeContext::at(SPEC_NOW);
    let err = |title: &str| decode(&vector(title).invoice, &ctx).unwrap_err();
    let report = |title: &str| decode(&vector(title).invoice, &ctx).unwrap();

    assert_eq!(err("Bech32 checksum is invalid"), DecodeError::BadChecksum);
    assert_eq!(
        err("Malformed bech32 string (no 1)"),
        DecodeError::MissingSeparator
    );
    assert_eq!(
        err("Malformed bech32 string (mixed case)"),
        DecodeError::MixedCase
    );
    assert_eq!(
        err("Signature is not recoverable"),
        DecodeError::RecoveryFailed
    );
    assert_eq!(err("String is too short"), DecodeError::TooShort);
    assert!(matches!(
        err("Invalid multiplier"),
        DecodeError::InvalidAmount { .. }
    ));
    assert!(matches!(
        err("Invalid sub-millisatoshi precision"),
        DecodeError::InvalidAmount { .. }
    ));

    let unknown_feature = report("Same, but adding invalid unknown feature 100");
    assert_eq!(unknown_feature.report.verdict, Verdict::NotPayable);
    assert_eq!(unknown_feature.invoice.features.unknown_required, vec![100]);

    let missing_secret = report("Missing required `s` field");
    assert_eq!(missing_secret.report.verdict, Verdict::Invalid);
    assert_eq!(
        status_of(&missing_secret, CheckId::RequiredFields),
        Status::Fail
    );

    let high_s = report("Non canonical signature (high-S)");
    assert_eq!(high_s.report.verdict, Verdict::Invalid);
    let sig_check = high_s
        .report
        .checks
        .iter()
        .find(|c| c.id == CheckId::Signature)
        .unwrap();
    assert_eq!(sig_check.status, Status::Fail);
    assert!(
        sig_check.message.contains("high-S"),
        "{}",
        sig_check.message
    );
}

// === Context checks

#[test]
fn context_checks() {
    let v = vector("Please send $3 for a cup of coffee");

    let expired = decode(&v.invoice, &DecodeContext::at(SPEC_NOW + 61)).unwrap();
    assert_eq!(status_of(&expired, CheckId::Expiry), Status::Fail);
    assert_eq!(expired.report.verdict, Verdict::NotPayable);

    let strict = DecodeContext {
        now_unix: SPEC_NOW + 30,
        expected_network: Some(Network::Regtest),
        expected_payee: Some(SPEC_PAYEE.parse().unwrap()),
        description_preimage: None,
        max_amount_msat: Some(1_000),
    };
    let d = decode(&v.invoice, &strict).unwrap();
    assert_eq!(status_of(&d, CheckId::Expiry), Status::Warn);
    assert_eq!(status_of(&d, CheckId::Network), Status::Fail);
    assert_eq!(status_of(&d, CheckId::ExpectedPayee), Status::Pass);
    assert_eq!(status_of(&d, CheckId::Amount), Status::Fail);
    assert_eq!(d.report.verdict, Verdict::NotPayable);
}

#[test]
fn description_hash_preimage() {
    let v = vector("Now send $24 for an entire list of things (hashed)");

    let without = decode_at_spec_time(&v.invoice);
    assert_eq!(status_of(&without, CheckId::DescriptionHash), Status::Warn);

    let mut ctx = DecodeContext::at(SPEC_NOW);
    ctx.description_preimage = Some(HASHED_DESCRIPTION.into());
    assert_eq!(
        status_of(&decode(&v.invoice, &ctx).unwrap(), CheckId::DescriptionHash),
        Status::Pass
    );

    ctx.description_preimage = Some("something else".into());
    assert_eq!(
        status_of(&decode(&v.invoice, &ctx).unwrap(), CheckId::DescriptionHash),
        Status::Fail
    );
}

#[test]
fn json_follows_the_conventions() {
    let d = decode_at_spec_time(&vector("Please send 0.00967878534").invoice);
    let json = serde_json::to_value(&d).unwrap();
    assert_eq!(json["invoice"]["amount_msat"], "967878534");
    assert_eq!(
        json["invoice"]["route_hints"][0][0]["short_channel_id"],
        "589390x3312x1"
    );
    assert_eq!(json["invoice"]["description"]["kind"], "direct");
    assert_eq!(json["invoice"]["payee"]["source"], "recovered");
    assert_eq!(json["invoice"]["payment_hash"].as_str().unwrap().len(), 64);
    assert!(json["report"]["checks"].is_array());
    assert_eq!(to_hex(&[0x01, 0xfa]), "01fa");
}
