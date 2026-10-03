use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ldk_node::bitcoin::secp256k1::PublicKey;
use ldk_node::bitcoin::{Address, Network};
use ldk_node::lightning::ln::msgs::SocketAddress;
use ldk_node::lightning_invoice::{Bolt11Invoice, Bolt11InvoiceDescription, Description};
use ldk_node::lightning::ln::types::ChannelId;
use ldk_node::payment::PaymentKind;
use ldk_node::LightningBalance;
use ldk_node::{Builder, UserChannelId};

use crate::config::NodeConfig;
use crate::error::NodeError;
use crate::first_seen::FirstSeen;
use crate::map;
use crate::mempool::Bitcoind;
use crate::payment_changes::{PaymentChanges, Snapshot};
use crate::pending_onchain::{PendingOnchain, PendingTx};
use crate::views::{
    Balances, ChannelView, CreatedInvoice, Direction, NodeEvent, NodeStatus, PaymentKindView,
    PaymentState, PaymentView, PeerView, SentPayment,
};

/// Smallest channel that can work. The peer must find a 1,000 sat reserve on each side after the
/// opening costs: about 500 sat for the first commitment (the vendored ldk-node pins the regtest fee
/// rate) and 660 sat for the two anchor outputs. 1,000 + 500 + 660 is about 2,160 sat, and 2,300 sat
/// was the smallest size that opened and became usable in testing. Anything smaller is accepted by
/// the opener and then closed by the peer ("Suitable channel reserve not found"), so it is refused
/// here with the reason instead.
pub const MIN_CHANNEL_SAT: u64 = 2_300;
/// What must stay on the opener's side after the push, to cover the same costs.
pub const MIN_OUR_SIDE_SAT: u64 = 2_300;

/// Cheap to clone: every clone shares the same running node.
///
/// ldk-node blocks on its own runtime inside most calls, which needs a multi-thread tokio runtime
/// when called from async code.
#[derive(Clone)]
pub struct LightningNode {
    inner: Arc<ldk_node::Node>,
    first_seen: Arc<FirstSeen>,
    payment_changes: Arc<PaymentChanges>,
    bitcoind: Arc<Bitcoind>,
    pending: Arc<PendingOnchain>,
}

impl LightningNode {
    // === Lifecycle

    /// Builds the node without starting it. The seed is created on first run inside `data_dir`.
    pub fn build(config: &NodeConfig) -> Result<Self, NodeError> {
        let listen = SocketAddress::from_str(&config.listen_address).map_err(|_| {
            NodeError::Config(format!("bad listen address {:?}", config.listen_address))
        })?;

        let mut builder = Builder::new();
        builder.set_network(config.network);
        builder.set_storage_dir_path(config.data_dir.to_string_lossy().into_owned());
        builder.set_chain_source_bitcoind_rpc(
            config.rpc_host.clone(),
            config.rpc_port,
            config.rpc_user.clone(),
            config.rpc_password.clone(),
        );
        builder
            .set_listening_addresses(vec![listen])
            .map_err(|e| NodeError::Config(e.to_string()))?;
        builder
            .set_node_alias(config.alias.clone())
            .map_err(|e| NodeError::Config(e.to_string()))?;

        let node = builder
            .build()
            .map_err(|e| NodeError::Build(e.to_string()))?;
        return Ok(Self {
            inner: Arc::new(node),
            first_seen: Arc::new(FirstSeen::load(
                config.data_dir.join("payments_first_seen.json"),
            )),
            payment_changes: Arc::new(PaymentChanges::default()),
            bitcoind: Arc::new(Bitcoind::new(
                &config.rpc_host,
                config.rpc_port,
                &config.rpc_user,
                &config.rpc_password,
            )),
            pending: Arc::new(PendingOnchain::load(
                config.data_dir.join("pending_onchain.json"),
            )),
        });
    }

    pub fn start(&self) -> Result<(), NodeError> {
        return Ok(self.inner.start()?);
    }

    pub fn stop(&self) -> Result<(), NodeError> {
        return Ok(self.inner.stop()?);
    }

    pub fn network(&self) -> Network {
        return self.inner.config().network;
    }

    pub fn status(&self) -> NodeStatus {
        let status = self.inner.status();
        let onchain = status.latest_onchain_wallet_sync_timestamp;
        let lightning = status.latest_lightning_wallet_sync_timestamp;
        return NodeStatus {
            node_id: self.inner.node_id().to_string(),
            network: self.network().to_string(),
            is_running: status.is_running,
            block_height: status.current_best_block.height,
            best_block_hash: status.current_best_block.block_hash.to_string(),
            is_synced: onchain.is_some() && lightning.is_some(),
            last_onchain_sync: onchain,
            last_lightning_sync: lightning,
            listening_addresses: self
                .inner
                .listening_addresses()
                .unwrap_or_default()
                .iter()
                .map(|a| a.to_string())
                .collect(),
        };
    }

    /// Pulls the latest chain state now instead of waiting for the background sync.
    pub fn sync(&self) -> Result<(), NodeError> {
        return Ok(self.inner.sync_wallets()?);
    }

    // === On-chain wallet

    pub fn new_address(&self) -> Result<String, NodeError> {
        let address = self.inner.onchain_payment().new_address()?.to_string();
        // Remembered so a payment to it is spotted in the mempool before it confirms.
        self.pending.track_address(&address);
        return Ok(address);
    }

    pub fn balances(&self) -> Balances {
        let b = self.inner.list_balances();
        return Balances {
            onchain_total_sat: b.total_onchain_balance_sats,
            onchain_spendable_sat: b.spendable_onchain_balance_sats,
            anchor_reserve_sat: b.total_anchor_channels_reserve_sats,
            lightning_sat: b.total_lightning_balance_sats,
        };
    }

    /// Sends on-chain and returns the txid.
    pub fn send_onchain(&self, address: &str, amount_sat: u64) -> Result<String, NodeError> {
        let address = Address::from_str(address)
            .map_err(|e| NodeError::InvalidInput(format!("bad address: {e}")))?
            .require_network(self.network())
            .map_err(|_| {
                NodeError::InvalidInput(format!("address is not for {}", self.network()))
            })?;
        let txid = self
            .inner
            .onchain_payment()
            .send_to_address(&address, amount_sat, None)?;
        let txid = txid.to_string();
        // ldk-node lists it only once a block confirms it, so show it as pending meanwhile.
        let fee_sat = self.bitcoind.fee_sat(&txid);
        self.pending.add_outbound(&txid, amount_sat, fee_sat, now_secs());
        return Ok(txid);
    }

    // === Peers and channels

    pub fn connect(&self, node_id: &str, address: &str) -> Result<(), NodeError> {
        let (node_id, address) = parse_peer(node_id, address)?;
        return Ok(self.inner.connect(node_id, address, true)?);
    }

    /// Drops the connection and forgets the peer, so the node does not reconnect after a restart.
    pub fn disconnect(&self, node_id: &str) -> Result<(), NodeError> {
        return Ok(self.inner.disconnect(parse_pubkey(node_id)?)?);
    }

    pub fn list_peers(&self) -> Vec<PeerView> {
        return self.inner.list_peers().iter().map(map::peer).collect();
    }

    /// Opens an unannounced channel and returns its `user_channel_id`.
    pub fn open_channel(
        &self,
        node_id: &str,
        address: &str,
        amount_sat: u64,
        push_msat: Option<u64>,
    ) -> Result<String, NodeError> {
        let (node_id, address) = parse_peer(node_id, address)?;
        let connected = self
            .inner
            .list_peers()
            .iter()
            .any(|peer| peer.node_id == node_id && peer.is_connected);
        check_peer_connected(connected)?;
        check_channel_size(amount_sat, push_msat)?;
        let id = self
            .inner
            .open_channel(node_id, address, amount_sat, push_msat, None)?;
        return Ok(id.0.to_string());
    }

    pub fn close_channel(
        &self,
        user_channel_id: &str,
        counterparty_node_id: &str,
        force: bool,
    ) -> Result<(), NodeError> {
        let id = user_channel_id
            .parse::<u128>()
            .map(UserChannelId)
            .map_err(|_| NodeError::InvalidInput("bad user_channel_id".to_string()))?;
        let counterparty = parse_pubkey(counterparty_node_id)?;
        if force {
            return Ok(self.inner.force_close_channel(&id, counterparty, None)?);
        }
        return Ok(self.inner.close_channel(&id, counterparty)?);
    }

    pub fn list_channels(&self) -> Vec<ChannelView> {
        // Our total share of each channel is in the balance list, keyed by channel id.
        let mut shares: HashMap<ChannelId, u64> = HashMap::new();
        for balance in self.inner.list_balances().lightning_balances {
            if let LightningBalance::ClaimableOnChannelClose {
                channel_id,
                amount_satoshis,
                ..
            } = balance
            {
                shares.insert(channel_id, amount_satoshis);
            }
        }
        return self
            .inner
            .list_channels()
            .iter()
            .map(|c| map::channel(c, shares.get(&c.channel_id).copied()))
            .collect();
    }

    // === Lightning payments

    /// Creates a BOLT11 invoice. `None` makes an any-amount invoice.
    pub fn create_invoice(
        &self,
        amount_msat: Option<u64>,
        description: &str,
        expiry_secs: u32,
    ) -> Result<CreatedInvoice, NodeError> {
        let description = Description::new(description.to_string())
            .map_err(|e| NodeError::InvalidInput(format!("bad description: {e}")))?;
        let description = Bolt11InvoiceDescription::Direct(description);
        let bolt11 = self.inner.bolt11_payment();
        let invoice = match amount_msat {
            Some(amount) => bolt11.receive(amount, &description, expiry_secs)?,
            None => bolt11.receive_variable_amount(&description, expiry_secs)?,
        };
        return Ok(CreatedInvoice {
            invoice: invoice.to_string(),
            payment_hash: invoice.payment_hash().to_string(),
        });
    }

    /// Starts paying an invoice; the outcome arrives later as an event. Callers should run the
    /// invoice through `invoice-core` first: this only checks what ldk-node itself enforces.
    pub fn pay_invoice(
        &self,
        invoice: &str,
        amount_msat: Option<u64>,
    ) -> Result<SentPayment, NodeError> {
        let invoice = Bolt11Invoice::from_str(invoice.trim())
            .map_err(|e| NodeError::InvalidInput(format!("bad invoice: {e}")))?;
        // Say why a payment cannot go out, before ldk-node answers with a bare "failed".
        if let Some(amount) = invoice.amount_milli_satoshis().or(amount_msat) {
            if let Some(why) = crate::liquidity::explain(amount, &self.list_channels()) {
                return Err(NodeError::Liquidity(why));
            }
        }
        let bolt11 = self.inner.bolt11_payment();
        let id = match (invoice.amount_milli_satoshis(), amount_msat) {
            (Some(_), None) => bolt11.send(&invoice, None)?,
            (None, Some(amount)) => bolt11.send_using_amount(&invoice, amount, None)?,
            (Some(_), Some(_)) => {
                return Err(NodeError::InvalidInput(
                    "the invoice already sets an amount".to_string(),
                ));
            }
            (None, None) => {
                return Err(NodeError::InvalidInput(
                    "any-amount invoice: an amount is required".to_string(),
                ));
            }
        };
        // Outbound payments get their exact start time instead of waiting for the next check.
        self.record_new_payments();
        return Ok(SentPayment {
            payment_id: invoice_core::to_hex(&id.0),
        });
    }

    /// Stores the first-seen time of any payment not recorded yet. Cheap: it reads ldk-node's
    /// in-memory list, so the server calls it every couple of seconds.
    pub fn record_new_payments(&self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // A payment shown as pending keeps that first-seen time once ldk-node lists it.
        let pending = self.pending.pending();
        for p in self.inner.list_payments() {
            if let PaymentKind::Onchain { txid, .. } = &p.kind {
                if let Some(tx) = pending.iter().find(|t| t.txid == txid.to_string()) {
                    self.first_seen
                        .record_at(&invoice_core::to_hex(&p.id.0), tx.seen_at);
                }
            }
        }
        self.first_seen.record_new(
            self.inner
                .list_payments()
                .iter()
                .map(|p| (invoice_core::to_hex(&p.id.0), p.latest_update_timestamp)),
            now,
        );
    }

    /// True when a payment appeared or changed state since the last call. The server polls this
    /// to push on-chain activity to the browser, since ldk-node has no event for it.
    pub fn payments_changed(&self) -> bool {
        let snapshot: Snapshot = self
            .payment_views()
            .iter()
            .map(|p| (p.id.clone(), (format!("{:?}", p.status), p.updated_at)))
            .collect();
        return self.payment_changes.changed(snapshot);
    }

    /// Reads bitcoind's mempool to keep the unconfirmed on-chain payments up to date. Does nothing
    /// when bitcoind cannot be reached.
    pub fn refresh_pending(&self) {
        let Ok(mempool) = self.bitcoind.mempool_txids() else {
            return;
        };
        let listed: HashSet<String> = self
            .inner
            .list_payments()
            .iter()
            .filter_map(|p| match &p.kind {
                PaymentKind::Onchain { txid, .. } => Some(txid.to_string()),
                _ => None,
            })
            .collect();
        self.pending.reconcile(
            &listed,
            &mempool,
            |txid| self.bitcoind.outputs(txid),
            now_secs(),
        );
    }

    /// Newest first, by when each payment first appeared.
    pub fn list_payments(&self) -> Vec<PaymentView> {
        self.record_new_payments();
        let mut payments = self.payment_views();
        payments.sort_by_key(|p| std::cmp::Reverse((p.first_seen_at, p.updated_at)));
        return payments;
    }

    /// ldk-node's payments plus the on-chain ones still waiting in the mempool.
    fn payment_views(&self) -> Vec<PaymentView> {
        let mut payments: Vec<PaymentView> = self
            .inner
            .list_payments()
            .iter()
            .map(|p| {
                let id = invoice_core::to_hex(&p.id.0);
                map::payment(p, self.first_seen.get(&id, p.latest_update_timestamp))
            })
            .collect();
        payments.extend(self.pending.pending().iter().map(pending_view));
        return payments;
    }

    // === Events

    /// Waits for the next node event and marks it handled. Only one task should call this: ldk-node
    /// has a single event queue and redelivers anything not marked handled.
    pub async fn next_event(&self) -> Result<NodeEvent, NodeError> {
        let event = self.inner.next_event_async().await;
        let view = map::event(&event);
        self.inner.event_handled()?;
        return Ok(view);
    }
}

// === Pending on-chain payments

fn now_secs() -> u64 {
    return SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
}

fn pending_view(tx: &PendingTx) -> PaymentView {
    return PaymentView {
        id: format!("pending-{}", tx.txid),
        kind: PaymentKindView::Onchain,
        direction: if tx.inbound {
            Direction::Inbound
        } else {
            Direction::Outbound
        },
        status: PaymentState::Pending,
        amount_msat: Some(tx.amount_sat * 1000),
        fee_paid_msat: tx.fee_sat.map(|fee| fee * 1000),
        payment_hash: None,
        preimage: None,
        txid: Some(tx.txid.clone()),
        updated_at: tx.seen_at,
        first_seen_at: tx.seen_at,
    };
}

// === Peer connection

/// A channel can only be opened with a peer the node is connected to right now. ldk-node would
/// otherwise dial the peer itself, which hides that the connection was missing or has dropped.
fn check_peer_connected(connected: bool) -> Result<(), NodeError> {
    if !connected {
        return Err(NodeError::InvalidInput(
            "not connected to that peer: connect to it on the Peers card first, then open the channel"
                .to_string(),
        ));
    }
    return Ok(());
}

// === Channel size

/// Refuses a channel the node would fail to open, with a message that says why.
fn check_channel_size(amount_sat: u64, push_msat: Option<u64>) -> Result<(), NodeError> {
    if amount_sat < MIN_CHANNEL_SAT {
        return Err(NodeError::InvalidInput(format!(
            "channel capacity must be at least {MIN_CHANNEL_SAT} sat: each side keeps a 1,000 sat reserve and opening costs about 1,160 sat, so a smaller channel is closed by the peer"
        )));
    }
    let push_sat = push_msat.unwrap_or(0) / 1000;
    if push_sat >= amount_sat {
        return Err(NodeError::InvalidInput(
            "the push amount must be smaller than the channel capacity".to_string(),
        ));
    }
    if amount_sat - push_sat < MIN_OUR_SIDE_SAT {
        return Err(NodeError::InvalidInput(format!(
            "leave at least {MIN_OUR_SIDE_SAT} sat on your side after the push to cover the opening fee"
        )));
    }
    return Ok(());
}

// === Parsing

fn parse_pubkey(value: &str) -> Result<PublicKey, NodeError> {
    return PublicKey::from_str(value.trim())
        .map_err(|_| NodeError::InvalidInput(format!("bad node id {value:?}")));
}

fn parse_peer(node_id: &str, address: &str) -> Result<(PublicKey, SocketAddress), NodeError> {
    let address = SocketAddress::from_str(address.trim())
        .map_err(|_| NodeError::InvalidInput(format!("bad peer address {address:?}")))?;
    return Ok((parse_pubkey(node_id)?, address));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(result: Result<(), NodeError>) -> String {
        return result.unwrap_err().to_string();
    }

    #[test]
    fn a_channel_needs_a_connected_peer() {
        assert!(message(check_peer_connected(false)).contains("not connected to that peer"));
        assert!(check_peer_connected(true).is_ok());
    }

    #[test]
    fn a_channel_below_the_minimum_is_refused() {
        // 500, 1,000 and 2,000 sat were all closed by the peer: the reserve leaves nothing.
        for too_small in [500, 1_000, 2_000, 2_299] {
            assert!(message(check_channel_size(too_small, None)).contains("at least 2300"));
        }
        assert!(check_channel_size(2_300, None).is_ok());
        assert!(check_channel_size(5_000, None).is_ok());
    }

    #[test]
    fn the_push_must_leave_enough_on_our_side() {
        assert!(message(check_channel_size(5_000, Some(2_800_000))).contains("leave at least 2300"));
        assert!(check_channel_size(5_000, Some(2_700_000)).is_ok());
        assert!(message(check_channel_size(5_000, Some(5_000_000))).contains("smaller than"));
    }
}
