//! Public WebSocket message dispatcher.
//!
//! Pure function: takes a parsed `ClientRequest` + a `&mut WsSession` +
//! a snapshot-aware `AppState`, and returns the frames that should be
//! sent back. The Axum handler is responsible for the WebSocket I/O and
//! periodic snapshots; it never embeds business logic.
//!
//! Splitting it this way means we can unit-test every code path
//! (subscribe / unsubscribe / auth / rate-limit / unknown method)
//! without standing up a real socket.

use super::config::PublicWsConfig;
use super::protocol::{
    build_canonical_challenge_message, Channel, ClientRequest, ServerNotification, ServerResponse,
    SubscribeParams, SubscriptionPushParams, UnsubscribeParams, WsError, WsErrorCode, WsMeta,
    PUBLIC_WS_AUTH_DOMAIN,
};
use super::session::{PendingChallenge, SubscriptionState, WsSession};
use super::snapshots::{build_snapshot, build_snapshot_for_address, SnapshotError};
use crate::api::AppState;
use crate::types::{now_ms, AccountId, TimestampMs};
use serde_json::{json, Value};
use uuid::Uuid;

/// Frames produced by the dispatcher for a single client request.
///
/// `response` is the JSON-RPC `result` / `error` reply to the request;
/// `pushes` is the (possibly empty) list of subscription push events
/// the server must emit as a side effect of handling the request — for
/// example, an immediate snapshot after a successful `subscribe`.
#[derive(Clone, Debug)]
pub struct DispatchOutcome {
    pub response: ServerResponse,
    pub pushes: Vec<ServerNotification>,
}

pub fn make_meta(state: &AppState) -> WsMeta {
    WsMeta {
        source: "backend",
        chain_id: state.chain_id,
        request_id: format!("req_{}", Uuid::new_v4()),
        generated_at_ms: now_ms(),
    }
}

fn build_push(
    subscription_id: &str,
    channel: Channel,
    seq: u64,
    chain_id: u64,
    data: Value,
    address: Option<String>,
) -> ServerNotification {
    ServerNotification {
        jsonrpc: "2.0",
        method: "subscription",
        params: SubscriptionPushParams {
            subscription_id: subscription_id.to_string(),
            channel: channel.as_str(),
            seq,
            event_id: format!("evt_{}", Uuid::new_v4()),
            source: "backend",
            chain_id,
            generated_at_ms: now_ms(),
            instrument_id: None,
            address,
            tx_hash: None,
            data,
        },
    }
}

/// Snapshot wrapped into a subscription push frame. Dispatches to the
/// public or private generator based on `channel.requires_auth()`.
/// Increments `next_seq` on the subscription and emits one frame.
async fn snapshot_push_for(
    session: &mut WsSession,
    sub_id: &str,
    channel: Channel,
    state: &AppState,
) -> Result<ServerNotification, WsError> {
    let snapshot_result = if channel.requires_auth() {
        match session.account.as_ref() {
            Some(addr) => build_snapshot_for_address(state, channel, &addr.0).await,
            None => Err(SnapshotError::NotImplemented),
        }
    } else {
        build_snapshot(state, channel).await
    };
    let data = snapshot_result.map_err(|e| match e {
        SnapshotError::SourceUnavailable(_) => {
            WsError::new(WsErrorCode::SourceUnavailable, "source unavailable")
        }
        SnapshotError::NotImplemented => WsError::new(
            WsErrorCode::NotImplemented,
            "channel snapshot not implemented",
        ),
    })?;
    // Bump the subscription's seq counter.
    let seq = if let Some(sub) = session.subscriptions.get_mut(sub_id) {
        let s = sub.next_seq;
        sub.next_seq = s.saturating_add(1);
        s
    } else {
        return Err(WsError::new(
            WsErrorCode::SubscriptionNotFound,
            "subscription disappeared between ack and snapshot",
        ));
    };
    Ok(build_push(
        sub_id,
        channel,
        seq,
        state.chain_id,
        data,
        session.account.as_ref().map(|a| a.0.clone()),
    ))
}

fn parse_params<T: serde::de::DeserializeOwned>(raw: Option<Value>) -> Result<T, WsError> {
    let value =
        raw.ok_or_else(|| WsError::new(WsErrorCode::InvalidParams, "missing params object"))?;
    serde_json::from_value(value)
        .map_err(|e| WsError::new(WsErrorCode::InvalidParams, format!("invalid params: {e}")))
}

fn lower_address(raw: &str) -> Option<String> {
    let t = raw.trim();
    if !t.starts_with("0x") || t.len() != 42 {
        return None;
    }
    Some(t.to_ascii_lowercase())
}

// ---------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------

pub async fn dispatch(
    state: &AppState,
    config: &PublicWsConfig,
    session: &mut WsSession,
    req: ClientRequest,
) -> DispatchOutcome {
    let meta = make_meta(state);
    let id = req.id.clone();
    if req.jsonrpc != "2.0" {
        return DispatchOutcome {
            response: ServerResponse::err(
                id,
                WsError::new(WsErrorCode::InvalidRequest, "jsonrpc must be \"2.0\""),
                meta,
            ),
            pushes: Vec::new(),
        };
    }
    match req.method.as_str() {
        "ping" => handle_ping(state, id, meta),
        "session.get" => handle_session_get(session, state, id, meta),
        "subscriptions" => handle_subscriptions(session, id, meta),
        "subscribe" => handle_subscribe(state, config, session, req.params, id, meta).await,
        "unsubscribe" => handle_unsubscribe(session, req.params, id, meta),
        "auth.challenge" => handle_auth_challenge(state, config, session, req.params, id, meta),
        "auth.verify" => handle_auth_verify(state, session, req.params, id, meta).await,
        other => DispatchOutcome {
            response: ServerResponse::err(
                id,
                WsError::new(
                    WsErrorCode::UnknownMethod,
                    format!("unknown method: {other}"),
                ),
                meta,
            ),
            pushes: Vec::new(),
        },
    }
}

fn handle_ping(state: &AppState, id: Option<Value>, meta: WsMeta) -> DispatchOutcome {
    DispatchOutcome {
        response: ServerResponse::ok(
            id,
            json!({"pong": true, "server_time_ms": now_ms(), "chain_id": state.chain_id}),
            meta,
        ),
        pushes: Vec::new(),
    }
}

fn handle_session_get(
    session: &WsSession,
    state: &AppState,
    id: Option<Value>,
    meta: WsMeta,
) -> DispatchOutcome {
    let result = json!({
        "connection_id": session.connection_id,
        "authenticated": session.is_authenticated(),
        "address": session.account.as_ref().map(|a| a.0.clone()),
        "chain_id": state.chain_id,
        "subscriptions": session.subscriptions.values().map(|s| {
            json!({
                "subscription_id": s.subscription_id,
                "channel": s.channel.as_str(),
                "next_seq": s.next_seq,
                "created_at_ms": s.created_at_ms,
            })
        }).collect::<Vec<_>>(),
        "connected_at_ms": session.connected_at_ms,
        "last_message_at_ms": session.last_message_at_ms,
    });
    DispatchOutcome {
        response: ServerResponse::ok(id, result, meta),
        pushes: Vec::new(),
    }
}

fn handle_subscriptions(session: &WsSession, id: Option<Value>, meta: WsMeta) -> DispatchOutcome {
    let items: Vec<Value> = session
        .subscriptions
        .values()
        .map(|s| {
            json!({
                "subscription_id": s.subscription_id,
                "channel": s.channel.as_str(),
            })
        })
        .collect();
    DispatchOutcome {
        response: ServerResponse::ok(id, json!({ "subscriptions": items }), meta),
        pushes: Vec::new(),
    }
}

async fn handle_subscribe(
    state: &AppState,
    config: &PublicWsConfig,
    session: &mut WsSession,
    params: Option<Value>,
    id: Option<Value>,
    meta: WsMeta,
) -> DispatchOutcome {
    let params: SubscribeParams = match parse_params(params) {
        Ok(p) => p,
        Err(e) => {
            return DispatchOutcome {
                response: ServerResponse::err(id, e, meta),
                pushes: Vec::new(),
            };
        }
    };
    let channel = match Channel::parse(&params.channel) {
        Some(c) => c,
        None => {
            return DispatchOutcome {
                response: ServerResponse::err(
                    id,
                    WsError::new(
                        WsErrorCode::InvalidChannel,
                        format!("unknown channel: {}", params.channel),
                    ),
                    meta,
                ),
                pushes: Vec::new(),
            };
        }
    };

    if channel.requires_auth() && !session.is_authenticated() {
        return DispatchOutcome {
            response: ServerResponse::err(
                id,
                WsError::new(
                    WsErrorCode::AuthRequired,
                    "this channel requires authentication; call auth.challenge + auth.verify first",
                ),
                meta,
            ),
            pushes: Vec::new(),
        };
    }

    // If the client supplied an explicit `address` for a private
    // channel, it MUST match the authenticated session address. We do
    // not silently use the bound address — surfacing the mismatch
    // protects against frontends accidentally querying for the wrong
    // wallet's data.
    if channel.requires_auth() {
        if let Some(supplied) = params.address.as_deref() {
            let supplied_lc = supplied.trim().to_ascii_lowercase();
            let bound = session
                .account
                .as_ref()
                .map(|a| a.0.to_ascii_lowercase())
                .unwrap_or_default();
            if supplied_lc != bound {
                return DispatchOutcome {
                    response: ServerResponse::err(
                        id,
                        WsError::new(
                            WsErrorCode::AuthAddressMismatch,
                            "supplied address does not match the authenticated session address",
                        ),
                        meta,
                    ),
                    pushes: Vec::new(),
                };
            }
        }
    }

    if session.subscription_for(channel).is_some() {
        return DispatchOutcome {
            response: ServerResponse::err(
                id,
                WsError::new(WsErrorCode::AlreadySubscribed, "channel already subscribed"),
                meta,
            ),
            pushes: Vec::new(),
        };
    }

    if session.subscription_count() >= config.max_subscriptions_per_connection {
        return DispatchOutcome {
            response: ServerResponse::err(
                id,
                WsError::new(
                    WsErrorCode::TooManySubscriptions,
                    "subscription cap reached for this connection",
                ),
                meta,
            ),
            pushes: Vec::new(),
        };
    }

    let subscription_id = format!("sub_{}", Uuid::new_v4());
    session.subscriptions.insert(
        subscription_id.clone(),
        SubscriptionState {
            subscription_id: subscription_id.clone(),
            channel,
            created_at_ms: now_ms(),
            next_seq: 0,
        },
    );
    let mut pushes = Vec::new();
    match snapshot_push_for(session, &subscription_id, channel, state).await {
        Ok(push) => pushes.push(push),
        Err(e) => {
            // Snapshot generation failed AFTER we already committed the
            // subscription. Roll the subscription back so the client
            // sees a clean error and is free to retry.
            session.subscriptions.remove(&subscription_id);
            return DispatchOutcome {
                response: ServerResponse::err(id, e, meta),
                pushes: Vec::new(),
            };
        }
    }
    DispatchOutcome {
        response: ServerResponse::ok(
            id,
            json!({
                "subscribed": true,
                "subscription_id": subscription_id,
                "channel": channel.as_str(),
            }),
            meta,
        ),
        pushes,
    }
}

fn handle_unsubscribe(
    session: &mut WsSession,
    params: Option<Value>,
    id: Option<Value>,
    meta: WsMeta,
) -> DispatchOutcome {
    let params: UnsubscribeParams = match parse_params(params) {
        Ok(p) => p,
        Err(e) => {
            return DispatchOutcome {
                response: ServerResponse::err(id, e, meta),
                pushes: Vec::new(),
            };
        }
    };
    if session
        .subscriptions
        .remove(&params.subscription_id)
        .is_none()
    {
        return DispatchOutcome {
            response: ServerResponse::err(
                id,
                WsError::new(
                    WsErrorCode::SubscriptionNotFound,
                    "no subscription with that id",
                ),
                meta,
            ),
            pushes: Vec::new(),
        };
    }
    DispatchOutcome {
        response: ServerResponse::ok(
            id,
            json!({
                "unsubscribed": true,
                "subscription_id": params.subscription_id,
            }),
            meta,
        ),
        pushes: Vec::new(),
    }
}

fn handle_auth_challenge(
    state: &AppState,
    config: &PublicWsConfig,
    session: &mut WsSession,
    params: Option<Value>,
    id: Option<Value>,
    meta: WsMeta,
) -> DispatchOutcome {
    let params: super::protocol::AuthChallengeParams = match parse_params(params) {
        Ok(p) => p,
        Err(e) => {
            return DispatchOutcome {
                response: ServerResponse::err(id, e, meta),
                pushes: Vec::new(),
            };
        }
    };
    let lc = match lower_address(&params.address) {
        Some(v) => v,
        None => {
            return DispatchOutcome {
                response: ServerResponse::err(
                    id,
                    WsError::new(
                        WsErrorCode::InvalidParams,
                        "address malformed; expected a 0x-prefixed 20-byte EVM address",
                    ),
                    meta,
                ),
                pushes: Vec::new(),
            };
        }
    };
    // Generate a random nonce. We never sign anything server-side —
    // the wallet performs the signature and `auth.verify` recovers the
    // signer address from the canonical challenge bytes + the
    // signature.
    let nonce = format!("nonce_{}", Uuid::new_v4());
    let issued_at = now_ms();
    let expires_at = issued_at.saturating_add(config.challenge_ttl_ms as TimestampMs);
    let domain = PUBLIC_WS_AUTH_DOMAIN.to_string();
    let canonical = build_canonical_challenge_message(
        &lc,
        state.chain_id,
        &nonce,
        issued_at,
        expires_at,
        &domain,
    );
    session.challenges.insert(
        lc.clone(),
        PendingChallenge {
            nonce: nonce.clone(),
            address: AccountId::new(lc.clone()),
            issued_at_ms: issued_at,
            expires_at_ms: expires_at,
            chain_id: state.chain_id,
            domain: domain.clone(),
        },
    );
    DispatchOutcome {
        response: ServerResponse::ok(
            id,
            json!({
                "address": lc,
                "nonce": nonce,
                "domain": domain,
                "chain_id": state.chain_id,
                "issued_at_ms": issued_at,
                "expires_at_ms": expires_at,
                "message": canonical,
            }),
            meta,
        ),
        pushes: Vec::new(),
    }
}

async fn handle_auth_verify(
    state: &AppState,
    session: &mut WsSession,
    params: Option<Value>,
    id: Option<Value>,
    meta: WsMeta,
) -> DispatchOutcome {
    let params: super::protocol::AuthVerifyParams = match parse_params(params) {
        Ok(p) => p,
        Err(e) => {
            return DispatchOutcome {
                response: ServerResponse::err(id, e, meta),
                pushes: Vec::new(),
            };
        }
    };

    // Normalize the supplied address and look up the matching pending
    // challenge. The lookup is keyed by the lower-cased address; the
    // signature recovery later proves the wallet actually owns it.
    let supplied_lc = match lower_address(&params.address) {
        Some(v) => v,
        None => {
            return DispatchOutcome {
                response: ServerResponse::err(
                    id,
                    WsError::new(
                        WsErrorCode::InvalidAddress,
                        "address malformed; expected a 0x-prefixed 20-byte EVM address",
                    ),
                    meta,
                ),
                pushes: Vec::new(),
            };
        }
    };

    let now = now_ms();
    session.prune_expired_challenges(now);

    // Take (not just read) the challenge so single-use is enforced
    // regardless of the verify outcome below. A failed verify still
    // consumes the nonce — the wallet must ask for a fresh challenge.
    let challenge = match session.challenges.remove(&supplied_lc) {
        Some(c) => c,
        None => {
            return DispatchOutcome {
                response: ServerResponse::err(
                    id,
                    WsError::new(
                        WsErrorCode::AuthChallengeNotFound,
                        "no active challenge for this address; call auth.challenge first",
                    ),
                    meta,
                ),
                pushes: Vec::new(),
            };
        }
    };

    if challenge.expires_at_ms <= now {
        return DispatchOutcome {
            response: ServerResponse::err(
                id,
                WsError::new(
                    WsErrorCode::AuthExpired,
                    "challenge expired; call auth.challenge for a fresh nonce",
                ),
                meta,
            ),
            pushes: Vec::new(),
        };
    }

    let canonical = build_canonical_challenge_message(
        &challenge.address.0,
        challenge.chain_id,
        &challenge.nonce,
        challenge.issued_at_ms,
        challenge.expires_at_ms,
        &challenge.domain,
    );

    let recovered = match crate::signing::recover_personal_signer(&canonical, &params.signature) {
        Ok(addr) => addr,
        Err(_) => {
            return DispatchOutcome {
                response: ServerResponse::err(
                    id,
                    WsError::new(
                        WsErrorCode::AuthInvalidSignature,
                        "signature did not recover a valid signer for the canonical challenge",
                    ),
                    meta,
                ),
                pushes: Vec::new(),
            };
        }
    };

    if !recovered.0.eq_ignore_ascii_case(&supplied_lc) {
        return DispatchOutcome {
            response: ServerResponse::err(
                id,
                WsError::new(
                    WsErrorCode::AuthAddressMismatch,
                    "recovered signer does not match the supplied address",
                ),
                meta,
            ),
            pushes: Vec::new(),
        };
    }

    // Success: bind the session to the canonical lower-cased form so
    // every downstream auth check uses an identical string.
    //
    // OPTIONS-HYBRID-V2-BACKEND-FINAL-CLOSURE-V1 Part M — if a session
    // rebinds to a different address, its existing subscriptions
    // become stale. Clear them so the client must re-subscribe under
    // the new identity. Data leakage was already prevented by the
    // two-gate filter in `handler.rs::forward_lifecycle_event`; this
    // is defense-in-depth against UI-side confusion when a session's
    // wallet identity changes.
    let bound_address = recovered.0.to_ascii_lowercase();
    let new_account = AccountId::new(bound_address.clone());
    let identity_changed = session
        .account
        .as_ref()
        .map(|prev| !prev.0.eq_ignore_ascii_case(&new_account.0))
        .unwrap_or(false);
    session.account = Some(new_account);
    if identity_changed {
        session.subscriptions.clear();
    }

    DispatchOutcome {
        response: ServerResponse::ok(
            id,
            json!({
                "authenticated": true,
                "address": bound_address,
                "expires_at_ms": challenge.expires_at_ms,
                "chain_id": state.chain_id,
            }),
            meta,
        ),
        pushes: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EngineState;

    fn test_state() -> AppState {
        let mut s = AppState::new(EngineState::new(Vec::new()));
        s.chain_id = 31337;
        s.network_name = "anvil".to_string();
        s.options_config.enabled = true;
        s
    }

    fn req(method: &str, params: Option<Value>) -> ClientRequest {
        ClientRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(Value::String("req_1".to_string())),
            method: method.to_string(),
            params,
        }
    }

    #[tokio::test]
    async fn dispatch_rejects_non_2_0_jsonrpc() {
        let state = test_state();
        let cfg = PublicWsConfig::default_testnet();
        let mut sess = WsSession::new("c1".to_string(), 0);
        let outcome = dispatch(
            &state,
            &cfg,
            &mut sess,
            ClientRequest {
                jsonrpc: "1.0".to_string(),
                id: None,
                method: "ping".to_string(),
                params: None,
            },
        )
        .await;
        let err = outcome.response.error.expect("error");
        assert_eq!(err.code, "INVALID_REQUEST");
    }

    #[tokio::test]
    async fn dispatch_ping_returns_pong_and_chain_id() {
        let state = test_state();
        let cfg = PublicWsConfig::default_testnet();
        let mut sess = WsSession::new("c1".to_string(), 0);
        let outcome = dispatch(&state, &cfg, &mut sess, req("ping", None)).await;
        let result = outcome.response.result.expect("result");
        assert_eq!(result["pong"], true);
        assert_eq!(result["chain_id"], 31337);
    }

    #[tokio::test]
    async fn dispatch_unknown_method_returns_unknown_method() {
        let state = test_state();
        let cfg = PublicWsConfig::default_testnet();
        let mut sess = WsSession::new("c1".to_string(), 0);
        let outcome = dispatch(&state, &cfg, &mut sess, req("not-a-method", None)).await;
        assert_eq!(
            outcome.response.error.expect("error").code,
            "UNKNOWN_METHOD"
        );
    }

    #[tokio::test]
    async fn subscribe_to_invalid_channel_returns_invalid_channel() {
        let state = test_state();
        let cfg = PublicWsConfig::default_testnet();
        let mut sess = WsSession::new("c1".to_string(), 0);
        let outcome = dispatch(
            &state,
            &cfg,
            &mut sess,
            req("subscribe", Some(json!({ "channel": "garbage" }))),
        )
        .await;
        assert_eq!(
            outcome.response.error.expect("error").code,
            "INVALID_CHANNEL"
        );
        assert!(sess.subscriptions.is_empty());
    }

    #[tokio::test]
    async fn subscribe_to_trading_health_acks_and_pushes_snapshot() {
        let state = test_state();
        let cfg = PublicWsConfig::default_testnet();
        let mut sess = WsSession::new("c1".to_string(), 0);
        let outcome = dispatch(
            &state,
            &cfg,
            &mut sess,
            req("subscribe", Some(json!({ "channel": "trading.health" }))),
        )
        .await;
        let result = outcome.response.result.expect("result");
        assert_eq!(result["subscribed"], true);
        assert_eq!(result["channel"], "trading.health");
        assert_eq!(outcome.pushes.len(), 1);
        let push = &outcome.pushes[0];
        assert_eq!(push.method, "subscription");
        assert_eq!(push.params.channel, "trading.health");
        assert_eq!(push.params.seq, 0);
        assert_eq!(sess.subscriptions.len(), 1);
    }

    #[tokio::test]
    async fn subscribing_twice_returns_already_subscribed() {
        let state = test_state();
        let cfg = PublicWsConfig::default_testnet();
        let mut sess = WsSession::new("c1".to_string(), 0);
        let _ = dispatch(
            &state,
            &cfg,
            &mut sess,
            req("subscribe", Some(json!({ "channel": "trading.health" }))),
        )
        .await;
        let outcome = dispatch(
            &state,
            &cfg,
            &mut sess,
            req("subscribe", Some(json!({ "channel": "trading.health" }))),
        )
        .await;
        assert_eq!(
            outcome.response.error.expect("error").code,
            "ALREADY_SUBSCRIBED"
        );
    }

    #[tokio::test]
    async fn account_channel_requires_auth() {
        let state = test_state();
        let cfg = PublicWsConfig::default_testnet();
        let mut sess = WsSession::new("c1".to_string(), 0);
        let outcome = dispatch(
            &state,
            &cfg,
            &mut sess,
            req("subscribe", Some(json!({ "channel": "account.positions" }))),
        )
        .await;
        assert_eq!(outcome.response.error.expect("error").code, "AUTH_REQUIRED");
        assert!(sess.subscriptions.is_empty());
    }

    #[tokio::test]
    async fn unsubscribe_removes_the_subscription() {
        let state = test_state();
        let cfg = PublicWsConfig::default_testnet();
        let mut sess = WsSession::new("c1".to_string(), 0);
        let ack = dispatch(
            &state,
            &cfg,
            &mut sess,
            req("subscribe", Some(json!({ "channel": "leaderboard" }))),
        )
        .await;
        let sub_id = ack.response.result.unwrap()["subscription_id"]
            .as_str()
            .unwrap()
            .to_string();
        let outcome = dispatch(
            &state,
            &cfg,
            &mut sess,
            req("unsubscribe", Some(json!({ "subscription_id": sub_id }))),
        )
        .await;
        let result = outcome.response.result.expect("result");
        assert_eq!(result["unsubscribed"], true);
        assert!(sess.subscriptions.is_empty());
    }

    #[tokio::test]
    async fn unsubscribe_unknown_id_returns_subscription_not_found() {
        let state = test_state();
        let cfg = PublicWsConfig::default_testnet();
        let mut sess = WsSession::new("c1".to_string(), 0);
        let outcome = dispatch(
            &state,
            &cfg,
            &mut sess,
            req(
                "unsubscribe",
                Some(json!({ "subscription_id": "sub_does_not_exist" })),
            ),
        )
        .await;
        assert_eq!(
            outcome.response.error.expect("error").code,
            "SUBSCRIPTION_NOT_FOUND"
        );
    }

    #[tokio::test]
    async fn auth_challenge_returns_nonce_and_expiry_and_persists_in_session() {
        let state = test_state();
        let cfg = PublicWsConfig::default_testnet();
        let mut sess = WsSession::new("c1".to_string(), 0);
        let outcome = dispatch(
            &state,
            &cfg,
            &mut sess,
            req(
                "auth.challenge",
                Some(json!({
                    "address": "0x1234567890abcdef1234567890abcdef12345678"
                })),
            ),
        )
        .await;
        let result = outcome.response.result.expect("result");
        assert!(result["nonce"].as_str().unwrap().starts_with("nonce_"));
        assert!(result["expires_at_ms"].as_i64().unwrap() > 0);
        assert_eq!(sess.challenges.len(), 1);
    }

    #[tokio::test]
    async fn auth_challenge_rejects_malformed_address() {
        let state = test_state();
        let cfg = PublicWsConfig::default_testnet();
        let mut sess = WsSession::new("c1".to_string(), 0);
        let outcome = dispatch(
            &state,
            &cfg,
            &mut sess,
            req(
                "auth.challenge",
                Some(json!({ "address": "not-an-address" })),
            ),
        )
        .await;
        assert_eq!(
            outcome.response.error.expect("error").code,
            "INVALID_PARAMS"
        );
        assert!(sess.challenges.is_empty());
    }

    // ---------- BACKEND-PUBLIC-WS-AUTH-V1 ----------
    //
    // The auth tests below exercise the real EIP-191 personal-sign
    // verification path. To produce valid signatures inside the unit
    // tests we re-use the existing `k256` + `personal_sign_digest`
    // utilities; no off-chain signer is required.

    use k256::ecdsa::SigningKey;

    /// Deterministic 32-byte secret keyed by the supplied tag so each
    /// test gets a distinct wallet without sharing a fixture across
    /// the suite.
    fn signing_key_for(tag: &str) -> SigningKey {
        let mut bytes = [0u8; 32];
        let tag_bytes = tag.as_bytes();
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = tag_bytes[i % tag_bytes.len()].wrapping_add(i as u8);
        }
        // k256 rejects "all-zero" keys; the tag-derived bytes above
        // can never collapse to zero for a non-empty tag.
        SigningKey::from_slice(&bytes).expect("non-zero secret")
    }

    fn address_for(key: &SigningKey) -> String {
        let encoded = key.verifying_key().to_encoded_point(false);
        let public = encoded.as_bytes();
        let hash = crate::signing::eip712::keccak256(&public[1..]);
        let mut hex = String::from("0x");
        for byte in &hash[12..] {
            hex.push_str(&format!("{:02x}", byte));
        }
        hex
    }

    fn personal_sign(key: &SigningKey, message: &str) -> String {
        let digest = crate::signing::personal_sign_digest(message);
        let (sig, recovery) = key.sign_prehash_recoverable(&digest).expect("sign prehash");
        let mut bytes = [0u8; 65];
        bytes[..32].copy_from_slice(&sig.r().to_bytes());
        bytes[32..64].copy_from_slice(&sig.s().to_bytes());
        // The shared `signature_v_to_recovery_id` accepts 0|1 and 27|28
        // — we emit the raw recovery byte so the test exercises the
        // 0|1 branch end-to-end.
        bytes[64] = recovery.to_byte();
        let mut hex = String::from("0x");
        for byte in &bytes {
            hex.push_str(&format!("{:02x}", byte));
        }
        hex
    }

    async fn issue_challenge(
        state: &AppState,
        cfg: &PublicWsConfig,
        sess: &mut WsSession,
        address: &str,
    ) -> serde_json::Value {
        let outcome = dispatch(
            state,
            cfg,
            sess,
            req("auth.challenge", Some(json!({ "address": address }))),
        )
        .await;
        outcome.response.result.expect("challenge ok")
    }

    fn canonical_message_from_challenge(result: &serde_json::Value) -> String {
        result["message"]
            .as_str()
            .expect("challenge message present")
            .to_string()
    }

    #[tokio::test]
    async fn auth_verify_without_challenge_returns_challenge_not_found() {
        let state = test_state();
        let cfg = PublicWsConfig::default_testnet();
        let mut sess = WsSession::new("c1".to_string(), 0);
        let outcome = dispatch(
            &state,
            &cfg,
            &mut sess,
            req(
                "auth.verify",
                Some(json!({
                    "address": "0x1234567890abcdef1234567890abcdef12345678",
                    "signature": "0x".to_string() + &"00".repeat(65),
                })),
            ),
        )
        .await;
        assert_eq!(
            outcome.response.error.expect("error").code,
            "AUTH_CHALLENGE_NOT_FOUND"
        );
        assert!(!sess.is_authenticated());
    }

    #[tokio::test]
    async fn auth_verify_rejects_malformed_address() {
        let state = test_state();
        let cfg = PublicWsConfig::default_testnet();
        let mut sess = WsSession::new("c1".to_string(), 0);
        let outcome = dispatch(
            &state,
            &cfg,
            &mut sess,
            req(
                "auth.verify",
                Some(json!({
                    "address": "not-an-address",
                    "signature": "0x".to_string() + &"00".repeat(65),
                })),
            ),
        )
        .await;
        assert_eq!(
            outcome.response.error.expect("error").code,
            "INVALID_ADDRESS"
        );
        assert!(!sess.is_authenticated());
    }

    #[tokio::test]
    async fn auth_verify_rejects_malformed_signature() {
        let state = test_state();
        let cfg = PublicWsConfig::default_testnet();
        let mut sess = WsSession::new("c1".to_string(), 0);
        let key = signing_key_for("alpha");
        let addr = address_for(&key);
        let _ = issue_challenge(&state, &cfg, &mut sess, &addr).await;
        let outcome = dispatch(
            &state,
            &cfg,
            &mut sess,
            req(
                "auth.verify",
                Some(json!({
                    "address": addr,
                    "signature": "0xdeadbeef",
                })),
            ),
        )
        .await;
        assert_eq!(
            outcome.response.error.expect("error").code,
            "AUTH_INVALID_SIGNATURE"
        );
        // The challenge was consumed (single-use enforcement).
        assert!(sess.challenges.is_empty());
        assert!(!sess.is_authenticated());
    }

    #[tokio::test]
    async fn auth_verify_rejects_signature_from_a_different_wallet() {
        let state = test_state();
        let cfg = PublicWsConfig::default_testnet();
        let mut sess = WsSession::new("c1".to_string(), 0);
        let alice = signing_key_for("alice-v1");
        let mallory = signing_key_for("mallory-v1");
        let alice_addr = address_for(&alice);
        let challenge = issue_challenge(&state, &cfg, &mut sess, &alice_addr).await;
        let canonical = canonical_message_from_challenge(&challenge);
        // Mallory signs Alice's challenge — recovery will yield
        // Mallory's address, which mismatches the supplied alice_addr.
        let mallory_sig = personal_sign(&mallory, &canonical);
        let outcome = dispatch(
            &state,
            &cfg,
            &mut sess,
            req(
                "auth.verify",
                Some(json!({
                    "address": alice_addr,
                    "signature": mallory_sig,
                })),
            ),
        )
        .await;
        assert_eq!(
            outcome.response.error.expect("error").code,
            "AUTH_ADDRESS_MISMATCH"
        );
        assert!(!sess.is_authenticated());
    }

    #[tokio::test]
    async fn auth_verify_rejects_expired_challenge() {
        let state = test_state();
        let cfg = PublicWsConfig::default_testnet();
        let mut sess = WsSession::new("c1".to_string(), 0);
        let key = signing_key_for("expired");
        let addr = address_for(&key);
        // Hand-stitch an already-expired challenge so the test does
        // not depend on wall-clock pauses.
        let domain = super::super::protocol::PUBLIC_WS_AUTH_DOMAIN.to_string();
        let nonce = "nonce_expired".to_string();
        let canonical = super::super::protocol::build_canonical_challenge_message(
            &addr.to_ascii_lowercase(),
            state.chain_id,
            &nonce,
            0,
            1, // expired at t=1ms; now_ms() is far in the future.
            &domain,
        );
        sess.challenges.insert(
            addr.to_ascii_lowercase(),
            PendingChallenge {
                nonce,
                address: AccountId::new(addr.to_ascii_lowercase()),
                issued_at_ms: 0,
                expires_at_ms: 1,
                chain_id: state.chain_id,
                domain,
            },
        );
        let sig = personal_sign(&key, &canonical);
        let outcome = dispatch(
            &state,
            &cfg,
            &mut sess,
            req(
                "auth.verify",
                Some(json!({ "address": addr, "signature": sig })),
            ),
        )
        .await;
        let code = outcome.response.error.expect("error").code;
        // Either AUTH_EXPIRED (challenge present + expired) or
        // AUTH_CHALLENGE_NOT_FOUND (pruned by prune_expired_challenges).
        // Both behaviours are honest; today the prune step runs first
        // so the second branch fires. Test for either to be robust to
        // small reorderings.
        assert!(
            code == "AUTH_EXPIRED" || code == "AUTH_CHALLENGE_NOT_FOUND",
            "unexpected code: {}",
            code
        );
        assert!(!sess.is_authenticated());
    }

    #[tokio::test]
    async fn auth_verify_happy_path_binds_session_to_recovered_address() {
        let state = test_state();
        let cfg = PublicWsConfig::default_testnet();
        let mut sess = WsSession::new("c1".to_string(), 0);
        let key = signing_key_for("happy");
        let addr = address_for(&key);
        let challenge = issue_challenge(&state, &cfg, &mut sess, &addr).await;
        let canonical = canonical_message_from_challenge(&challenge);
        let sig = personal_sign(&key, &canonical);
        let outcome = dispatch(
            &state,
            &cfg,
            &mut sess,
            req(
                "auth.verify",
                Some(json!({ "address": addr, "signature": sig })),
            ),
        )
        .await;
        let result = outcome.response.result.expect("result");
        assert_eq!(result["authenticated"], true);
        assert_eq!(result["address"], addr.to_ascii_lowercase());
        assert_eq!(result["chain_id"], 31337);
        assert!(sess.is_authenticated());
        assert_eq!(sess.account.as_ref().unwrap().0, addr.to_ascii_lowercase());
        // Single-use: the challenge is gone.
        assert!(sess.challenges.is_empty());
    }

    #[tokio::test]
    async fn auth_verify_replay_after_success_returns_challenge_not_found() {
        let state = test_state();
        let cfg = PublicWsConfig::default_testnet();
        let mut sess = WsSession::new("c1".to_string(), 0);
        let key = signing_key_for("replay");
        let addr = address_for(&key);
        let challenge = issue_challenge(&state, &cfg, &mut sess, &addr).await;
        let canonical = canonical_message_from_challenge(&challenge);
        let sig = personal_sign(&key, &canonical);
        // First call binds the session.
        let _ = dispatch(
            &state,
            &cfg,
            &mut sess,
            req(
                "auth.verify",
                Some(json!({ "address": addr, "signature": sig.clone() })),
            ),
        )
        .await;
        // Second call MUST fail — the nonce is single-use.
        let outcome = dispatch(
            &state,
            &cfg,
            &mut sess,
            req(
                "auth.verify",
                Some(json!({ "address": addr, "signature": sig })),
            ),
        )
        .await;
        assert_eq!(
            outcome.response.error.expect("error").code,
            "AUTH_CHALLENGE_NOT_FOUND"
        );
    }

    #[tokio::test]
    async fn session_get_after_auth_shows_authenticated_address() {
        let state = test_state();
        let cfg = PublicWsConfig::default_testnet();
        let mut sess = WsSession::new("c1".to_string(), 0);
        let key = signing_key_for("session-get");
        let addr = address_for(&key);
        let challenge = issue_challenge(&state, &cfg, &mut sess, &addr).await;
        let canonical = canonical_message_from_challenge(&challenge);
        let sig = personal_sign(&key, &canonical);
        let _ = dispatch(
            &state,
            &cfg,
            &mut sess,
            req(
                "auth.verify",
                Some(json!({ "address": addr, "signature": sig })),
            ),
        )
        .await;
        let outcome = dispatch(&state, &cfg, &mut sess, req("session.get", None)).await;
        let result = outcome.response.result.expect("result");
        assert_eq!(result["authenticated"], true);
        assert_eq!(result["address"], addr.to_ascii_lowercase());
    }

    #[tokio::test]
    async fn private_subscribe_after_auth_succeeds_for_bound_address() {
        let state = test_state();
        let cfg = PublicWsConfig::default_testnet();
        let mut sess = WsSession::new("c1".to_string(), 0);
        let key = signing_key_for("private-ok");
        let addr = address_for(&key);
        let challenge = issue_challenge(&state, &cfg, &mut sess, &addr).await;
        let canonical = canonical_message_from_challenge(&challenge);
        let sig = personal_sign(&key, &canonical);
        let _ = dispatch(
            &state,
            &cfg,
            &mut sess,
            req(
                "auth.verify",
                Some(json!({ "address": addr, "signature": sig })),
            ),
        )
        .await;
        // Now subscribe to a private channel WITHOUT an `address`
        // param — server must use the bound address.
        let outcome = dispatch(
            &state,
            &cfg,
            &mut sess,
            req("subscribe", Some(json!({ "channel": "account.positions" }))),
        )
        .await;
        let result = outcome.response.result.expect("result");
        assert_eq!(result["subscribed"], true);
        assert_eq!(result["channel"], "account.positions");
        assert_eq!(outcome.pushes.len(), 1);
        let push = &outcome.pushes[0];
        assert_eq!(push.params.channel, "account.positions");
        // The push frame's `address` mirrors the bound session address.
        assert_eq!(
            push.params.address.as_deref(),
            Some(addr.to_ascii_lowercase().as_str())
        );
    }

    #[tokio::test]
    async fn private_subscribe_rejects_address_override_for_another_wallet() {
        let state = test_state();
        let cfg = PublicWsConfig::default_testnet();
        let mut sess = WsSession::new("c1".to_string(), 0);
        let key = signing_key_for("address-override");
        let addr = address_for(&key);
        let challenge = issue_challenge(&state, &cfg, &mut sess, &addr).await;
        let canonical = canonical_message_from_challenge(&challenge);
        let sig = personal_sign(&key, &canonical);
        let _ = dispatch(
            &state,
            &cfg,
            &mut sess,
            req(
                "auth.verify",
                Some(json!({ "address": addr, "signature": sig })),
            ),
        )
        .await;
        // Now try to subscribe pretending to be someone else.
        let outcome = dispatch(
            &state,
            &cfg,
            &mut sess,
            req(
                "subscribe",
                Some(json!({
                    "channel": "account.positions",
                    "address": "0xdeaddeaddeaddeaddeaddeaddeaddeaddeaddead",
                })),
            ),
        )
        .await;
        assert_eq!(
            outcome.response.error.expect("error").code,
            "AUTH_ADDRESS_MISMATCH"
        );
        // No subscription was created.
        assert!(sess
            .subscriptions
            .values()
            .all(|s| s.channel != Channel::AccountPositions));
    }

    #[tokio::test]
    async fn private_subscribe_before_auth_returns_auth_required() {
        // Sanity-check that the brief's pre-auth refusal still holds
        // — covered by the V1 test too, repeated here so the auth
        // suite is self-contained.
        let state = test_state();
        let cfg = PublicWsConfig::default_testnet();
        let mut sess = WsSession::new("c1".to_string(), 0);
        let outcome = dispatch(
            &state,
            &cfg,
            &mut sess,
            req("subscribe", Some(json!({ "channel": "account.history" }))),
        )
        .await;
        assert_eq!(outcome.response.error.expect("error").code, "AUTH_REQUIRED");
    }

    #[tokio::test]
    async fn session_get_returns_self_state() {
        let state = test_state();
        let cfg = PublicWsConfig::default_testnet();
        let mut sess = WsSession::new("c1".to_string(), 0);
        let _ = dispatch(
            &state,
            &cfg,
            &mut sess,
            req("subscribe", Some(json!({ "channel": "trading.health" }))),
        )
        .await;
        let outcome = dispatch(&state, &cfg, &mut sess, req("session.get", None)).await;
        let result = outcome.response.result.expect("result");
        assert_eq!(result["authenticated"], false);
        assert_eq!(result["connection_id"], "c1");
        assert_eq!(result["chain_id"], 31337);
        assert_eq!(result["subscriptions"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn subscribe_respects_max_subscriptions_per_connection() {
        let state = test_state();
        let mut cfg = PublicWsConfig::default_testnet();
        cfg.max_subscriptions_per_connection = 1;
        let mut sess = WsSession::new("c1".to_string(), 0);
        let _ = dispatch(
            &state,
            &cfg,
            &mut sess,
            req("subscribe", Some(json!({ "channel": "trading.health" }))),
        )
        .await;
        let outcome = dispatch(
            &state,
            &cfg,
            &mut sess,
            req("subscribe", Some(json!({ "channel": "leaderboard" }))),
        )
        .await;
        assert_eq!(
            outcome.response.error.expect("error").code,
            "TOO_MANY_SUBSCRIPTIONS"
        );
    }
}
