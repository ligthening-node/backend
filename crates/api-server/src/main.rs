use std::sync::Arc;
use std::time::Duration;

use api_server::config::ServerConfig;
use api_server::{AppState, pump_events, router};
use node_core::{LightningNode, NodeEvent};
use tokio::sync::broadcast;

const EVENT_BUFFER: usize = 64;
/// How often new payments get their first-seen time recorded.
const FIRST_SEEN_EVERY: Duration = Duration::from_secs(2);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = ServerConfig::from_env()?;
    let node = LightningNode::build(&config.node)?;

    // ldk-node owns a runtime of its own, so start and stop run off the async threads.
    let starting = node.clone();
    tokio::task::spawn_blocking(move || starting.start()).await??;

    let (events, _) = broadcast::channel(EVENT_BUFFER);
    let pump = tokio::spawn(pump_events(node.clone(), events.clone()));

    // Records when each payment first appeared, even with no browser open, and tells browsers
    // when the payment list changed.
    let recorder = node.clone();
    let changes = events.clone();
    let first_seen = tokio::spawn(async move {
        let mut tick = tokio::time::interval(FIRST_SEEN_EVERY);
        loop {
            tick.tick().await;
            let node = recorder.clone();
            let changed = tokio::task::spawn_blocking(move || {
                node.refresh_pending();
                node.record_new_payments();
                node.payments_changed()
            })
            .await;
            if matches!(changed, Ok(true)) {
                let _ = changes.send(NodeEvent::PaymentsChanged);
            }
        }
    });

    let state = AppState {
        node: node.clone(),
        token: Arc::from(config.token.as_str()),
        max_pay_msat: config.max_pay_msat,
        events,
    };
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    println!("api-server listening on http://{}", config.bind);
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    pump.abort();
    first_seen.abort();
    tokio::task::spawn_blocking(move || node.stop()).await??;
    return Ok(());
}

// Docker stops containers with SIGTERM, a terminal with Ctrl+C.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
