//! Conversions from ldk-node types to the JSON views.

use invoice_core::{format_scid, to_hex};
use ldk_node::payment::{PaymentDetails, PaymentDirection, PaymentKind, PaymentStatus};
use ldk_node::{ChannelDetails, Event, PeerDetails};

use crate::views::{
    ChannelView, Direction, NodeEvent, PaymentKindView, PaymentState, PaymentView, PeerView,
};

pub fn peer(p: &PeerDetails) -> PeerView {
    return PeerView {
        node_id: p.node_id.to_string(),
        address: p.address.to_string(),
        is_connected: p.is_connected,
        is_persisted: p.is_persisted,
    };
}

/// `our_balance_sat` comes from the node's balance list, which is keyed by channel id.
pub fn channel(c: &ChannelDetails, our_balance_sat: Option<u64>) -> ChannelView {
    return ChannelView {
        channel_id: to_hex(&c.channel_id.0),
        user_channel_id: c.user_channel_id.0.to_string(),
        counterparty_node_id: c.counterparty_node_id.to_string(),
        funding_txo: c.funding_txo.map(|o| o.to_string()),
        short_channel_id: c.short_channel_id.map(format_scid),
        capacity_sat: c.channel_value_sats,
        outbound_msat: c.outbound_capacity_msat,
        inbound_msat: c.inbound_capacity_msat,
        max_send_msat: c.next_outbound_htlc_limit_msat,
        is_outbound: c.is_outbound,
        is_channel_ready: c.is_channel_ready,
        is_usable: c.is_usable,
        confirmations: c.confirmations,
        confirmations_required: c.confirmations_required,
        our_balance_sat,
        our_reserve_sat: c.unspendable_punishment_reserve,
        their_reserve_sat: c.counterparty_unspendable_punishment_reserve,
    };
}

pub fn payment(p: &PaymentDetails, first_seen_at: u64) -> PaymentView {
    let (kind, hash, preimage, txid) = match &p.kind {
        PaymentKind::Onchain { txid, .. } => {
            (PaymentKindView::Onchain, None, None, Some(txid.to_string()))
        }
        PaymentKind::Bolt11 { hash, preimage, .. } => (
            PaymentKindView::Bolt11,
            Some(hash.0),
            preimage.map(|p| p.0),
            None,
        ),
        PaymentKind::Bolt11Jit { hash, preimage, .. } => (
            PaymentKindView::Bolt11Jit,
            Some(hash.0),
            preimage.map(|p| p.0),
            None,
        ),
        PaymentKind::Bolt12Offer { hash, preimage, .. } => (
            PaymentKindView::Bolt12Offer,
            hash.map(|h| h.0),
            preimage.map(|p| p.0),
            None,
        ),
        PaymentKind::Bolt12Refund { hash, preimage, .. } => (
            PaymentKindView::Bolt12Refund,
            hash.map(|h| h.0),
            preimage.map(|p| p.0),
            None,
        ),
        PaymentKind::Spontaneous { hash, preimage } => (
            PaymentKindView::Spontaneous,
            Some(hash.0),
            preimage.map(|p| p.0),
            None,
        ),
    };
    return PaymentView {
        id: to_hex(&p.id.0),
        kind,
        direction: match p.direction {
            PaymentDirection::Inbound => Direction::Inbound,
            PaymentDirection::Outbound => Direction::Outbound,
        },
        status: match p.status {
            PaymentStatus::Pending => PaymentState::Pending,
            PaymentStatus::Succeeded => PaymentState::Succeeded,
            PaymentStatus::Failed => PaymentState::Failed,
        },
        amount_msat: p.amount_msat,
        fee_paid_msat: p.fee_paid_msat,
        payment_hash: hash.map(|h| to_hex(&h)),
        preimage: preimage.map(|p| to_hex(&p)),
        txid,
        updated_at: p.latest_update_timestamp,
        first_seen_at,
    };
}

pub fn event(e: &Event) -> NodeEvent {
    return match e {
        Event::PaymentSuccessful {
            payment_hash,
            fee_paid_msat,
            ..
        } => NodeEvent::PaymentSuccessful {
            payment_hash: to_hex(&payment_hash.0),
            fee_paid_msat: *fee_paid_msat,
        },
        Event::PaymentFailed {
            payment_hash,
            reason,
            ..
        } => NodeEvent::PaymentFailed {
            payment_hash: payment_hash.map(|h| to_hex(&h.0)),
            reason: reason.map(|r| format!("{r:?}")),
        },
        Event::PaymentReceived {
            payment_hash,
            amount_msat,
            ..
        } => NodeEvent::PaymentReceived {
            payment_hash: to_hex(&payment_hash.0),
            amount_msat: *amount_msat,
        },
        Event::ChannelPending {
            channel_id,
            counterparty_node_id,
            ..
        } => NodeEvent::ChannelPending {
            channel_id: to_hex(&channel_id.0),
            counterparty_node_id: counterparty_node_id.to_string(),
        },
        Event::ChannelReady {
            channel_id,
            counterparty_node_id,
            ..
        } => NodeEvent::ChannelReady {
            channel_id: to_hex(&channel_id.0),
            counterparty_node_id: counterparty_node_id.map(|k| k.to_string()),
        },
        Event::ChannelClosed {
            channel_id, reason, ..
        } => NodeEvent::ChannelClosed {
            channel_id: to_hex(&channel_id.0),
            reason: reason.as_ref().map(|r| r.to_string()),
        },
        Event::PaymentForwarded { .. } => other("payment_forwarded"),
        Event::PaymentClaimable { .. } => other("payment_claimable"),
        Event::SplicePending { .. } => other("splice_pending"),
        Event::SpliceFailed { .. } => other("splice_failed"),
    };
}

fn other(name: &str) -> NodeEvent {
    return NodeEvent::Other {
        name: name.to_string(),
    };
}
