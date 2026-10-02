//! Invoices made by a real node (ldk-node on regtest, captured from the node-core regtest test),
//! checked field by field against `lightning-invoice`.

use std::str::FromStr;

use invoice_core::{CheckId, DecodeContext, Network, PayeeSource, Status, Verdict, decode, to_hex};
use lightning_invoice::Bolt11Invoice;
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    title: String,
    invoice: String,
    amount_msat: Option<u64>,
    expiry_secs: u64,
}

fn fixtures() -> Vec<Fixture> {
    let json = include_str!("fixtures/regtest_invoices.json");
    return serde_json::from_str(json).expect("fixture file is valid JSON");
}

#[test]
fn real_regtest_invoices_match_the_oracle() {
    for f in fixtures() {
        let oracle = Bolt11Invoice::from_str(&f.invoice).expect(&f.title);
        let timestamp = oracle.duration_since_epoch().as_secs();
        let ctx = DecodeContext {
            expected_network: Some(Network::Regtest),
            ..DecodeContext::at(timestamp)
        };
        let decoded = decode(&f.invoice, &ctx).expect(&f.title);
        let invoice = &decoded.invoice;

        assert_eq!(decoded.report.verdict, Verdict::Payable, "{}", f.title);
        assert_eq!(invoice.network, Network::Regtest, "{}", f.title);
        assert_eq!(invoice.amount_msat, f.amount_msat, "{}", f.title);
        assert_eq!(
            invoice.amount_msat,
            oracle.amount_milli_satoshis(),
            "{}",
            f.title
        );
        assert_eq!(invoice.expiry_secs, f.expiry_secs, "{}", f.title);
        assert_eq!(invoice.timestamp, timestamp, "{}", f.title);
        assert_eq!(
            invoice.min_final_cltv_expiry,
            oracle.min_final_cltv_expiry_delta(),
            "{}",
            f.title
        );
        assert_eq!(
            to_hex(&invoice.payment_hash.as_ref().unwrap().0),
            oracle.payment_hash().to_string(),
            "{}",
            f.title
        );
        // ldk-node writes the payee key into the invoice; the signature must still verify against it.
        assert_eq!(invoice.payee.source, PayeeSource::Explicit, "{}", f.title);
        assert_eq!(
            invoice.payee.pubkey,
            oracle.get_payee_pub_key(),
            "{}",
            f.title
        );

        let hints = oracle.route_hints();
        assert_eq!(invoice.route_hints.len(), hints.len(), "{}", f.title);
        assert!(!hints.is_empty(), "private channels need a route hint");
        for (ours, theirs) in invoice.route_hints.iter().zip(hints.iter()) {
            assert_eq!(ours.len(), theirs.0.len(), "{}", f.title);
            for (a, b) in ours.iter().zip(theirs.0.iter()) {
                assert_eq!(a.pubkey, b.src_node_id, "{}", f.title);
                assert_eq!(a.short_channel_id, b.short_channel_id, "{}", f.title);
                assert_eq!(a.fee_base_msat, b.fees.base_msat, "{}", f.title);
                assert_eq!(
                    a.fee_proportional_millionths, b.fees.proportional_millionths,
                    "{}",
                    f.title
                );
                assert_eq!(a.cltv_expiry_delta, b.cltv_expiry_delta, "{}", f.title);
            }
        }
    }
}

#[test]
fn short_expiry_is_caught_just_after_it_passes() {
    let tip = fixtures()
        .into_iter()
        .find(|f| f.amount_msat.is_none())
        .unwrap();
    let timestamp = Bolt11Invoice::from_str(&tip.invoice)
        .unwrap()
        .duration_since_epoch()
        .as_secs();
    let decoded = decode(&tip.invoice, &DecodeContext::at(timestamp + 61)).unwrap();
    let expiry = decoded
        .report
        .checks
        .iter()
        .find(|c| c.id == CheckId::Expiry)
        .unwrap();
    assert_eq!(expiry.status, Status::Fail);
    assert_eq!(decoded.report.verdict, Verdict::NotPayable);
}

#[test]
fn regtest_invoice_on_a_mainnet_wallet_is_not_payable() {
    let f = fixtures().remove(0);
    let timestamp = Bolt11Invoice::from_str(&f.invoice)
        .unwrap()
        .duration_since_epoch()
        .as_secs();
    let ctx = DecodeContext {
        expected_network: Some(Network::Bitcoin),
        ..DecodeContext::at(timestamp)
    };
    let decoded = decode(&f.invoice, &ctx).unwrap();
    assert_eq!(decoded.report.verdict, Verdict::NotPayable);
}
