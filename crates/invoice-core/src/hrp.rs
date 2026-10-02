//! Human-readable part: `ln` + currency prefix + optional amount.

use crate::decoded::Network;
use crate::error::DecodeError;

const MSAT_PER_BTC: u64 = 100_000_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hrp {
    pub network: Network,
    pub amount_msat: Option<u64>,
}

pub fn parse(hrp: &str) -> Result<Hrp, DecodeError> {
    let Some(rest) = hrp.strip_prefix("ln") else {
        return Err(DecodeError::UnknownCurrency {
            prefix: hrp.to_string(),
        });
    };

    // Currency codes never contain digits, so the currency ends at the first digit.
    let split = rest
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or(rest.len());
    let (currency, amount) = rest.split_at(split);

    let network = match currency {
        "bc" => Network::Bitcoin,
        "tb" => Network::Testnet,
        "tbs" => Network::Signet,
        "bcrt" => Network::Regtest,
        _ => {
            return Err(DecodeError::UnknownCurrency {
                prefix: format!("ln{currency}"),
            });
        }
    };

    let amount_msat = if amount.is_empty() {
        None
    } else {
        Some(parse_amount(amount)?)
    };
    return Ok(Hrp {
        network,
        amount_msat,
    });
}

/// Converts an HRP amount such as `2500u` or `9678785340p` to millisatoshis.
pub fn parse_amount(amount: &str) -> Result<u64, DecodeError> {
    let invalid = |reason: &str| DecodeError::InvalidAmount {
        amount: amount.to_string(),
        reason: reason.to_string(),
    };

    let (digits, multiplier) = match amount.char_indices().last() {
        Some((i, c)) if c.is_ascii_alphabetic() => (&amount[..i], Some(c)),
        _ => (amount, None),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid(
            "expected digits followed by an optional multiplier m, u, n or p",
        ));
    }
    if digits.starts_with('0') {
        return Err(invalid("amount must be positive with no leading zeros"));
    }
    let value: u64 = digits.parse().map_err(|_| invalid("amount is too large"))?;

    let msat = match multiplier {
        None => value.checked_mul(MSAT_PER_BTC),
        Some('m') => value.checked_mul(MSAT_PER_BTC / 1_000),
        Some('u') => value.checked_mul(MSAT_PER_BTC / 1_000_000),
        Some('n') => value.checked_mul(MSAT_PER_BTC / 1_000_000_000),
        Some('p') => {
            // One pico-bitcoin is 0.1 msat, and HTLCs cannot carry fractions of a msat.
            if value % 10 != 0 {
                return Err(invalid(
                    "pico amounts must end in 0 (no sub-millisatoshi precision)",
                ));
            }
            Some(value / 10)
        }
        Some(_) => return Err(invalid("unknown multiplier, expected m, u, n or p")),
    };
    return msat.ok_or_else(|| invalid("amount is too large"));
}

// === Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn networks() {
        assert_eq!(parse("lnbc").unwrap().network, Network::Bitcoin);
        assert_eq!(parse("lntb").unwrap().network, Network::Testnet);
        assert_eq!(parse("lntbs").unwrap().network, Network::Signet);
        assert_eq!(parse("lnbcrt2500u").unwrap().network, Network::Regtest);
        assert!(matches!(
            parse("lnxx"),
            Err(DecodeError::UnknownCurrency { .. })
        ));
        assert!(matches!(
            parse("bc"),
            Err(DecodeError::UnknownCurrency { .. })
        ));
    }

    #[test]
    fn amounts() {
        assert_eq!(parse("lnbc").unwrap().amount_msat, None);
        assert_eq!(parse_amount("1").unwrap(), 100_000_000_000);
        assert_eq!(parse_amount("20m").unwrap(), 2_000_000_000);
        assert_eq!(parse_amount("2500u").unwrap(), 250_000_000);
        assert_eq!(parse_amount("10n").unwrap(), 1_000);
        assert_eq!(parse_amount("9678785340p").unwrap(), 967_878_534);
    }

    #[test]
    fn invalid_amounts() {
        for bad in [
            "2500x",
            "2500000001p",
            "0",
            "01m",
            "m",
            "1mm",
            "99999999999999999999",
        ] {
            assert!(parse_amount(bad).is_err(), "{bad}");
        }
    }
}
