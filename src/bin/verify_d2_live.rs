//! Read-only verification of the five executed Base Sepolia D2 fee tiers.
//! Receipt hashes and RPC URL are supplied by the operator; no signer is used.
use deopt_v2_backend::fees::d2_onchain::verify_d2_fee_policy;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let rpc = std::env::var("BASE_SEPOLIA_RPC_URL").unwrap_or_default();
    let hashes = std::env::var("D2_PERP_TIER_S2_TX_HASHES").unwrap_or_default();
    let hashes: Vec<String> = hashes
        .split(',')
        .map(str::trim)
        .map(str::to_owned)
        .collect();
    let Ok(hashes): Result<[String; 5], _> = hashes.try_into() else {
        eprintln!("D2 policy NOT_VERIFIED: exactly five receipt hashes required");
        std::process::exit(2);
    };
    if rpc.is_empty() {
        eprintln!("D2 policy NOT_VERIFIED: read-only RPC URL required");
        std::process::exit(2);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or_default();
    match verify_d2_fee_policy(&rpc, now, 300, &hashes).await {
        Ok(policy) => {
            let snapshot = policy.snapshot();
            println!("D2 policy ONCHAIN_VERIFIED chain={} block={} hash={} profiles={:?} recipient={} latest_execution_block={} trading_authorized=false",
                snapshot.chain_id, snapshot.block_number, snapshot.block_hash,
                snapshot.profiles, snapshot.recipient, policy.latest_execution_block());
        }
        Err(error) => {
            let reason = if error.contains("http://") || error.contains("https://") {
                "RPC transport error (endpoint suppressed)"
            } else {
                &error
            };
            eprintln!("D2 policy NOT_VERIFIED: {reason}");
            std::process::exit(1);
        }
    }
}
