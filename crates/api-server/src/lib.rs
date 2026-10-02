//! Localhost REST API in front of the node and the invoice decoder.
//!
//! The browser never calls this directly: the Next.js server holds the bearer token and proxies.

mod amount;
pub mod auth;
pub mod config;
pub mod error;
pub mod firewall;
pub mod routes;

use std::sync::Arc;

use axum::Router;
use axum::middleware;
use axum::routing::{get, post};
use node_core::{LightningNode, NodeEvent};
use tokio::sync::broadcast;

#[derive(Clone)]
pub struct AppState {
    pub node: LightningNode,
    pub token: Arc<str>,
    pub max_pay_msat: u64,
    /// Fed by the single task that drains ldk-node's event queue; each SSE client subscribes.
    pub events: broadcast::Sender<NodeEvent>,
}

pub fn router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/node/status", get(routes::node_status))
        .route("/node/sync", post(routes::sync))
        .route("/wallet/address", post(routes::new_address))
        .route("/wallet/balance", get(routes::balance))
        .route("/wallet/send", post(routes::send_onchain))
        .route("/peers", get(routes::list_peers).post(routes::connect_peer))
        .route("/peers/disconnect", post(routes::disconnect_peer))
        .route(
            "/channels",
            get(routes::list_channels).post(routes::open_channel),
        )
        .route("/channels/close", post(routes::close_channel))
        .route("/invoices", post(routes::create_invoice))
        .route("/payments", get(routes::list_payments).post(routes::pay))
        .route("/events", get(routes::events))
        .route("/decode", post(routes::decode))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_token,
        ));

    return Router::new()
        .route("/health", get(routes::health))
        .merge(protected)
        .with_state(state);
}

/// Drains ldk-node's event queue into the broadcast channel until the node goes away.
pub async fn pump_events(node: LightningNode, events: broadcast::Sender<NodeEvent>) {
    loop {
        match node.next_event().await {
            // No subscribers is normal: nobody has the page open.
            Ok(event) => {
                let _ = events.send(event);
            }
            Err(err) => eprintln!("could not mark event handled: {err}"),
        }
    }
}
