//! V2G-B: tier Merkle artifact generator CLI.
//!
//! Reads a deterministic JSON input file describing trader signals
//! and writes a fully-assembled tier Merkle artifact (snapshot rows,
//! Merkle root, per-row proofs, canonical fee schedule, env metadata)
//! to disk.
//!
//! Usage:
//!
//! ```bash
//! cargo run --bin generate_tier_artifact -- \
//!     --input fixtures/tier_snapshot/base_sepolia_v2g_b_smoke.json \
//!     --output artifacts/tier_merkle/base_sepolia_v2g_b.json
//! ```
//!
//! The CLI is intentionally hermetic: it never reads the live chain,
//! never accesses Postgres, and never signs anything. The output is
//! pure-function of the input file plus the system clock.

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use deopt_v2_backend::fees::tier_artifact::{generate_tier_artifact, ArtifactConfig};
use deopt_v2_backend::fees::tier_snapshot::{SnapshotConfig, TraderInputs};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Cli {
    input: PathBuf,
    output: PathBuf,
}

/// Input file shape. Stable JSON schema:
///
/// ```json
/// {
///   "chain_id": 84532,
///   "fees_manager_v2": "0x00dA0B98…",
///   "valid_from": 1700000000,
///   "valid_until": 1700604800,
///   "accounts": [
///     {
///       "label": "v2f_lm_taker",
///       "trader": "0x8b94a83d…",
///       "option_volume_28d_1e8": "1250000000000000",
///       "perp_volume_28d_1e8":   "1250000000000000",
///       "volume_share_ppm": 0,
///       "staked_deopt_1e8": "0"
///     }
///   ]
/// }
/// ```
#[derive(Debug, Deserialize)]
struct InputFile {
    chain_id: u64,
    fees_manager_v2: String,
    valid_from: u64,
    valid_until: u64,
    accounts: Vec<AccountInput>,
}

#[derive(Debug, Deserialize)]
struct AccountInput {
    #[serde(default)]
    #[allow(dead_code)]
    label: Option<String>,
    trader: String,
    option_volume_28d_1e8: String,
    perp_volume_28d_1e8: String,
    volume_share_ppm: u32,
    staked_deopt_1e8: String,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("generate_tier_artifact: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let cli = parse_args()?;
    let raw = fs::read_to_string(&cli.input)
        .map_err(|error| format!("read {}: {error}", cli.input.display()))?;
    let input: InputFile =
        serde_json::from_str(&raw).map_err(|error| format!("parse input JSON: {error}"))?;
    if input.valid_until <= input.valid_from {
        return Err(format!(
            "valid_until ({}) must be strictly greater than valid_from ({})",
            input.valid_until, input.valid_from
        ));
    }
    let fees_manager = parse_address(&input.fees_manager_v2)?;
    let snapshot_config = SnapshotConfig {
        valid_from: input.valid_from,
        valid_until: input.valid_until,
    };
    let mut trader_inputs: Vec<TraderInputs> = Vec::with_capacity(input.accounts.len());
    for account in &input.accounts {
        trader_inputs.push(TraderInputs {
            account: parse_address(&account.trader)?,
            option_volume_28d_1e8: parse_u128(&account.option_volume_28d_1e8)?,
            perp_volume_28d_1e8: parse_u128(&account.perp_volume_28d_1e8)?,
            volume_share_ppm: account.volume_share_ppm,
            staked_deopt_1e8: parse_u128(&account.staked_deopt_1e8)?,
        });
    }
    let generated_at_ms = current_unix_millis();
    let config = ArtifactConfig {
        chain_id: input.chain_id,
        fees_manager_v2: fees_manager,
        generated_at_ms,
        snapshot_config,
    };
    let artifact = generate_tier_artifact(&trader_inputs, config)
        .map_err(|error| format!("assemble artifact: {error}"))?;
    let serialised = serde_json::to_string_pretty(&artifact)
        .map_err(|error| format!("serialise artifact: {error}"))?;
    if let Some(parent) = cli.output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create {}: {error}", parent.display()))?;
        }
    }
    fs::write(&cli.output, &serialised)
        .map_err(|error| format!("write {}: {error}", cli.output.display()))?;
    println!(
        "wrote {} ({} rows, root {})",
        cli.output.display(),
        artifact.rows.len(),
        artifact.merkle_root
    );
    Ok(())
}

fn parse_args() -> Result<Cli, String> {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--input" => {
                input = Some(PathBuf::from(
                    args.next().ok_or("expected value after --input")?,
                ));
            }
            "--output" => {
                output = Some(PathBuf::from(
                    args.next().ok_or("expected value after --output")?,
                ));
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(Cli {
        input: input.ok_or("--input <path> is required")?,
        output: output.ok_or("--output <path> is required")?,
    })
}

fn parse_address(hex: &str) -> Result<[u8; 20], String> {
    let stripped = hex.strip_prefix("0x").unwrap_or(hex);
    if stripped.len() != 40 {
        return Err(format!("invalid address: {hex}"));
    }
    let mut bytes = [0u8; 20];
    for (idx, chunk) in stripped.as_bytes().chunks(2).enumerate() {
        let high = hex_nibble(chunk[0])?;
        let low = hex_nibble(chunk[1])?;
        bytes[idx] = (high << 4) | low;
    }
    Ok(bytes)
}

fn hex_nibble(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        other => Err(format!("invalid hex nibble: 0x{other:02x}")),
    }
}

fn parse_u128(value: &str) -> Result<u128, String> {
    value
        .parse::<u128>()
        .map_err(|error| format!("invalid u128 \"{value}\": {error}"))
}

fn current_unix_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
