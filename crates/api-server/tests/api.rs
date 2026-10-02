use std::sync::Arc;

use api_server::{AppState, router};
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use node_core::{LightningNode, Network, NodeConfig};
use serde_json::Value;
use tokio::sync::broadcast;
use tower::ServiceExt;

const TOKEN: &str = "test-token-0123456789";
const COFFEE: &str = "lnbc2500u1pvjluezsp5zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zygspp5qqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqypqdq5xysxxatsyp3k7enxv4jsxqzpu9qrsgquk0rl77nj30yxdy8j9vdx85fkpmdla2087ne0xh8nhedh8w27kyke0lp53ut353s06fv3qfegext0eh0ymjpf39tuven09sam30g4vgpfna3rh";

// Building a node does not contact bitcoind; only start does, so these tests need no chain. ldk-node
// blocks inside build and drop, hence the multi-thread runtime on every test.
fn app(name: &str) -> Router {
    return app_with_dir(name).0;
}

fn app_with_dir(name: &str) -> (Router, std::path::PathBuf) {
    let data_dir =
        std::env::temp_dir().join(format!("api-server-test-{name}-{}", std::process::id()));
    let node = LightningNode::build(&NodeConfig {
        network: Network::Regtest,
        data_dir: data_dir.clone(),
        listen_address: "127.0.0.1:0".to_string(),
        alias: "test".to_string(),
        rpc_host: "127.0.0.1".to_string(),
        rpc_port: 18443,
        rpc_user: "user".to_string(),
        rpc_password: "pass".to_string(),
    })
    .expect("node builds");
    let (events, _) = broadcast::channel(8);
    let app = router(AppState {
        node,
        token: Arc::from(TOKEN),
        max_pay_msat: 1_000_000_000,
        events,
    });
    return (app, data_dir);
}

async fn call(app: Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    return (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    );
}

fn get(uri: &str, token: Option<&str>) -> Request<Body> {
    let mut builder = Request::get(uri);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    return builder.body(Body::empty()).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn health_needs_no_token() {
    let (status, body) = call(app("health"), get("/health", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn protected_routes_reject_missing_and_wrong_tokens() {
    for token in [None, Some("wrong-token-0123456789")] {
        let (status, body) = call(app("auth"), get("/node/status", token)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["code"], "unauthorized");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn status_reports_a_node_that_is_not_running() {
    let (status, body) = call(app("status"), get("/node/status", Some(TOKEN))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["is_running"], false);
    assert_eq!(body["network"], "regtest");
    assert_eq!(body["is_synced"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn balance_amounts_are_strings() {
    let (status, body) = call(app("balance"), get("/wallet/balance", Some(TOKEN))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["onchain_total_sat"], "0");
}

fn decode_request(invoice: &str, token: Option<&str>) -> Request<Body> {
    let body = serde_json::json!({
        "invoice": invoice,
        "context": { "now_unix": 1496314658u64 },
    });
    let mut builder = Request::post("/decode").header("content-type", "application/json");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    return builder.body(Body::from(body.to_string())).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn decode_returns_the_decoded_invoice() {
    let (status, body) = call(app("decode"), decode_request(COFFEE, Some(TOKEN))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["invoice"]["amount_msat"], "250000000");
    assert_eq!(body["report"]["verdict"], "payable");
}

#[tokio::test(flavor = "multi_thread")]
async fn decode_error_uses_the_error_envelope() {
    let (status, body) = call(app("decode-err"), decode_request("lnbc1nope", Some(TOKEN))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "decode_failed");
    assert!(body["error"]["message"].is_string());
}

#[tokio::test(flavor = "multi_thread")]
async fn decode_requires_the_token() {
    let (status, _) = call(app("decode-auth"), decode_request(COFFEE, None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

fn post(uri: &str, body: &str) -> Request<Body> {
    return Request::post(uri)
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::from(body.to_string()))
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn malformed_json_uses_the_error_envelope() {
    let (status, body) = call(app("bad-json"), post("/peers", "{not json")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_input");
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_node_id_is_rejected_before_touching_the_node() {
    let request = post("/peers", r#"{"node_id":"nope","address":"127.0.0.1:9735"}"#);
    let (status, body) = call(app("bad-peer"), request).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_input");
}

#[tokio::test(flavor = "multi_thread")]
async fn disconnect_rejects_a_bad_node_id() {
    let (status, body) = call(
        app("bad-disconnect"),
        post("/peers/disconnect", r#"{"node_id":"nope"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid_input");
}

#[tokio::test(flavor = "multi_thread")]
async fn amounts_must_be_strings_of_digits() {
    let request = post(
        "/channels",
        r#"{"node_id":"02eec7245d6b7d2ccb30380bfbe2a3648cd7a942653f5aa340edcea1f283686619","address":"127.0.0.1:9735","amount_sat":"lots"}"#,
    );
    let (status, body) = call(app("bad-amount"), request).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("whole number")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_lists_before_any_activity() {
    let app = app("lists");
    for uri in ["/peers", "/channels", "/payments"] {
        let (status, body) = call(app.clone(), get(uri, Some(TOKEN))).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert_eq!(body, serde_json::json!([]), "{uri}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn mainnet_invoice_is_refused_on_a_regtest_node() {
    let body = serde_json::json!({ "invoice": COFFEE }).to_string();
    let (status, body) = call(app("pay-network"), post("/payments", &body)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"]["code"], "payment_refused");
    assert_eq!(body["error"]["details"]["verdict"], "not_payable");
}

#[tokio::test(flavor = "multi_thread")]
async fn events_need_the_token() {
    let (status, _) = call(app("events"), get("/events", None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

// Plan, section 8: the seed is never returned by any endpoint.
#[tokio::test(flavor = "multi_thread")]
async fn no_endpoint_returns_the_seed() {
    let (app, data_dir) = app_with_dir("seed");
    let seed = std::fs::read(data_dir.join("keys_seed")).expect("ldk-node writes keys_seed");
    let seed_hex: String = seed.iter().map(|b| format!("{b:02x}")).collect();

    let requests = [
        get("/health", None),
        get("/node/status", Some(TOKEN)),
        get("/wallet/balance", Some(TOKEN)),
        get("/peers", Some(TOKEN)),
        get("/channels", Some(TOKEN)),
        get("/payments", Some(TOKEN)),
        post("/wallet/address", ""),
        post("/invoices", r#"{"description":"x"}"#),
    ];
    for request in requests {
        let uri = request.uri().to_string();
        let response = app.clone().oneshot(request).await.unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&bytes).to_lowercase();
        assert!(!text.contains(&seed_hex), "{uri} leaked the seed");
        assert!(
            !bytes.windows(seed.len()).any(|w| w == seed.as_slice()),
            "{uri} leaked the seed"
        );
    }
}
