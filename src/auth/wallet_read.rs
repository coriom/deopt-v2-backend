//! Wallet-scoped, read-only HTTP sessions. Disabled unless explicitly wired
//! into the VPS release. The signed challenge is generated server-side;
//! bearer tokens and signatures are never persisted or logged.
use crate::api::AppState;
use crate::error::{BackendError, Result};
use crate::types::now_ms;
use axum::extract::State;
use axum::http::{header::AUTHORIZATION, HeaderMap, StatusCode};
use axum::Json;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};

const CHALLENGE_TTL_MS: i64 = 5 * 60 * 1_000;
const SESSION_TTL_MS: i64 = 15 * 60 * 1_000;
const CHAIN_ID: u64 = 84532;
type HttpError = (StatusCode, &'static str);

#[derive(Clone, Debug, Default)]
pub struct WalletReadAuthConfig {
    pub enabled: bool,
    pub origin: String,
}

impl WalletReadAuthConfig {
    pub fn from_env(read_only: bool, has_database: bool, chain_id: u64) -> Result<Self> {
        let enabled = std::env::var("WALLET_READ_AUTH_ENABLED").unwrap_or_default();
        if !matches!(enabled.as_str(), "" | "false" | "true") {
            return Err(BackendError::Config(
                "invalid WALLET_READ_AUTH_ENABLED".into(),
            ));
        }
        if enabled != "true" {
            return Ok(Self::default());
        }
        if !read_only || !has_database || chain_id != CHAIN_ID {
            return Err(BackendError::Config(
                "wallet read auth requires read-only Base Sepolia with PostgreSQL".into(),
            ));
        }
        let origin = std::env::var("WALLET_READ_AUTH_ORIGIN")
            .map_err(|_| BackendError::Config("WALLET_READ_AUTH_ORIGIN required".into()))?;
        let parsed = reqwest::Url::parse(&origin)
            .map_err(|_| BackendError::Config("invalid wallet auth origin".into()))?;
        if parsed.scheme() != "https"
            || parsed.host_str().is_none()
            || parsed.port().is_some()
            || parsed.path() != "/"
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || origin.ends_with('/')
        {
            return Err(BackendError::Config(
                "wallet auth origin must be an HTTPS origin".into(),
            ));
        }
        Ok(Self {
            enabled: true,
            origin,
        })
    }
}

fn digest(value: &[u8]) -> Vec<u8> {
    Sha256::digest(value).to_vec()
}

fn random_hex<const N: usize>() -> String {
    let mut bytes = [0u8; N];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn checksum_address(lower: &str) -> String {
    let raw = &lower[2..];
    let hash = crate::signing::eip712::keccak256(raw.as_bytes());
    let mut out = String::from("0x");
    for (i, ch) in raw.bytes().enumerate() {
        let nibble = if i % 2 == 0 {
            hash[i / 2] >> 4
        } else {
            hash[i / 2] & 15
        };
        out.push(if nibble >= 8 {
            (ch as char).to_ascii_uppercase()
        } else {
            ch as char
        });
    }
    out
}

fn challenge_message(
    wallet: &str,
    origin: &str,
    nonce: &str,
    issued: i64,
    expires: i64,
) -> Option<String> {
    let host = origin.strip_prefix("https://").unwrap_or(origin);
    let issued = chrono::DateTime::from_timestamp_millis(issued)?;
    let expires = chrono::DateTime::from_timestamp_millis(expires)?;
    Some(format!(
        "{host} wants you to sign in with your Ethereum account:\n{}\n\nAccess DeOpt read-only wallet fee history.\n\nURI: {origin}\nVersion: 1\nChain ID: {CHAIN_ID}\nNonce: {nonce}\nIssued At: {}\nExpiration Time: {}",
        checksum_address(wallet), issued.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        expires.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    ))
}

async fn rate_limit(
    tx: &mut Transaction<'_, Postgres>,
    scope: &str,
    now: i64,
    max: i32,
) -> std::result::Result<(), HttpError> {
    let row = sqlx::query(
        "INSERT INTO wallet_read_rate (scope, minute_bucket, attempts) VALUES ($1, $2, 1) \
         ON CONFLICT (scope, minute_bucket) DO UPDATE SET attempts = wallet_read_rate.attempts + 1 \
         WHERE wallet_read_rate.attempts < $3 RETURNING attempts",
    )
    .bind(scope)
    .bind(now / 60_000)
    .bind(max)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "auth storage unavailable"))?;
    if row.is_none() {
        return Err((StatusCode::TOO_MANY_REQUESTS, "auth rate limit"));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChallengeRequest {
    pub address: String,
}
#[derive(Serialize)]
pub struct ChallengeResponse {
    pub nonce: String,
    pub message: String,
    pub expires_at_ms: i64,
}

pub async fn challenge(
    State(state): State<AppState>,
    Json(request): Json<ChallengeRequest>,
) -> std::result::Result<Json<ChallengeResponse>, HttpError> {
    if !state.wallet_read_auth.enabled || state.chain_id != CHAIN_ID {
        return Err((StatusCode::SERVICE_UNAVAILABLE, "wallet auth disabled"));
    }
    let wallet = crate::subaccounts::normalize_owner_address(&request.address)
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid wallet address"))?;
    let repo = state
        .repository
        .as_ref()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "auth storage unavailable"))?;
    let now = now_ms();
    let expires = now + CHALLENGE_TTL_MS;
    let nonce = random_hex::<16>();
    let message = challenge_message(
        &wallet,
        &state.wallet_read_auth.origin,
        &nonce,
        now,
        expires,
    )
    .ok_or((StatusCode::SERVICE_UNAVAILABLE, "invalid server clock"))?;
    let mut tx = repo
        .pool()
        .begin()
        .await
        .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "auth storage unavailable"))?;
    rate_limit(&mut tx, "challenge:global", now, 120).await?;
    rate_limit(&mut tx, &format!("challenge:{wallet}"), now, 5).await?;
    sqlx::query("INSERT INTO wallet_read_challenges (nonce_hash,wallet,origin,chain_id,message,issued_at_ms,expires_at_ms) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(digest(nonce.as_bytes())).bind(wallet).bind(&state.wallet_read_auth.origin)
        .bind(CHAIN_ID as i64).bind(&message).bind(now).bind(expires)
        .execute(&mut *tx).await.map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "auth storage unavailable"))?;
    tx.commit()
        .await
        .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "auth storage unavailable"))?;
    Ok(Json(ChallengeResponse {
        nonce,
        message,
        expires_at_ms: expires,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyRequest {
    pub nonce: String,
    pub signature: String,
}
#[derive(Serialize)]
pub struct VerifyResponse {
    pub wallet: String,
    pub token: String,
    pub expires_at_ms: i64,
}

pub async fn verify(
    State(state): State<AppState>,
    Json(request): Json<VerifyRequest>,
) -> std::result::Result<Json<VerifyResponse>, HttpError> {
    if !state.wallet_read_auth.enabled || state.chain_id != CHAIN_ID {
        return Err((StatusCode::SERVICE_UNAVAILABLE, "wallet auth disabled"));
    }
    if request.nonce.len() != 32
        || !request.nonce.bytes().all(|b| b.is_ascii_hexdigit())
        || request.signature.len() != 132
    {
        return Err((StatusCode::UNAUTHORIZED, "invalid wallet proof"));
    }
    let repo = state
        .repository
        .as_ref()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "auth storage unavailable"))?;
    let now = now_ms();
    let nonce_hash = digest(request.nonce.as_bytes());
    // Commit limits independently: failed signatures must still count.
    let mut global_rate = repo
        .pool()
        .begin()
        .await
        .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "auth storage unavailable"))?;
    rate_limit(&mut global_rate, "verify:global", now, 120).await?;
    global_rate
        .commit()
        .await
        .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "auth storage unavailable"))?;
    let wallet_for_rate: Option<String> = sqlx::query_scalar(
        "SELECT wallet FROM wallet_read_challenges WHERE nonce_hash=$1 AND expires_at_ms>$2",
    )
    .bind(&nonce_hash)
    .bind(now)
    .fetch_optional(repo.pool())
    .await
    .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "auth storage unavailable"))?;
    if let Some(wallet) = wallet_for_rate {
        let mut wallet_rate = repo
            .pool()
            .begin()
            .await
            .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "auth storage unavailable"))?;
        rate_limit(&mut wallet_rate, &format!("verify:{wallet}"), now, 10).await?;
        wallet_rate
            .commit()
            .await
            .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "auth storage unavailable"))?;
    }
    let mut tx = repo
        .pool()
        .begin()
        .await
        .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "auth storage unavailable"))?;
    let row = sqlx::query("SELECT wallet,origin,chain_id,message,issued_at_ms,expires_at_ms FROM wallet_read_challenges WHERE nonce_hash=$1 AND consumed_at_ms IS NULL AND expires_at_ms>$2 FOR UPDATE")
        .bind(&nonce_hash).bind(now).fetch_optional(&mut *tx).await
        .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "auth storage unavailable"))?
        .ok_or((StatusCode::UNAUTHORIZED, "invalid wallet proof"))?;
    let wallet: String = row.get("wallet");
    let origin: String = row.get("origin");
    let chain_id: i64 = row.get("chain_id");
    let issued: i64 = row.get("issued_at_ms");
    let expires: i64 = row.get("expires_at_ms");
    let message: String = row.get("message");
    if origin != state.wallet_read_auth.origin
        || chain_id != CHAIN_ID as i64
        || issued > now + 30_000
        || Some(message.as_str())
            != challenge_message(&wallet, &origin, &request.nonce, issued, expires).as_deref()
    {
        return Err((StatusCode::UNAUTHORIZED, "invalid wallet proof"));
    }
    let signer = crate::signing::recover_personal_signer(&message, &request.signature)
        .map_err(|_| (StatusCode::UNAUTHORIZED, "invalid wallet proof"))?;
    if !signer.0.eq_ignore_ascii_case(&wallet) {
        return Err((StatusCode::UNAUTHORIZED, "invalid wallet proof"));
    }
    let token = random_hex::<32>();
    let session_expiry = now + SESSION_TTL_MS;
    sqlx::query("UPDATE wallet_read_challenges SET consumed_at_ms=$2 WHERE nonce_hash=$1")
        .bind(&nonce_hash)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "auth storage unavailable"))?;
    sqlx::query("INSERT INTO wallet_read_sessions (token_hash,wallet,origin,chain_id,issued_at_ms,expires_at_ms) VALUES ($1,$2,$3,$4,$5,$6)")
        .bind(digest(token.as_bytes())).bind(&wallet).bind(&origin).bind(CHAIN_ID as i64)
        .bind(now).bind(session_expiry).execute(&mut *tx).await
        .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "auth storage unavailable"))?;
    tx.commit()
        .await
        .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "auth storage unavailable"))?;
    Ok(Json(VerifyResponse {
        wallet,
        token,
        expires_at_ms: session_expiry,
    }))
}

pub async fn session_wallet(
    state: &AppState,
    headers: &HeaderMap,
) -> std::result::Result<String, HttpError> {
    if !state.wallet_read_auth.enabled || state.chain_id != CHAIN_ID {
        return Err((StatusCode::UNAUTHORIZED, "wallet authentication required"));
    }
    let auth = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let token = auth.strip_prefix("Bearer ").unwrap_or("");
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err((StatusCode::UNAUTHORIZED, "wallet authentication required"));
    }
    let repo = state
        .repository
        .as_ref()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "auth storage unavailable"))?;
    let row = sqlx::query("SELECT wallet FROM wallet_read_sessions WHERE token_hash=$1 AND origin=$2 AND chain_id=$3 AND expires_at_ms>$4 AND revoked_at_ms IS NULL")
        .bind(digest(token.as_bytes())).bind(&state.wallet_read_auth.origin).bind(CHAIN_ID as i64).bind(now_ms())
        .fetch_optional(repo.pool()).await.map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "auth storage unavailable"))?
        .ok_or((StatusCode::UNAUTHORIZED, "wallet authentication required"))?;
    Ok(row.get("wallet"))
}

#[derive(Serialize)]
pub struct SessionResponse {
    pub wallet: String,
}
pub async fn session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> std::result::Result<Json<SessionResponse>, HttpError> {
    let wallet = session_wallet(&state, &headers).await?;
    Ok(Json(SessionResponse { wallet }))
}
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> std::result::Result<StatusCode, HttpError> {
    session_wallet(&state, &headers).await?;
    let token = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .strip_prefix("Bearer ")
        .unwrap_or("");
    let repo = state
        .repository
        .as_ref()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "auth storage unavailable"))?;
    sqlx::query("UPDATE wallet_read_sessions SET revoked_at_ms=$2 WHERE token_hash=$1 AND revoked_at_ms IS NULL")
        .bind(digest(token.as_bytes())).bind(now_ms()).execute(repo.pool()).await
        .map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "auth storage unavailable"))?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn siwe_address_uses_eip55_checksum() {
        assert_eq!(
            checksum_address("0x52908400098527886e0f7030069857d2e4169ee7"),
            "0x52908400098527886E0F7030069857D2E4169EE7"
        );
    }
    #[test]
    fn canonical_message_binds_wallet_chain_origin_nonce_and_time() {
        let wallet = "0x1111111111111111111111111111111111111111";
        let message = challenge_message(
            wallet,
            "https://deoptv2-frontend.vercel.app",
            "12345678901234567890123456789012",
            1_700_000_000_000,
            1_700_000_300_000,
        )
        .unwrap();
        assert!(message.contains("Chain ID: 84532"));
        assert!(message.contains("URI: https://deoptv2-frontend.vercel.app"));
        assert!(message.contains("Nonce: 12345678901234567890123456789012"));
        assert!(message.contains(wallet));
        assert_ne!(
            Some(message),
            challenge_message(
                wallet,
                "https://other.example",
                "12345678901234567890123456789012",
                1_700_000_000_000,
                1_700_000_300_000
            )
        );
    }
}
