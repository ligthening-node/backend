//! Full regtest flow with two nodes: fund, open, pay, receive, close.
//!
//! Needs a regtest bitcoind; ignored by default:
//!
//!   LN_API_TOKEN=unused docker compose up -d --wait bitcoind
//!   cargo test -p node-core --test regtest -- --ignored --nocapture

use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use invoice_core::{CheckId, DecodeContext, Status, Verdict};
use node_core::{
    Direction, LightningNode, Network, NodeConfig, NodeEvent, PaymentKindView, PaymentState,
};
use serde_json::{Value, json};

// === bitcoind

const RPC_URL: &str = "http://127.0.0.1:18443";
const RPC_USER: &str = "lightning";
const RPC_PASSWORD: &str = "lightning";
const WALLET: &str = "itest";

fn rpc(path: &str, method: &str, params: Value) -> Result<Value, String> {
    let body = json!({ "jsonrpc": "1.0", "id": "itest", "method": method, "params": params });
    let response: Value = reqwest::blocking::Client::new()
        .post(format!("{RPC_URL}{path}"))
        .basic_auth(RPC_USER, Some(RPC_PASSWORD))
        .json(&body)
        .send()
        .map_err(|e| format!("bitcoind unreachable: {e}"))?
        .json()
        .map_err(|e| e.to_string())?;
    if !response["error"].is_null() {
        return Err(response["error"].to_string());
    }
    return Ok(response["result"].clone());
}

fn wallet(method: &str, params: Value) -> Value {
    return rpc(&format!("/wallet/{WALLET}"), method, params).expect(method);
}

fn mine(blocks: u32) {
    let address = wallet("getnewaddress", json!([]));
    wallet("generatetoaddress", json!([blocks, address]));
}

fn fund(address: &str, btc: f64) {
    wallet("sendtoaddress", json!([address, btc]));
}

fn setup_wallet() {
    // Either call fails harmlessly when the wallet already exists or is already loaded.
    let _ = rpc("", "createwallet", json!([WALLET]));
    let _ = rpc("", "loadwallet", json!([WALLET]));
    let balance = wallet("getbalance", json!([])).as_f64().unwrap_or(0.0);
    if balance < 1.0 {
        mine(101);
    }
}

// === Nodes

fn free_port() -> u16 {
    return TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
}

struct TestNode {
    node: LightningNode,
    address: String,
    _dir: PathBuf,
}

fn start_node(name: &str) -> TestNode {
    let dir = std::env::temp_dir().join(format!(
        "node-core-regtest-{name}-{}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    let address = format!("127.0.0.1:{}", free_port());
    let node = LightningNode::build(&NodeConfig {
        network: Network::Regtest,
        data_dir: dir.clone(),
        listen_address: address.clone(),
        alias: name.to_string(),
        rpc_host: "127.0.0.1".to_string(),
        rpc_port: 18443,
        rpc_user: RPC_USER.to_string(),
        rpc_password: RPC_PASSWORD.to_string(),
    })
    .expect("node builds");
    node.start().expect("node starts");
    return TestNode {
        node,
        address,
        _dir: dir,
    };
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn sync(nodes: &[&TestNode]) {
    for n in nodes {
        n.node.sync().expect("sync");
    }
}

fn now() -> u64 {
    return std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
}

// === The flow

#[test]
#[ignore = "needs a regtest bitcoind on 127.0.0.1:18443 (docker compose up -d bitcoind)"]
fn fund_open_pay_receive_close() {
    setup_wallet();
    let alice = start_node("alice");
    let bob = start_node("bob");
    let bob_id = bob.node.status().node_id;

    // Fund: bob needs a little on-chain money too, as the anchor reserve for accepting a channel.
    fund(&alice.node.new_address().unwrap(), 0.02);
    fund(&bob.node.new_address().unwrap(), 0.001);
    mine(6);
    sync(&[&alice, &bob]);
    wait_for("alice's funds", || {
        alice.node.balances().onchain_spendable_sat >= 1_900_000
    });

    // Open
    alice.node.connect(&bob_id, &bob.address).expect("connect");
    assert!(
        alice
            .node
            .list_peers()
            .iter()
            .any(|p| p.node_id == bob_id && p.is_connected)
    );
    let user_channel_id = alice
        .node
        .open_channel(&bob_id, &bob.address, 1_000_000, None)
        .expect("open channel");
    wait_for("the funding transaction", || {
        alice
            .node
            .list_channels()
            .iter()
            .any(|c| c.funding_txo.is_some())
    });
    mine(6);
    wait_for("a usable channel", || {
        sync(&[&alice, &bob]);
        let usable = |n: &TestNode| n.node.list_channels().iter().any(|c| c.is_usable);
        let liquid = alice
            .node
            .list_channels()
            .iter()
            .any(|c| c.max_send_msat >= 50_000_000);
        usable(&alice) && usable(&bob) && liquid
    });
    let channel = alice.node.list_channels().remove(0);
    assert_eq!(channel.capacity_sat, 1_000_000);
    assert!(channel.short_channel_id.unwrap().contains('x'));

    // Pay a fixed-amount invoice, checked by our decoder first like the API does.
    let created = bob
        .node
        .create_invoice(Some(50_000_000), "itest coffee", 600)
        .expect("invoice");
    eprintln!("fixed-amount invoice: {}", created.invoice);
    let ctx = DecodeContext {
        now_unix: now(),
        expected_network: Some(invoice_core::Network::Regtest),
        expected_payee: Some(bob_id.parse().unwrap()),
        description_preimage: None,
        max_amount_msat: Some(100_000_000),
    };
    let decoded = invoice_core::decode(&created.invoice, &ctx).expect("our decoder reads it");
    assert_eq!(decoded.report.verdict, Verdict::Payable);
    assert_eq!(decoded.invoice.amount_msat, Some(50_000_000));
    assert!(
        !decoded.invoice.route_hints.is_empty(),
        "private channel adds a route hint"
    );

    alice.node.pay_invoice(&created.invoice, None).expect("pay");
    wait_for("the fixed payment", || {
        alice.node.list_payments().iter().any(|p| {
            p.kind == PaymentKindView::Bolt11
                && p.direction == Direction::Outbound
                && p.status == PaymentState::Succeeded
                && p.payment_hash.as_deref() == Some(created.payment_hash.as_str())
        })
    });

    // Pay an any-amount, short-expiry invoice with an amount of our choosing.
    let tip = bob.node.create_invoice(None, "tip", 60).expect("invoice");
    eprintln!("any-amount invoice: {}", tip.invoice);
    let decoded = invoice_core::decode(&tip.invoice, &ctx).expect("our decoder reads it");
    assert_eq!(decoded.invoice.amount_msat, None);
    assert_eq!(decoded.invoice.expiry_secs, 60);
    let amount = decoded
        .report
        .checks
        .iter()
        .find(|c| c.id == CheckId::Amount)
        .unwrap();
    assert_eq!(amount.status, Status::Warn);
    assert!(
        alice.node.pay_invoice(&tip.invoice, None).is_err(),
        "needs an amount"
    );
    alice
        .node
        .pay_invoice(&tip.invoice, Some(1_000_000))
        .expect("pay tip");

    // Receive: bob sees both payments as events and in his history.
    let runtime = ldk_node::tokio::runtime::Runtime::new().unwrap();
    let mut received = 0;
    while received < 2 {
        let event = runtime
            .block_on(async {
                ldk_node::tokio::time::timeout(Duration::from_secs(60), bob.node.next_event()).await
            })
            .expect("an event within a minute")
            .expect("event handled");
        if let NodeEvent::PaymentReceived { .. } = event {
            received += 1;
        }
    }
    let inbound = bob
        .node
        .list_payments()
        .into_iter()
        .filter(|p| {
            p.kind == PaymentKindView::Bolt11
                && p.direction == Direction::Inbound
                && p.status == PaymentState::Succeeded
        })
        .count();
    assert_eq!(inbound, 2);
    assert!(bob.node.balances().lightning_sat >= 51_000);

    // Close cooperatively.
    alice
        .node
        .close_channel(&user_channel_id, &bob_id, false)
        .expect("close");
    wait_for("the channel to close", || {
        mine(1);
        sync(&[&alice, &bob]);
        alice.node.list_channels().is_empty() && bob.node.list_channels().is_empty()
    });

    alice.node.stop().unwrap();
    bob.node.stop().unwrap();
}
