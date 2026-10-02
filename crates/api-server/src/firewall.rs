//! Pre-payment checks: every invoice goes through `invoice-core` before the node may pay it.

use invoice_core::{DecodeContext, Network, Verdict};
use serde_json::json;

use crate::error::ApiError;

pub struct PayPolicy {
    pub network: node_core::Network,
    pub max_amount_msat: u64,
}

/// Returns `Ok` only when the invoice is payable for this node: right network, not expired,
/// correctly signed, within the amount limit and, when given, from the expected payee.
pub fn check(
    invoice: &str,
    amount_msat: Option<u64>,
    expected_payee: Option<&str>,
    policy: &PayPolicy,
    now_unix: u64,
) -> Result<(), ApiError> {
    let expected_payee = match expected_payee {
        None => None,
        Some(key) => Some(
            key.trim()
                .parse()
                .map_err(|_| ApiError::bad_request(format!("bad expected_payee {key:?}")))?,
        ),
    };
    let ctx = DecodeContext {
        now_unix,
        expected_network: Some(invoice_network(policy.network)),
        expected_payee,
        description_preimage: None,
        max_amount_msat: Some(policy.max_amount_msat),
    };
    let decoded = invoice_core::decode(invoice, &ctx)
        .map_err(|err| ApiError::decode_failed(err.to_string()))?;

    if decoded.report.verdict != Verdict::Payable {
        let failed: Vec<String> = decoded
            .report
            .checks
            .iter()
            .filter(|c| c.status == invoice_core::Status::Fail)
            .map(|c| c.message.clone())
            .collect();
        return Err(ApiError::payment_refused(
            format!("refused to pay: {}", failed.join("; ")),
            json!(decoded.report),
        ));
    }

    match (decoded.invoice.amount_msat, amount_msat) {
        (None, Some(amount)) if amount > policy.max_amount_msat => {
            return Err(ApiError::payment_refused(
                format!(
                    "refused to pay: {amount} msat is above the {} msat limit",
                    policy.max_amount_msat
                ),
                json!(decoded.report),
            ));
        }
        (None, Some(0)) => {
            return Err(ApiError::bad_request(
                "amount must be above zero".to_string(),
            ));
        }
        _ => {}
    }
    return Ok(());
}

fn invoice_network(network: node_core::Network) -> Network {
    return match network {
        node_core::Network::Bitcoin => Network::Bitcoin,
        node_core::Network::Signet => Network::Signet,
        node_core::Network::Regtest => Network::Regtest,
        _ => Network::Testnet,
    };
}

// === Tests

#[cfg(test)]
mod tests {
    use super::*;

    // BOLT11 spec examples: mainnet, signed at 1496314658 with a one-minute expiry for COFFEE.
    const COFFEE: &str = "lnbc2500u1pvjluezsp5zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zygspp5qqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqypqdq5xysxxatsyp3k7enxv4jsxqzpu9qrsgquk0rl77nj30yxdy8j9vdx85fkpmdla2087ne0xh8nhedh8w27kyke0lp53ut353s06fv3qfegext0eh0ymjpf39tuven09sam30g4vgpfna3rh";
    const NOW: u64 = 1496314658;

    fn mainnet(max: u64) -> PayPolicy {
        return PayPolicy {
            network: node_core::Network::Bitcoin,
            max_amount_msat: max,
        };
    }

    #[test]
    fn payable_invoice_passes() {
        assert!(check(COFFEE, None, None, &mainnet(u64::MAX), NOW).is_ok());
    }

    #[test]
    fn wrong_network_is_refused() {
        let policy = PayPolicy {
            network: node_core::Network::Regtest,
            max_amount_msat: u64::MAX,
        };
        let err = check(COFFEE, None, None, &policy, NOW).unwrap_err();
        assert!(format!("{err:?}").contains("payment_refused"));
    }

    #[test]
    fn expired_invoice_is_refused() {
        let err = check(COFFEE, None, None, &mainnet(u64::MAX), NOW + 3600).unwrap_err();
        assert!(format!("{err:?}").contains("payment_refused"));
    }

    #[test]
    fn amount_above_the_limit_is_refused() {
        let err = check(COFFEE, None, None, &mainnet(1000), NOW).unwrap_err();
        assert!(format!("{err:?}").contains("payment_refused"));
    }

    #[test]
    fn unexpected_payee_is_refused() {
        let other = "02eec7245d6b7d2ccb30380bfbe2a3648cd7a942653f5aa340edcea1f283686619";
        let err = check(COFFEE, None, Some(other), &mainnet(u64::MAX), NOW).unwrap_err();
        assert!(format!("{err:?}").contains("payment_refused"));
    }

    // The invoice amount is fixed at 250,000 sat: the limit check is inclusive of the limit itself.
    const COFFEE_MSAT: u64 = 250_000_000;

    #[test]
    fn the_limit_is_inclusive() {
        assert!(check(COFFEE, None, None, &mainnet(COFFEE_MSAT), NOW).is_ok());
        let err = check(COFFEE, None, None, &mainnet(COFFEE_MSAT - 1), NOW).unwrap_err();
        assert!(format!("{err:?}").contains("payment_refused"));
    }

    #[test]
    fn a_matching_payee_is_accepted() {
        let payee = "03e7156ae33b0a208d0744199163177e909e80176e55d97a2f221ede0f934dd9ad";
        assert!(check(COFFEE, None, Some(payee), &mainnet(u64::MAX), NOW).is_ok());
        let padded = format!("  {payee}\n");
        assert!(check(COFFEE, None, Some(&padded), &mainnet(u64::MAX), NOW).is_ok());
    }

    #[test]
    fn a_malformed_expected_payee_is_a_bad_request() {
        for payee in ["", "nope", "02"] {
            let err = check(COFFEE, None, Some(payee), &mainnet(u64::MAX), NOW).unwrap_err();
            assert!(format!("{err:?}").contains("invalid_input"), "{payee:?}");
        }
    }

    #[test]
    fn the_signing_time_boundary_is_enforced() {
        // One minute of expiry: still payable on the last second, refused one second later.
        assert!(check(COFFEE, None, None, &mainnet(u64::MAX), NOW + 59).is_ok());
        assert!(check(COFFEE, None, None, &mainnet(u64::MAX), NOW + 61).is_err());
    }

    #[test]
    fn empty_and_whitespace_invoices_are_decode_errors() {
        for invoice in ["", "   ", "\n"] {
            let err = check(invoice, None, None, &mainnet(u64::MAX), NOW).unwrap_err();
            assert!(format!("{err:?}").contains("decode_failed"), "{invoice:?}");
        }
    }

    #[test]
    fn unreadable_invoice_is_a_decode_error() {
        let err = check("lnbc1nope", None, None, &mainnet(u64::MAX), NOW).unwrap_err();
        assert!(format!("{err:?}").contains("decode_failed"));
    }
}
