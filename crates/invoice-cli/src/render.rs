//! Human-readable output.

use std::fmt::Write;

use chrono::DateTime;
use invoice_core::{
    DecodeError, Decoded, Description, FieldStatus, PayeeSource, SegmentKind, Status, Verdict,
    clean_input, format_scid,
};

const LABEL_WIDTH: usize = 16;
const RAW_PREVIEW: usize = 40;

pub fn decoded(d: &Decoded, now_unix: u64) -> String {
    let inv = &d.invoice;
    let mut out = String::new();

    let verdict = match d.report.verdict {
        Verdict::Payable => "PAYABLE",
        Verdict::NotPayable => "NOT PAYABLE",
        Verdict::Invalid => "INVALID",
    };
    let _ = writeln!(out, "Verdict: {verdict}\n\nInvoice");

    let mut row = |label: &str, value: String| {
        let _ = writeln!(out, "  {label:<LABEL_WIDTH$} {value}");
    };
    row("Network", inv.network.to_string());
    row(
        "Amount",
        inv.amount_msat
            .map_or("any amount (payer decides)".into(), amount),
    );
    row("Created", date(inv.timestamp));
    let default = if inv.expiry_is_default {
        " (default)"
    } else {
        ""
    };
    row(
        "Expiry",
        format!(
            "{}s{default}, until {}",
            inv.expiry_secs,
            date(inv.expires_at)
        ),
    );
    row("Evaluated at", date(now_unix));
    row(
        "Description",
        match &inv.description {
            Some(Description::Direct(text)) => format!("{text:?}"),
            Some(Description::Hash(hash)) => format!("hash {hash}"),
            None => "(missing)".into(),
        },
    );
    row(
        "Payment hash",
        inv.payment_hash
            .map_or("(missing)".into(), |h| h.to_string()),
    );
    row(
        "Payment secret",
        inv.payment_secret
            .map_or("(missing)".into(), |s| s.to_string()),
    );
    let source = match inv.payee.source {
        PayeeSource::Explicit => "from the n field",
        PayeeSource::Recovered => "recovered from the signature",
    };
    row("Payee", format!("{} ({source})", inv.payee.pubkey));
    row("Min final CLTV", inv.min_final_cltv_expiry.to_string());
    if let Some(metadata) = &inv.metadata {
        row("Metadata", metadata.to_string());
    }

    if !inv.route_hints.is_empty() {
        let _ = writeln!(out, "\nRoute hints (paths into private channels)");
        for (i, hint) in inv.route_hints.iter().enumerate() {
            for (j, hop) in hint.iter().enumerate() {
                let _ = writeln!(
                    out,
                    "  route {} hop {}: {} via {} | fee {} msat + {} ppm | cltv delta {}",
                    i + 1,
                    j + 1,
                    hop.pubkey,
                    format_scid(hop.short_channel_id),
                    hop.fee_base_msat,
                    hop.fee_proportional_millionths,
                    hop.cltv_expiry_delta
                );
            }
        }
    }

    if !inv.fallbacks.is_empty() {
        let _ = writeln!(out, "\nOn-chain fallbacks");
        for fallback in &inv.fallbacks {
            let kind = match fallback.version {
                17 => "P2PKH".to_string(),
                18 => "P2SH".to_string(),
                v => format!("segwit v{v}"),
            };
            let _ = writeln!(out, "  {} ({kind})", fallback.address);
        }
    }

    let features = &inv.features;
    if !features.bits.is_empty() {
        let _ = writeln!(out, "\nFeatures");
        for known in &features.known {
            let kind = if known.required {
                "required"
            } else {
                "optional"
            };
            let _ = writeln!(out, "  {:>3}  {} ({kind})", known.bit, known.name);
        }
        for bit in &features.unknown_required {
            let _ = writeln!(out, "  {bit:>3}  unknown (required, cannot pay)");
        }
        for bit in &features.unknown_optional {
            let _ = writeln!(out, "  {bit:>3}  unknown (optional, ignored)");
        }
    }

    let _ = writeln!(out, "\nChecks");
    for check in &d.report.checks {
        let mark = match check.status {
            Status::Pass => "PASS",
            Status::Info => "INFO",
            Status::Warn => "WARN",
            Status::Fail => "FAIL",
            Status::Skipped => "SKIP",
        };
        let id = format!("{:?}", check.id);
        let _ = writeln!(out, "  [{mark}] {id:<LABEL_WIDTH$} {}", check.message);
    }

    let _ = writeln!(out, "\nAnatomy");
    for segment in &d.anatomy {
        let label = match (segment.kind, segment.tag) {
            (SegmentKind::TaggedField, Some(tag)) => format!("field '{tag}'"),
            (kind, _) => format!("{kind:?}").to_lowercase(),
        };
        let words = segment
            .len_words
            .map_or(String::new(), |n| format!("{n} words"));
        let status = match segment.status {
            FieldStatus::Parsed => "",
            FieldStatus::Duplicate => "duplicate, ignored",
            FieldStatus::SkippedBadLength => "bad length, ignored",
            FieldStatus::Invalid => "invalid, ignored",
            FieldStatus::Unknown => "unknown, ignored",
        };
        let _ = writeln!(
            out,
            "  {:>4}  {label:<12} {words:<9} {:<width$} {status}",
            segment.start,
            preview(&segment.raw),
            width = RAW_PREVIEW + 3
        );
    }
    return out;
}

/// Shows where a fatal error happened, with a caret under the offending character when known.
pub fn error(input: &str, err: &DecodeError) -> String {
    let mut out = format!("error: {err}");
    let pos = match err {
        DecodeError::InvalidChar { pos, .. } | DecodeError::FieldOverrun { pos, .. } => *pos,
        _ => return out,
    };
    let cleaned = clean_input(input);
    let start = pos.saturating_sub(RAW_PREVIEW / 2);
    let end = (pos + RAW_PREVIEW / 2).min(cleaned.len());
    if let Some(window) = cleaned.get(start..end) {
        let _ = write!(out, "\n  {window}\n  {}^", " ".repeat(pos - start));
    }
    return out;
}

// === Formatting

fn amount(msat: u64) -> String {
    let sat = format!("{}.{:03}", msat / 1_000, msat % 1_000);
    let btc = format!("{}.{:011}", msat / 100_000_000_000, msat % 100_000_000_000);
    return format!(
        "{msat} msat = {} sat = {} BTC",
        sat.trim_end_matches('0').trim_end_matches('.'),
        btc.trim_end_matches('0').trim_end_matches('.')
    );
}

fn date(unix: u64) -> String {
    let formatted = i64::try_from(unix)
        .ok()
        .and_then(|secs| DateTime::from_timestamp(secs, 0))
        .map(|dt| DateTime::format(&dt, "%Y-%m-%d %H:%M:%S UTC").to_string());
    return match formatted {
        Some(text) => format!("{text} ({unix})"),
        None => unix.to_string(),
    };
}

fn preview(raw: &str) -> String {
    if raw.len() <= RAW_PREVIEW {
        return raw.to_string();
    }
    return format!("{}...", &raw[..RAW_PREVIEW]);
}

// === Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts() {
        assert_eq!(
            amount(250_000_000),
            "250000000 msat = 250000 sat = 0.0025 BTC"
        );
        assert_eq!(
            amount(967_878_534),
            "967878534 msat = 967878.534 sat = 0.00967878534 BTC"
        );
        assert_eq!(
            amount(100_000_000_000),
            "100000000000 msat = 100000000 sat = 1 BTC"
        );
    }

    #[test]
    fn dates() {
        assert_eq!(date(1_496_314_658), "2017-06-01 10:57:38 UTC (1496314658)");
    }
}
