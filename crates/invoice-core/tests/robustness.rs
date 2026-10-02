//! The decoder sits behind a public text box and the payment firewall, so arbitrary input must
//! produce an `Err` or a report, never a panic.

use invoice_core::{DecodeContext, decode};
use proptest::prelude::*;
use serde::Deserialize;

const SPEC_NOW: u64 = 1_496_314_658;

#[derive(Deserialize)]
struct Vector {
    invoice: String,
    valid: bool,
}

fn valid_invoices() -> Vec<String> {
    let json = include_str!("fixtures/bolt11_vectors.json");
    let vectors: Vec<Vector> = serde_json::from_str(json).expect("fixture file is valid JSON");
    return vectors
        .into_iter()
        .filter(|v| v.valid)
        .map(|v| v.invoice)
        .collect();
}

// === Arbitrary input

proptest! {
    #[test]
    fn arbitrary_text_never_panics(input in any::<String>()) {
        let _ = decode(&input, &DecodeContext::at(SPEC_NOW));
    }

    #[test]
    fn arbitrary_text_after_a_valid_prefix_never_panics(tail in "\\PC{0,200}") {
        for invoice in valid_invoices().iter().take(4) {
            let _ = decode(&format!("{invoice}{tail}"), &DecodeContext::at(SPEC_NOW));
        }
    }

    #[test]
    fn multibyte_text_around_the_uri_prefix_never_panics(
        head in "\\PC{0,12}",
        tail in "\\PC{0,40}",
    ) {
        let _ = decode(&format!("{head}lightning:{tail}"), &DecodeContext::at(SPEC_NOW));
    }
}

// === Mutated valid invoices

proptest! {
    #[test]
    fn deleting_a_character_never_panics(index in any::<prop::sample::Index>()) {
        for invoice in valid_invoices() {
            let mut chars: Vec<char> = invoice.chars().collect();
            chars.remove(index.index(chars.len()));
            let broken: String = chars.into_iter().collect();
            prop_assert!(decode(&broken, &DecodeContext::at(SPEC_NOW)).is_err());
        }
    }

    #[test]
    fn replacing_a_character_never_panics(
        index in any::<prop::sample::Index>(),
        replacement in any::<char>(),
    ) {
        for invoice in valid_invoices() {
            let mut chars: Vec<char> = invoice.chars().collect();
            let i = index.index(chars.len());
            chars[i] = replacement;
            let mutated: String = chars.into_iter().collect();
            let _ = decode(&mutated, &DecodeContext::at(SPEC_NOW));
        }
    }

    #[test]
    fn truncating_never_panics(keep in 0usize..400) {
        for invoice in valid_invoices() {
            let cut: String = invoice.chars().take(keep).collect();
            let _ = decode(&cut, &DecodeContext::at(SPEC_NOW));
        }
    }
}

// === Fixed edge cases

#[test]
fn degenerate_inputs_are_errors() {
    let ctx = DecodeContext::at(SPEC_NOW);
    for input in [
        "",
        " ",
        "1",
        "ln",
        "lnbc",
        "lnbc1",
        "lightning:",
        "LIGHTNING:",
        "lnbc1\u{0}",
        "\u{feff}lnbc1",
    ] {
        assert!(decode(input, &ctx).is_err(), "{input:?} should not decode");
    }
}

#[test]
fn oversized_input_is_rejected_without_blowing_up() {
    let huge = format!("lnbc1{}", "q".repeat(2_000_000));
    assert!(decode(&huge, &DecodeContext::at(SPEC_NOW)).is_err());
}

#[test]
fn a_context_with_extreme_numbers_does_not_overflow() {
    let invoice = valid_invoices().remove(0);
    for now in [0, u64::MAX] {
        let ctx = DecodeContext {
            max_amount_msat: Some(u64::MAX),
            ..DecodeContext::at(now)
        };
        assert!(decode(&invoice, &ctx).is_ok());
    }
}
