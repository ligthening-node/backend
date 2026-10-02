//! JSON views of node state. Amounts follow the decoder's conventions: strings, because u64 can
//! exceed JavaScript's 2^53.

use serde::Serialize;

// === Node

/// Node state for the dashboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct NodeStatus {
    pub node_id: String,
    pub network: String,
    pub is_running: bool,
    pub block_height: u32,
    pub best_block_hash: String,
    /// False until both wallets have finished their first sync with bitcoind.
    pub is_synced: bool,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub last_onchain_sync: Option<u64>,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub last_lightning_sync: Option<u64>,
    /// Where peers can reach this node, as `host:port`.
    pub listening_addresses: Vec<String>,
}

/// Wallet balances in satoshis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Balances {
    #[serde(with = "as_string")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub onchain_total_sat: u64,
    #[serde(with = "as_string")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub onchain_spendable_sat: u64,
    #[serde(with = "as_string")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub anchor_reserve_sat: u64,
    #[serde(with = "as_string")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub lightning_sat: u64,
}

// === Peers and channels

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct PeerView {
    pub node_id: String,
    pub address: String,
    pub is_connected: bool,
    pub is_persisted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct ChannelView {
    pub channel_id: String,
    /// Our own id for the channel (a u128 in decimal); closing a channel needs it.
    pub user_channel_id: String,
    pub counterparty_node_id: String,
    /// `txid:vout` of the funding output, once it exists.
    pub funding_txo: Option<String>,
    /// `block x tx x output`, once the funding transaction is confirmed.
    pub short_channel_id: Option<String>,
    #[serde(with = "as_string")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub capacity_sat: u64,
    /// What we can send right now.
    #[serde(with = "as_string")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub outbound_msat: u64,
    /// What we can receive right now.
    #[serde(with = "as_string")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub inbound_msat: u64,
    /// The largest single payment this channel can carry now. Often well below `outbound_msat`:
    /// the peer caps how much may be in flight (LDK's default is 10% of the channel).
    #[serde(with = "as_string")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub max_send_msat: u64,
    pub is_outbound: bool,
    pub is_channel_ready: bool,
    pub is_usable: bool,
    pub confirmations: Option<u32>,
    pub confirmations_required: Option<u32>,
    /// Everything on our side of the channel, reserve included: what we would claim if it closed
    /// now (before on-chain fees). None until the channel is confirmed.
    #[serde(with = "opt_as_string")]
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub our_balance_sat: Option<u64>,
    /// What we must keep in the channel. It is part of our balance but cannot be spent.
    #[serde(with = "opt_as_string")]
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub our_reserve_sat: Option<u64>,
    /// What the peer must keep in the channel. The first sats they receive fill it, so they do not
    /// show up as spendable for them or as receivable for us.
    #[serde(with = "as_string")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub their_reserve_sat: u64,
}

// === Payments

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum PaymentKindView {
    Onchain,
    Bolt11,
    Bolt11Jit,
    Bolt12Offer,
    Bolt12Refund,
    Spontaneous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Inbound,
    Outbound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(rename_all = "snake_case")]
pub enum PaymentState {
    Pending,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct PaymentView {
    pub id: String,
    pub kind: PaymentKindView,
    pub direction: Direction,
    pub status: PaymentState,
    #[serde(with = "opt_as_string")]
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub amount_msat: Option<u64>,
    #[serde(with = "opt_as_string")]
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    pub fee_paid_msat: Option<u64>,
    pub payment_hash: Option<String>,
    /// Proof of payment. Only present once an outbound payment succeeded or an inbound one was claimed.
    pub preimage: Option<String>,
    pub txid: Option<String>,
    /// When the last status change happened. Moves on every change, for example on confirmation.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub updated_at: u64,
    /// When this node first saw the payment. Fixed once recorded; payments that existed before the
    /// node started tracking this carry their last update time as an estimate.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub first_seen_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct CreatedInvoice {
    pub invoice: String,
    pub payment_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct SentPayment {
    pub payment_id: String,
}

// === Events

/// Live node events, streamed to the browser over SSE.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeEvent {
    PaymentSuccessful {
        payment_hash: String,
        #[serde(with = "opt_as_string")]
        #[cfg_attr(feature = "ts", ts(type = "string | null"))]
        fee_paid_msat: Option<u64>,
    },
    PaymentFailed {
        payment_hash: Option<String>,
        reason: Option<String>,
    },
    PaymentReceived {
        payment_hash: String,
        #[serde(with = "as_string")]
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        amount_msat: u64,
    },
    ChannelPending {
        channel_id: String,
        counterparty_node_id: String,
    },
    ChannelReady {
        channel_id: String,
        counterparty_node_id: Option<String>,
    },
    ChannelClosed {
        channel_id: String,
        reason: Option<String>,
    },
    /// A payment appeared or changed state, including on-chain ones that raise no ldk-node event.
    /// Pages refetch; the UI shows no toast for it.
    PaymentsChanged,
    /// Anything the UI does not render specially, such as forwards and splices.
    Other { name: String },
}

// === Serde helpers

mod as_string {
    use serde::Serializer;

    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        return serializer.collect_str(value);
    }
}

mod opt_as_string {
    use serde::Serializer;

    pub fn serialize<S: Serializer>(value: &Option<u64>, serializer: S) -> Result<S::Ok, S::Error> {
        return match value {
            Some(v) => serializer.collect_str(v),
            None => serializer.serialize_none(),
        };
    }
}

// === Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balances_serialize_as_strings() {
        let balances = Balances {
            onchain_total_sat: 9_007_199_254_740_993,
            onchain_spendable_sat: 1,
            anchor_reserve_sat: 0,
            lightning_sat: 2,
        };
        let json = serde_json::to_value(&balances).unwrap();
        assert_eq!(json["onchain_total_sat"], "9007199254740993");
        assert_eq!(json["lightning_sat"], "2");
    }

    #[test]
    fn events_are_tagged_by_kind() {
        let event = NodeEvent::PaymentReceived {
            payment_hash: "ab".to_string(),
            amount_msat: 1000,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["kind"], "payment_received");
        assert_eq!(json["amount_msat"], "1000");
    }

    #[test]
    fn missing_amounts_are_null() {
        let event = NodeEvent::PaymentSuccessful {
            payment_hash: "ab".to_string(),
            fee_paid_msat: None,
        };
        assert!(serde_json::to_value(&event).unwrap()["fee_paid_msat"].is_null());
    }
}
