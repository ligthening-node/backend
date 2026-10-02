//! Minimal bitcoind JSON-RPC client for reading the mempool. ldk-node lists an on-chain payment
//! only once a block confirms it, so unconfirmed ones are read from bitcoind directly.

use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::Duration;

use serde_json::{Value, json};

const TIMEOUT: Duration = Duration::from_secs(5);
const SATS_PER_BTC: f64 = 100_000_000.0;

/// reqwest's blocking client owns a runtime, which panics when created or dropped inside async
/// code. A static is first used on a blocking thread (see `LightningNode`) and is never dropped.
static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();

fn client() -> &'static reqwest::blocking::Client {
    return CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .expect("an HTTP client with a timeout builds")
    });
}

pub struct Bitcoind {
    url: String,
    user: String,
    password: String,
}

impl Bitcoind {
    pub fn new(host: &str, port: u16, user: &str, password: &str) -> Self {
        return Self {
            url: format!("http://{host}:{port}/"),
            user: user.to_string(),
            password: password.to_string(),
        };
    }

    fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let response = client()
            .post(&self.url)
            .basic_auth(&self.user, Some(&self.password))
            .json(&json!({"jsonrpc": "1.0", "id": "node-core", "method": method, "params": params}))
            .send()
            .map_err(|e| e.to_string())?;
        let body: Value = response.json().map_err(|e| e.to_string())?;
        if let Some(message) = body["error"]["message"].as_str() {
            return Err(format!("{method}: {message}"));
        }
        return Ok(body["result"].clone());
    }

    pub fn mempool_txids(&self) -> Result<HashSet<String>, String> {
        let result = self.call("getrawmempool", json!([false]))?;
        return Ok(result
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default());
    }

    /// `(address, sats)` for every output that has an address. None when the lookup fails, for
    /// example because the transaction was just mined away.
    pub fn outputs(&self, txid: &str) -> Option<Vec<(String, u64)>> {
        let tx = self.call("getrawtransaction", json!([txid, true])).ok()?;
        return Some(parse_outputs(&tx));
    }

    /// The fee in sats of a transaction that is in the mempool.
    pub fn fee_sat(&self, txid: &str) -> Option<u64> {
        let entry = self.call("getmempoolentry", json!([txid])).ok()?;
        return entry["fees"]["base"].as_f64().map(btc_to_sat);
    }
}

fn btc_to_sat(btc: f64) -> u64 {
    return (btc * SATS_PER_BTC).round() as u64;
}

fn parse_outputs(tx: &Value) -> Vec<(String, u64)> {
    return tx["vout"]
        .as_array()
        .map(|outputs| {
            outputs
                .iter()
                .filter_map(|out| {
                    let address = out["scriptPubKey"]["address"].as_str()?;
                    let sats = btc_to_sat(out["value"].as_f64()?);
                    Some((address.to_string(), sats))
                })
                .collect()
        })
        .unwrap_or_default();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn btc_converts_to_whole_sats() {
        assert_eq!(btc_to_sat(0.00001111), 1_111);
        assert_eq!(btc_to_sat(0.00000143), 143);
    }

    #[test]
    fn outputs_skip_entries_without_an_address() {
        let tx = json!({"vout": [
            {"value": 0.00003, "scriptPubKey": {"address": "bcrt1qa"}},
            {"value": 0.0, "scriptPubKey": {"type": "nulldata"}},
        ]});
        assert_eq!(parse_outputs(&tx), vec![("bcrt1qa".to_string(), 3_000)]);
    }
}
