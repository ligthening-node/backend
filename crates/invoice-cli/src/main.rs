mod render;

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use bitcoin::secp256k1::PublicKey;
use clap::{Parser, Subcommand, ValueEnum};
use invoice_core::{DecodeContext, Network, Verdict, decode};

/// Exit codes: 0 payable, 1 not payable (expired, wrong network, policy), 2 invalid or unreadable.
#[derive(Parser)]
#[command(
    name = "invoice-cli",
    version,
    about = "Decode and validate BOLT11 Lightning invoices"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Decode an invoice given as an argument, with --file, or on stdin.
    Decode(DecodeArgs),
}

#[derive(clap::Args)]
struct DecodeArgs {
    /// The invoice. `lightning:` prefixes and upper case are accepted.
    invoice: Option<String>,

    /// Read the invoice from a file instead.
    #[arg(long, conflicts_with = "invoice")]
    file: Option<PathBuf>,

    /// Print JSON instead of a table.
    #[arg(long)]
    json: bool,

    /// Fail the network check unless the invoice is for this network.
    #[arg(long, value_enum)]
    network: Option<NetworkArg>,

    /// Fail unless the invoice is signed by this node public key (hex).
    #[arg(long, value_name = "PUBKEY")]
    expect_payee: Option<PublicKey>,

    /// Check this text against the invoice's description hash.
    #[arg(long, value_name = "TEXT")]
    description: Option<String>,

    /// Fail the amount check above this many millisatoshis.
    #[arg(long, value_name = "MSAT")]
    max_msat: Option<u64>,

    /// Evaluate expiry at this Unix time instead of now.
    #[arg(long, value_name = "UNIX_SECS")]
    now: Option<u64>,
}

#[derive(Clone, Copy, ValueEnum)]
enum NetworkArg {
    Bitcoin,
    Testnet,
    Signet,
    Regtest,
}

impl From<NetworkArg> for Network {
    fn from(arg: NetworkArg) -> Self {
        return match arg {
            NetworkArg::Bitcoin => Network::Bitcoin,
            NetworkArg::Testnet => Network::Testnet,
            NetworkArg::Signet => Network::Signet,
            NetworkArg::Regtest => Network::Regtest,
        };
    }
}

fn main() -> ExitCode {
    let Command::Decode(args) = Cli::parse().command;

    let input = match read_input(&args) {
        Ok(input) => input,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(2);
        }
    };

    let ctx = DecodeContext {
        now_unix: args.now.unwrap_or_else(unix_now),
        expected_network: args.network.map(Network::from),
        expected_payee: args.expect_payee,
        description_preimage: args.description.clone(),
        max_amount_msat: args.max_msat,
    };

    let decoded = match decode(&input, &ctx) {
        Ok(decoded) => decoded,
        Err(err) => {
            if args.json {
                println!(
                    "{}",
                    serde_json::json!({ "error": err, "message": err.to_string() })
                );
            } else {
                eprintln!("{}", render::error(&input, &err));
            }
            return ExitCode::from(2);
        }
    };

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&decoded).expect("output types serialize")
        );
    } else {
        print!("{}", render::decoded(&decoded, ctx.now_unix));
    }

    return match decoded.report.verdict {
        Verdict::Payable => ExitCode::SUCCESS,
        Verdict::NotPayable => ExitCode::from(1),
        Verdict::Invalid => ExitCode::from(2),
    };
}

fn read_input(args: &DecodeArgs) -> Result<String, String> {
    if let Some(invoice) = &args.invoice {
        return Ok(invoice.clone());
    }
    if let Some(path) = &args.file {
        return std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()));
    }
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| format!("stdin: {e}"))?;
    if buf.trim().is_empty() {
        return Err("no invoice given (pass it as an argument, with --file, or on stdin)".into());
    }
    return Ok(buf);
}

fn unix_now() -> u64 {
    return SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
}
