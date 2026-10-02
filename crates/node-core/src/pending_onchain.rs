//! On-chain payments that sit in the mempool. ldk-node lists an on-chain payment only once a
//! block confirms it, so this store keeps unconfirmed ones until ldk-node takes them over:
//! outbound ones are recorded when the node sends, inbound ones are spotted in the mempool by the
//! addresses the node handed out.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// How long a transaction may be absent from the mempool and still unlisted by ldk-node before it
/// is dropped. It covers the short gap between a block arriving and ldk-node listing the payment.
const MISSING_GRACE_SECS: u64 = 30;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingTx {
    pub txid: String,
    pub inbound: bool,
    pub amount_sat: u64,
    pub fee_sat: Option<u64>,
    pub seen_at: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    addresses: HashSet<String>,
    pending: Vec<PendingTx>,
}

#[derive(Default)]
struct State {
    saved: Saved,
    /// Mempool transactions already checked for payments to our addresses.
    examined: HashSet<String>,
    /// When a pending transaction was first noticed missing from the mempool.
    missing_since: HashMap<String, u64>,
}

pub struct PendingOnchain {
    path: PathBuf,
    state: Mutex<State>,
}

impl PendingOnchain {
    /// A missing or unreadable file starts empty.
    pub fn load(path: PathBuf) -> Self {
        let saved = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Saved>(&bytes).ok())
            .unwrap_or_default();
        return Self {
            path,
            state: Mutex::new(State {
                saved,
                ..State::default()
            }),
        };
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        return self.state.lock().unwrap_or_else(|e| e.into_inner());
    }

    pub fn track_address(&self, address: &str) {
        let mut state = self.lock();
        if state.saved.addresses.insert(address.to_string()) {
            self.save(&state.saved);
        }
    }

    pub fn add_outbound(&self, txid: &str, amount_sat: u64, fee_sat: Option<u64>, now: u64) {
        let mut state = self.lock();
        if state.saved.pending.iter().any(|p| p.txid == txid) {
            return;
        }
        state.saved.pending.push(PendingTx {
            txid: txid.to_string(),
            inbound: false,
            amount_sat,
            fee_sat,
            seen_at: now,
        });
        self.save(&state.saved);
    }

    pub fn pending(&self) -> Vec<PendingTx> {
        return self.lock().saved.pending.clone();
    }

    /// Brings the store in line with the chain. `listed` holds the txids ldk-node already lists,
    /// `mempool` the txids in bitcoind's mempool, and `outputs` looks up a transaction's
    /// `(address, sats)` outputs. Returns true when the pending set changed.
    pub fn reconcile(
        &self,
        listed: &HashSet<String>,
        mempool: &HashSet<String>,
        mut outputs: impl FnMut(&str) -> Option<Vec<(String, u64)>>,
        now: u64,
    ) -> bool {
        let mut guard = self.lock();
        let State {
            saved,
            examined,
            missing_since,
        } = &mut *guard;
        let mut changed = false;

        // Drop what ldk-node took over, and what vanished without confirming.
        saved.pending.retain(|p| {
            if listed.contains(&p.txid) {
                changed = true;
                return false;
            }
            if mempool.contains(&p.txid) {
                missing_since.remove(&p.txid);
                return true;
            }
            let since = *missing_since.entry(p.txid.clone()).or_insert(now);
            if now.saturating_sub(since) >= MISSING_GRACE_SECS {
                changed = true;
                return false;
            }
            return true;
        });
        missing_since.retain(|txid, _| saved.pending.iter().any(|p| p.txid == *txid));
        examined.retain(|txid| mempool.contains(txid));

        // Spot new payments to addresses we handed out.
        for txid in mempool {
            if examined.contains(txid)
                || listed.contains(txid)
                || saved.pending.iter().any(|p| p.txid == *txid)
            {
                continue;
            }
            let Some(outs) = outputs(txid) else { continue };
            examined.insert(txid.clone());
            let received: u64 = outs
                .iter()
                .filter(|(address, _)| saved.addresses.contains(address))
                .map(|(_, sats)| sats)
                .sum();
            if received > 0 {
                saved.pending.push(PendingTx {
                    txid: txid.clone(),
                    inbound: true,
                    amount_sat: received,
                    fee_sat: None,
                    seen_at: now,
                });
                changed = true;
            }
        }

        if changed {
            self.save(saved);
        }
        return changed;
    }

    /// Writes to a temp file and renames it. A failed write is logged and never fails a request.
    fn save(&self, saved: &Saved) {
        let tmp = self.path.with_extension("json.tmp");
        let result = serde_json::to_vec(saved)
            .map_err(|e| e.to_string())
            .and_then(|bytes| fs::write(&tmp, bytes).map_err(|e| e.to_string()))
            .and_then(|()| fs::rename(&tmp, &self.path).map_err(|e| e.to_string()));
        if let Err(err) = result {
            eprintln!("could not save pending on-chain payments: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> PendingOnchain {
        let dir = std::env::temp_dir().join(format!("pending-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        return PendingOnchain::load(dir.join("pending.json"));
    }

    fn set(items: &[&str]) -> HashSet<String> {
        return items.iter().map(|s| s.to_string()).collect();
    }

    fn pays(address: &str, sats: u64) -> impl FnMut(&str) -> Option<Vec<(String, u64)>> {
        let address = address.to_string();
        return move |_| Some(vec![(address.clone(), sats), ("other".to_string(), 5)]);
    }

    #[test]
    fn a_payment_to_a_handed_out_address_becomes_pending_inbound() {
        let s = store("inbound");
        s.track_address("mine");
        assert!(s.reconcile(&set(&[]), &set(&["t1"]), pays("mine", 4_000), 10));
        let p = s.pending();
        assert_eq!(p.len(), 1);
        assert!(p[0].inbound);
        assert_eq!(p[0].amount_sat, 4_000);
        assert_eq!(p[0].seen_at, 10);
    }

    #[test]
    fn a_payment_to_someone_else_is_ignored() {
        let s = store("other");
        s.track_address("mine");
        assert!(!s.reconcile(&set(&[]), &set(&["t1"]), pays("stranger", 4_000), 10));
        assert!(s.pending().is_empty());
    }

    #[test]
    fn a_transaction_is_only_looked_up_once() {
        let s = store("once");
        let mut calls = 0;
        for _ in 0..3 {
            s.reconcile(
                &set(&[]),
                &set(&["t1"]),
                |_| {
                    calls += 1;
                    Some(vec![])
                },
                10,
            );
        }
        assert_eq!(calls, 1);
    }

    #[test]
    fn a_failed_lookup_is_retried() {
        let s = store("retry");
        s.track_address("mine");
        assert!(!s.reconcile(&set(&[]), &set(&["t1"]), |_| None, 10));
        assert!(s.reconcile(&set(&[]), &set(&["t1"]), pays("mine", 1), 11));
    }

    #[test]
    fn outbound_is_dropped_when_ldk_node_lists_it() {
        let s = store("handoff");
        s.add_outbound("t1", 3_000, Some(143), 10);
        assert!(!s.reconcile(&set(&[]), &set(&["t1"]), |_| None, 11));
        assert_eq!(s.pending().len(), 1);
        assert!(s.reconcile(&set(&["t1"]), &set(&[]), |_| None, 12));
        assert!(s.pending().is_empty());
    }

    #[test]
    fn a_missing_transaction_survives_the_grace_period_then_goes() {
        let s = store("grace");
        s.add_outbound("t1", 3_000, None, 10);
        assert!(!s.reconcile(&set(&[]), &set(&[]), |_| None, 20));
        assert!(!s.reconcile(&set(&[]), &set(&[]), |_| None, 49));
        assert!(s.reconcile(&set(&[]), &set(&[]), |_| None, 50));
        assert!(s.pending().is_empty());
    }

    #[test]
    fn a_listed_transaction_is_never_added_as_pending() {
        let s = store("listed");
        s.track_address("mine");
        assert!(!s.reconcile(&set(&["t1"]), &set(&["t1"]), pays("mine", 4_000), 10));
        assert!(s.pending().is_empty());
    }

    #[test]
    fn state_survives_a_reload() {
        let s = store("reload");
        s.track_address("mine");
        s.add_outbound("t1", 3_000, None, 10);
        let again = PendingOnchain::load(s.path.clone());
        assert_eq!(again.pending().len(), 1);
        assert!(again.lock().saved.addresses.contains("mine"));
    }
}
