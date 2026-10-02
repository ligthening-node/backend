//! Builds the `ValidationReport`. The verdict is always derived from the checks, never set directly.

use bitcoin::hashes::{Hash, sha256};

use crate::decoded::{
    Check, CheckId, DecodeContext, DecodedInvoice, Description, FieldStatus, RawField, Status,
    ValidationReport, Verdict,
};
use crate::fields::Fields;
use crate::signature::SigOutcome;

const EXPIRY_WARN_SECS: u64 = 60;

pub fn run(
    invoice: &DecodedInvoice,
    fields: &Fields,
    sig: &SigOutcome,
    ctx: &DecodeContext,
) -> ValidationReport {
    let checks = vec![
        signature(sig),
        expected_payee(invoice, ctx),
        expiry(invoice, ctx),
        network(invoice, ctx),
        amount(invoice, ctx),
        description_hash(invoice, ctx),
        required_fields(fields),
        features(invoice),
        field_encoding(fields),
        unknown_fields(&fields.raw),
    ];
    return ValidationReport {
        verdict: verdict(&checks),
        checks,
    };
}

fn verdict(checks: &[Check]) -> Verdict {
    let failed = || checks.iter().filter(|c| c.status == Status::Fail);
    if failed().any(|c| c.id.is_structural()) {
        return Verdict::Invalid;
    }
    if failed().next().is_some() {
        return Verdict::NotPayable;
    }
    return Verdict::Payable;
}

fn check(id: CheckId, status: Status, message: impl Into<String>) -> Check {
    return Check {
        id,
        status,
        message: message.into(),
    };
}

// === Checks

fn signature(sig: &SigOutcome) -> Check {
    let id = CheckId::Signature;
    return match sig {
        SigOutcome::VerifiedExplicit => check(
            id,
            Status::Pass,
            "Signature verifies against the payee key in the n field",
        ),
        SigOutcome::Recovered(_) => check(
            id,
            Status::Info,
            "No n field: payee key recovered from the signature. This proves who signed, not that it is who you expect",
        ),
        SigOutcome::ExplicitMismatch { high_s: true, .. } => check(
            id,
            Status::Fail,
            "Signature is high-S (non-canonical); with an n field only low-S signatures are accepted",
        ),
        SigOutcome::ExplicitMismatch { recovered, .. } => {
            let detail = match recovered {
                Some(key) => format!(" (it was signed by {key})"),
                None => String::new(),
            };
            check(
                id,
                Status::Fail,
                format!("Signature does not match the payee key in the n field{detail}"),
            )
        }
    };
}

fn expected_payee(invoice: &DecodedInvoice, ctx: &DecodeContext) -> Check {
    let id = CheckId::ExpectedPayee;
    return match &ctx.expected_payee {
        None => check(id, Status::Skipped, "No expected payee given"),
        Some(key) if *key == invoice.payee.pubkey => {
            check(id, Status::Pass, "Payee matches the expected key")
        }
        Some(key) => check(
            id,
            Status::Fail,
            format!("Payee {} is not the expected {key}", invoice.payee.pubkey),
        ),
    };
}

fn expiry(invoice: &DecodedInvoice, ctx: &DecodeContext) -> Check {
    let id = CheckId::Expiry;
    let now = ctx.now_unix;
    if now >= invoice.expires_at {
        return check(
            id,
            Status::Fail,
            format!("Expired {} ago", human_duration(now - invoice.expires_at)),
        );
    }
    let remaining = invoice.expires_at - now;
    let status = if remaining < EXPIRY_WARN_SECS {
        Status::Warn
    } else {
        Status::Pass
    };
    return check(
        id,
        status,
        format!("Expires in {}", human_duration(remaining)),
    );
}

fn network(invoice: &DecodedInvoice, ctx: &DecodeContext) -> Check {
    let id = CheckId::Network;
    return match ctx.expected_network {
        None => check(
            id,
            Status::Info,
            format!("Invoice is for {}", invoice.network),
        ),
        Some(expected) if expected == invoice.network => check(
            id,
            Status::Pass,
            format!("Invoice is for {expected}, as expected"),
        ),
        Some(expected) => check(
            id,
            Status::Fail,
            format!(
                "Invoice is for {}, but {expected} was expected",
                invoice.network
            ),
        ),
    };
}

fn amount(invoice: &DecodedInvoice, ctx: &DecodeContext) -> Check {
    let id = CheckId::Amount;
    return match (invoice.amount_msat, ctx.max_amount_msat) {
        (Some(amount), Some(max)) if amount > max => check(
            id,
            Status::Fail,
            format!("{amount} msat is above the {max} msat limit"),
        ),
        (Some(amount), Some(max)) => check(
            id,
            Status::Pass,
            format!("{amount} msat is within the {max} msat limit"),
        ),
        (Some(amount), None) => check(id, Status::Info, format!("{amount} msat")),
        (None, Some(max)) => check(
            id,
            Status::Warn,
            format!("Any-amount invoice: the payer chooses, the {max} msat limit still applies"),
        ),
        (None, None) => check(
            id,
            Status::Info,
            "Any-amount invoice: the payer chooses how much to send",
        ),
    };
}

fn description_hash(invoice: &DecodedInvoice, ctx: &DecodeContext) -> Check {
    let id = CheckId::DescriptionHash;
    let Some(Description::Hash(hash)) = &invoice.description else {
        return check(id, Status::Skipped, "Invoice has no description hash");
    };
    let Some(preimage) = &ctx.description_preimage else {
        return check(
            id,
            Status::Warn,
            "Only a description hash is present; the text cannot be shown",
        );
    };
    if sha256::Hash::hash(preimage.as_bytes()).to_byte_array() == hash.0 {
        return check(id, Status::Pass, "Provided description matches the hash");
    }
    return check(
        id,
        Status::Fail,
        "Provided description does not match the hash",
    );
}

fn required_fields(fields: &Fields) -> Check {
    let id = CheckId::RequiredFields;
    let mut problems = Vec::new();
    if fields.payment_hash_count != 1 {
        problems.push(format!(
            "expected exactly one payment hash (p), found {}",
            fields.payment_hash_count
        ));
    }
    if fields.payment_secret_count != 1 {
        problems.push(format!(
            "expected exactly one payment secret (s), found {}",
            fields.payment_secret_count
        ));
    }
    let descriptions = fields.description_count + fields.description_hash_count;
    if descriptions != 1 {
        problems.push(format!(
            "expected exactly one description (d) or description hash (h), found {descriptions}"
        ));
    }
    if problems.is_empty() {
        return check(
            id,
            Status::Pass,
            "Payment hash, payment secret and description are present",
        );
    }
    return check(id, Status::Fail, problems.join("; "));
}

fn features(invoice: &DecodedInvoice) -> Check {
    let id = CheckId::Features;
    let features = &invoice.features;
    if !features.unknown_required.is_empty() {
        return check(
            id,
            Status::Fail,
            format!(
                "Unknown required (even) feature bits {:?}: cannot pay",
                features.unknown_required
            ),
        );
    }
    if !features.unknown_optional.is_empty() {
        return check(
            id,
            Status::Info,
            format!(
                "Unknown optional (odd) feature bits {:?} ignored",
                features.unknown_optional
            ),
        );
    }
    return check(id, Status::Pass, "All feature bits are understood");
}

fn field_encoding(fields: &Fields) -> Check {
    let id = CheckId::FieldEncoding;
    if fields.non_minimal.is_empty() {
        return check(id, Status::Pass, "Integer fields use minimal encoding");
    }
    let tags: Vec<String> = fields.non_minimal.iter().map(char::to_string).collect();
    return check(
        id,
        Status::Warn,
        format!(
            "Fields {} start with a zero word (non-minimal encoding)",
            tags.join(", ")
        ),
    );
}

fn unknown_fields(raw: &[RawField]) -> Check {
    let id = CheckId::UnknownFields;
    let count = |status: FieldStatus| raw.iter().filter(|f| f.status == status).count();
    let skipped = count(FieldStatus::SkippedBadLength) + count(FieldStatus::Invalid);
    let unknown = count(FieldStatus::Unknown);
    let duplicate = count(FieldStatus::Duplicate);

    if skipped + unknown + duplicate == 0 {
        return check(id, Status::Pass, "Every tagged field was read");
    }
    let status = if skipped > 0 {
        Status::Warn
    } else {
        Status::Info
    };
    return check(
        id,
        status,
        format!(
            "{unknown} unknown, {skipped} malformed and {duplicate} duplicate fields were ignored"
        ),
    );
}

// === Helpers

fn human_duration(secs: u64) -> String {
    const UNITS: [(u64, &str); 5] = [
        (365 * 86_400, "y"),
        (86_400, "d"),
        (3_600, "h"),
        (60, "m"),
        (1, "s"),
    ];
    let mut rest = secs;
    let mut parts = Vec::new();
    for (size, unit) in UNITS {
        if rest >= size {
            parts.push(format!("{}{unit}", rest / size));
            rest %= size;
        }
        if parts.len() == 2 {
            break;
        }
    }
    if parts.is_empty() {
        return "0s".to_string();
    }
    return parts.join(" ");
}

// === Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(human_duration(0), "0s");
        assert_eq!(human_duration(42), "42s");
        assert_eq!(human_duration(3_661), "1h 1m");
        assert_eq!(human_duration(400 * 86_400), "1y 35d");
    }
}
