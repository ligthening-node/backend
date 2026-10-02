//! Spots payments that appeared or changed state. ldk-node raises no event for on-chain
//! transactions, so the server compares snapshots and tells the browser when one differs.

use std::collections::HashMap;
use std::sync::Mutex;

/// Payment id to (status, last update time).
pub type Snapshot = HashMap<String, (String, u64)>;

#[derive(Default)]
pub struct PaymentChanges {
    last: Mutex<Option<Snapshot>>,
}

impl PaymentChanges {
    /// True when `next` differs from the previous snapshot. The first call only records the
    /// baseline: payments that existed before the server started are not news.
    pub fn changed(&self, next: Snapshot) -> bool {
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        let differs = last.as_ref().is_some_and(|prev| *prev != next);
        *last = Some(next);
        return differs;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(items: &[(&str, &str, u64)]) -> Snapshot {
        return items
            .iter()
            .map(|(id, status, at)| (id.to_string(), (status.to_string(), *at)))
            .collect();
    }

    #[test]
    fn first_call_is_only_a_baseline() {
        let changes = PaymentChanges::default();
        assert!(!changes.changed(snap(&[("a", "pending", 1)])));
    }

    #[test]
    fn same_snapshot_is_not_a_change() {
        let changes = PaymentChanges::default();
        changes.changed(snap(&[("a", "pending", 1)]));
        assert!(!changes.changed(snap(&[("a", "pending", 1)])));
    }

    #[test]
    fn new_payment_is_a_change() {
        let changes = PaymentChanges::default();
        changes.changed(snap(&[("a", "succeeded", 1)]));
        assert!(changes.changed(snap(&[("a", "succeeded", 1), ("b", "pending", 2)])));
    }

    #[test]
    fn status_change_is_reported_once() {
        let changes = PaymentChanges::default();
        changes.changed(snap(&[("a", "pending", 1)]));
        assert!(changes.changed(snap(&[("a", "succeeded", 2)])));
        assert!(!changes.changed(snap(&[("a", "succeeded", 2)])));
    }
}
