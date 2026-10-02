//! Browser entry point for `invoice-core`.
//!
//! Results cross the boundary as JSON strings: the frontend parses them into the TypeScript types
//! that ts-rs generates from the same Rust structs, so both sides share one schema.

use invoice_core::{DecodeContext, DecodeError, Decoded};
use serde::Serialize;
use wasm_bindgen::prelude::wasm_bindgen;

#[derive(Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DecodeResult {
    Ok {
        decoded: Box<Decoded>,
    },
    Error {
        error: DecodeError,
        message: String,
    },
    /// The context JSON from the caller could not be parsed.
    InvalidContext {
        message: String,
    },
}

/// Decodes `input` with `ctx_json` (a JSON `DecodeContext`) and returns a JSON `DecodeResult`.
#[wasm_bindgen]
pub fn decode(input: &str, ctx_json: &str) -> String {
    let result = match serde_json::from_str::<DecodeContext>(ctx_json) {
        Err(err) => DecodeResult::InvalidContext {
            message: err.to_string(),
        },
        Ok(ctx) => match invoice_core::decode(input, &ctx) {
            Ok(decoded) => DecodeResult::Ok {
                decoded: Box::new(decoded),
            },
            Err(error) => DecodeResult::Error {
                message: error.to_string(),
                error,
            },
        },
    };
    return serde_json::to_string(&result).expect("output types serialize");
}

// === Tests

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const COFFEE: &str = "lnbc2500u1pvjluezsp5zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zygspp5qqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqypqdq5xysxxatsyp3k7enxv4jsxqzpu9qrsgquk0rl77nj30yxdy8j9vdx85fkpmdla2087ne0xh8nhedh8w27kyke0lp53ut353s06fv3qfegext0eh0ymjpf39tuven09sam30g4vgpfna3rh";

    fn call(input: &str, ctx: &str) -> Value {
        return serde_json::from_str(&decode(input, ctx)).unwrap();
    }

    #[test]
    fn ok_result() {
        let ctx =
            r#"{"now_unix":1496314658,"expected_network":"bitcoin","max_amount_msat":"300000000"}"#;
        let out = call(COFFEE, ctx);
        assert_eq!(out["status"], "ok");
        assert_eq!(out["decoded"]["invoice"]["amount_msat"], "250000000");
        assert_eq!(out["decoded"]["report"]["verdict"], "payable");
    }

    #[test]
    fn expected_payee_from_json() {
        let ctx = r#"{"now_unix":1496314658,"expected_payee":"03e7156ae33b0a208d0744199163177e909e80176e55d97a2f221ede0f934dd9ad"}"#;
        let checks = call(COFFEE, ctx)["decoded"]["report"]["checks"].clone();
        let payee = checks
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == "expected_payee")
            .unwrap()
            .clone();
        assert_eq!(payee["status"], "pass");
    }

    #[test]
    fn decode_error_result() {
        let out = call("lnbc1nope", r#"{"now_unix":0}"#);
        assert_eq!(out["status"], "error");
        assert!(out["error"]["code"].is_string());
        assert!(out["message"].is_string());
    }

    #[test]
    fn invalid_context_result() {
        let out = call(COFFEE, r#"{"now_unix":"soon"}"#);
        assert_eq!(out["status"], "invalid_context");
    }
}
