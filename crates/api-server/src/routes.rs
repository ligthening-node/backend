use std::convert::Infallible;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::Stream;
use futures_util::stream;
use invoice_core::{DecodeContext, Decoded};
use node_core::{
    Balances, ChannelView, CreatedInvoice, LightningNode, NodeStatus, PaymentView, PeerView,
    SentPayment,
};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;

use crate::AppState;
use crate::amount;
use crate::error::ApiError;
use crate::firewall::{self, PayPolicy};

type Body<T> = Result<Json<T>, JsonRejection>;

const DEFAULT_INVOICE_EXPIRY_SECS: u32 = 3600;

// === Request and response shapes

#[derive(Serialize)]
pub struct Health {
    pub ok: bool,
}

#[derive(Serialize)]
pub struct AddressResponse {
    pub address: String,
}

#[derive(Serialize)]
pub struct TxidResponse {
    pub txid: String,
}

#[derive(Serialize)]
pub struct Done {
    pub ok: bool,
}

#[derive(Serialize)]
pub struct OpenedChannel {
    pub user_channel_id: String,
}

#[derive(Deserialize)]
pub struct DecodeRequest {
    pub invoice: String,
    pub context: DecodeContext,
}

#[derive(Deserialize)]
pub struct SendOnchainRequest {
    pub address: String,
    #[serde(deserialize_with = "amount::required")]
    pub amount_sat: u64,
}

#[derive(Deserialize)]
pub struct DisconnectRequest {
    pub node_id: String,
}

#[derive(Deserialize)]
pub struct ConnectRequest {
    pub node_id: String,
    pub address: String,
}

#[derive(Deserialize)]
pub struct OpenChannelRequest {
    pub node_id: String,
    pub address: String,
    #[serde(deserialize_with = "amount::required")]
    pub amount_sat: u64,
    #[serde(default, deserialize_with = "amount::optional")]
    pub push_msat: Option<u64>,
}

#[derive(Deserialize)]
pub struct CloseChannelRequest {
    pub user_channel_id: String,
    pub counterparty_node_id: String,
    #[serde(default)]
    pub force: bool,
}

#[derive(Deserialize)]
pub struct InvoiceRequest {
    #[serde(default, deserialize_with = "amount::optional")]
    pub amount_msat: Option<u64>,
    #[serde(default)]
    pub description: String,
    pub expiry_secs: Option<u32>,
}

#[derive(Deserialize)]
pub struct PayRequest {
    pub invoice: String,
    /// Only for any-amount invoices.
    #[serde(default, deserialize_with = "amount::optional")]
    pub amount_msat: Option<u64>,
    pub expected_payee: Option<String>,
}

// === Node and wallet

pub async fn health() -> Json<Health> {
    return Json(Health { ok: true });
}

pub async fn node_status(State(state): State<AppState>) -> Json<NodeStatus> {
    return Json(state.node.status());
}

pub async fn sync(State(state): State<AppState>) -> Result<Json<Done>, ApiError> {
    blocking(&state.node, |node| node.sync()).await?;
    return Ok(Json(Done { ok: true }));
}

pub async fn new_address(State(state): State<AppState>) -> Result<Json<AddressResponse>, ApiError> {
    let address = state.node.new_address()?;
    return Ok(Json(AddressResponse { address }));
}

pub async fn balance(State(state): State<AppState>) -> Json<Balances> {
    return Json(state.node.balances());
}

pub async fn send_onchain(
    State(state): State<AppState>,
    body: Body<SendOnchainRequest>,
) -> Result<Json<TxidResponse>, ApiError> {
    let Json(req) = body?;
    let txid = blocking(&state.node, move |node| {
        node.send_onchain(&req.address, req.amount_sat)
    })
    .await?;
    return Ok(Json(TxidResponse { txid }));
}

// === Peers and channels

pub async fn list_peers(State(state): State<AppState>) -> Json<Vec<PeerView>> {
    return Json(state.node.list_peers());
}

pub async fn connect_peer(
    State(state): State<AppState>,
    body: Body<ConnectRequest>,
) -> Result<Json<Done>, ApiError> {
    let Json(req) = body?;
    blocking(&state.node, move |node| {
        node.connect(&req.node_id, &req.address)
    })
    .await?;
    return Ok(Json(Done { ok: true }));
}

pub async fn disconnect_peer(
    State(state): State<AppState>,
    body: Body<DisconnectRequest>,
) -> Result<Json<Done>, ApiError> {
    let Json(req) = body?;
    blocking(&state.node, move |node| node.disconnect(&req.node_id)).await?;
    return Ok(Json(Done { ok: true }));
}

pub async fn list_channels(State(state): State<AppState>) -> Json<Vec<ChannelView>> {
    return Json(state.node.list_channels());
}

pub async fn open_channel(
    State(state): State<AppState>,
    body: Body<OpenChannelRequest>,
) -> Result<Json<OpenedChannel>, ApiError> {
    let Json(req) = body?;
    let user_channel_id = blocking(&state.node, move |node| {
        node.open_channel(&req.node_id, &req.address, req.amount_sat, req.push_msat)
    })
    .await?;
    return Ok(Json(OpenedChannel { user_channel_id }));
}

pub async fn close_channel(
    State(state): State<AppState>,
    body: Body<CloseChannelRequest>,
) -> Result<Json<Done>, ApiError> {
    let Json(req) = body?;
    blocking(&state.node, move |node| {
        node.close_channel(&req.user_channel_id, &req.counterparty_node_id, req.force)
    })
    .await?;
    return Ok(Json(Done { ok: true }));
}

// === Invoices and payments

pub async fn create_invoice(
    State(state): State<AppState>,
    body: Body<InvoiceRequest>,
) -> Result<Json<CreatedInvoice>, ApiError> {
    let Json(req) = body?;
    let expiry = req.expiry_secs.unwrap_or(DEFAULT_INVOICE_EXPIRY_SECS);
    let invoice = state
        .node
        .create_invoice(req.amount_msat, &req.description, expiry)?;
    return Ok(Json(invoice));
}

pub async fn pay(
    State(state): State<AppState>,
    body: Body<PayRequest>,
) -> Result<Json<SentPayment>, ApiError> {
    let Json(req) = body?;
    let policy = PayPolicy {
        network: state.node.network(),
        max_amount_msat: state.max_pay_msat,
    };
    firewall::check(
        &req.invoice,
        req.amount_msat,
        req.expected_payee.as_deref(),
        &policy,
        now_unix(),
    )?;
    let sent = blocking(&state.node, move |node| {
        node.pay_invoice(&req.invoice, req.amount_msat)
    })
    .await?;
    return Ok(Json(sent));
}

pub async fn list_payments(State(state): State<AppState>) -> Json<Vec<PaymentView>> {
    return Json(state.node.list_payments());
}

pub async fn events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let receiver = state.events.subscribe();
    let stream = stream::unfold(receiver, |mut receiver| async move {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    let data = serde_json::to_string(&event).expect("events serialize");
                    return Some((Ok(Event::default().data(data)), receiver));
                }
                // A slow client missed some events; keep streaming the newer ones.
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return None,
            }
        }
    });
    return Sse::new(stream).keep_alive(KeepAlive::default());
}

pub async fn decode(body: Body<DecodeRequest>) -> Result<Json<Decoded>, ApiError> {
    let Json(request) = body?;
    return invoice_core::decode(&request.invoice, &request.context)
        .map(Json)
        .map_err(|err| ApiError::decode_failed(err.to_string()));
}

// === Helpers

/// Runs a node call that may block for seconds (network or chain round trips) off the async
/// worker threads.
async fn blocking<T, F>(node: &LightningNode, f: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce(&LightningNode) -> Result<T, node_core::NodeError> + Send + 'static,
{
    let node = node.clone();
    return tokio::task::spawn_blocking(move || f(&node))
        .await
        .map_err(|err| ApiError::internal(err.to_string()))?
        .map_err(ApiError::from);
}

fn now_unix() -> u64 {
    return SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
}
