use std::io::Write;
use std::process::{Command, Output, Stdio};

const COFFEE: &str = "lnbc2500u1pvjluezsp5zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zyg3zygspp5qqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqqqsyqcyq5rqwzqfqypqdq5xysxxatsyp3k7enxv4jsxqzpu9qrsgquk0rl77nj30yxdy8j9vdx85fkpmdla2087ne0xh8nhedh8w27kyke0lp53ut353s06fv3qfegext0eh0ymjpf39tuven09sam30g4vgpfna3rh";
const SPEC_NOW: &str = "1496314658";

fn run(args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_invoice-cli"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("binary starts");
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    return child.wait_with_output().expect("binary finishes");
}

fn stdout(output: &Output) -> String {
    return String::from_utf8_lossy(&output.stdout).into_owned();
}

#[test]
fn table_output_for_a_payable_invoice() {
    let out = run(&["decode", COFFEE, "--now", SPEC_NOW], None);
    assert_eq!(out.status.code(), Some(0));
    let text = stdout(&out);
    assert!(text.contains("Verdict: PAYABLE"), "{text}");
    assert!(
        text.contains("250000000 msat = 250000 sat = 0.0025 BTC"),
        "{text}"
    );
    assert!(text.contains("\"1 cup coffee\""), "{text}");
    assert!(text.contains("recovered from the signature"), "{text}");
    assert!(text.contains("[PASS] RequiredFields"), "{text}");
}

#[test]
fn json_output() {
    let out = run(&["decode", COFFEE, "--now", SPEC_NOW, "--json"], None);
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert_eq!(json["invoice"]["amount_msat"], "250000000");
    assert_eq!(json["report"]["verdict"], "payable");
}

#[test]
fn stdin_input_and_expiry_exit_code() {
    let out = run(&["decode"], Some(&format!("lightning:{COFFEE}\n")));
    assert_eq!(
        out.status.code(),
        Some(1),
        "long expired against the real clock"
    );
    assert!(stdout(&out).contains("[FAIL] Expiry"));
}

#[test]
fn policy_flags() {
    let out = run(
        &[
            "decode",
            COFFEE,
            "--now",
            SPEC_NOW,
            "--network",
            "regtest",
            "--max-msat",
            "1000",
        ],
        None,
    );
    assert_eq!(out.status.code(), Some(1));
    let text = stdout(&out);
    assert!(text.contains("[FAIL] Network"), "{text}");
    assert!(text.contains("[FAIL] Amount"), "{text}");
}

#[test]
fn unreadable_input_points_at_the_bad_character() {
    let bad = COFFEE.replacen("zyg3", "zyb3", 1);
    let out = run(&["decode", &bad], None);
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("invalid character 'b'"), "{err}");
    assert!(err.contains('^'), "{err}");

    let json = run(&["decode", &bad, "--json"], None);
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(value["error"]["code"], "invalid_char");
}
