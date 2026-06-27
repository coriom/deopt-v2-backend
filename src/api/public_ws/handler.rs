//! Axum WebSocket handler for the public WS API.
//!
//! Responsibility:
//!   * accept the WS upgrade,
//!   * own the per-connection `WsSession`,
//!   * forward parsed client requests to `dispatcher::dispatch`,
//!   * push the dispatcher's response + any side-effect push frames,
//!   * emit periodic snapshots for active subscriptions,
//!   * send a heartbeat ping on idle,
//!   * enforce frame size + rate limits.
//!
//! This file does not contain any business logic — see `dispatcher.rs`
//! for that. The split is deliberate: every code path in `dispatcher`
//! is unit-tested without a real socket.

use super::config::PublicWsConfig;
use super::dispatcher::{dispatch, make_meta};
use super::lifecycle::{LifecycleChannel, LifecycleEvent};
use super::protocol::{Channel, ClientRequest, ServerResponse, WsError, WsErrorCode};
use super::session::WsSession;
use super::snapshots::{build_snapshot, build_snapshot_for_address};
use crate::api::AppState;
use crate::types::now_ms;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use futures::StreamExt;
use tracing::{info, warn};
use uuid::Uuid;

/// GET handler for `/ws`. When `public_ws_config.enabled == false`, the
/// upgrade is refused with a JSON 503 so the surface fails loudly
/// instead of silently dropping connections.
pub async fn public_ws_route(
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> axum::response::Response {
    if !state.public_ws_config.enabled {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "status": "error",
                "error": {
                    "code": "SOURCE_UNAVAILABLE",
                    "message": "public WebSocket is disabled on this server",
                }
            })),
        )
            .into_response();
    }
    let config = state.public_ws_config.clone();
    ws.max_message_size(config.max_frame_bytes)
        .max_frame_size(config.max_frame_bytes)
        .on_upgrade(move |socket| handle_socket(state, config, socket))
}

async fn handle_socket(state: AppState, config: PublicWsConfig, mut socket: WebSocket) {
    let connection_id = format!("conn_{}", Uuid::new_v4());
    info!(target: "deopt.public_ws", connection_id = %connection_id, "public_ws_open");
    let mut session = WsSession::new(connection_id.clone(), now_ms());
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_millis(
        config.heartbeat_interval_ms.max(1_000),
    ));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut snapshot_tick = tokio::time::interval(std::time::Duration::from_millis(
        config.snapshot_interval_ms.max(1_000),
    ));
    snapshot_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Skip first tick (fires immediately by default) so we don't
    // double-snapshot right after the initial subscribe.
    snapshot_tick.tick().await;
    heartbeat.tick().await;

    // ORDER-LIFECYCLE-OBSERVABILITY-V1 — per-session subscriber for
    // the process-wide LifecycleEvent broadcast. Filter happens
    // INSIDE the select arm: only events whose `event.account` matches
    // `session.account` AND whose channel has an active subscription
    // are forwarded to the client.
    let mut lifecycle_rx = state.lifecycle_events.subscribe();

    loop {
        tokio::select! {
            biased;
            incoming = socket.next() => {
                match incoming {
                    Some(Ok(Message::Text(raw))) => {
                        if raw.len() > config.max_frame_bytes {
                            let _ = send_unsolicited_error(
                                &mut socket,
                                &state,
                                WsErrorCode::FrameTooLarge,
                                "frame exceeds max_frame_bytes",
                            )
                            .await;
                            continue;
                        }
                        let now = now_ms();
                        session.record_message(now);
                        if session.is_rate_limited(config.client_rate_limit_per_sec) {
                            let _ = send_unsolicited_error(
                                &mut socket,
                                &state,
                                WsErrorCode::RateLimited,
                                "client rate limit exceeded; slow down",
                            )
                            .await;
                            continue;
                        }
                        session.prune_expired_challenges(now);
                        let req = match serde_json::from_str::<ClientRequest>(&raw) {
                            Ok(r) => r,
                            Err(e) => {
                                let _ = send_unsolicited_error(
                                    &mut socket,
                                    &state,
                                    WsErrorCode::InvalidRequest,
                                    format!("invalid JSON-RPC frame: {e}"),
                                )
                                .await;
                                continue;
                            }
                        };
                        let outcome = dispatch(&state, &config, &mut session, req).await;
                        if let Err(e) = send_response(&mut socket, &outcome.response).await {
                            warn!(target: "deopt.public_ws", connection_id = %session.connection_id, error = %e, "send_response failed");
                            break;
                        }
                        for push in outcome.pushes {
                            if let Err(e) = send_text(&mut socket, &push).await {
                                warn!(target: "deopt.public_ws", connection_id = %session.connection_id, error = %e, "send_push failed");
                                break;
                            }
                        }
                    }
                    Some(Ok(Message::Binary(_))) => {
                        let _ = send_unsolicited_error(
                            &mut socket,
                            &state,
                            WsErrorCode::InvalidRequest,
                            "binary frames are not accepted; send JSON text frames",
                        )
                        .await;
                    }
                    Some(Ok(Message::Ping(p))) => {
                        let _ = socket.send(Message::Pong(p)).await;
                    }
                    Some(Ok(Message::Pong(_))) => {
                        // Browsers don't expose Ping/Pong to JS so this
                        // path mostly handles WS clients that do. We
                        // just refresh the liveness counter.
                        session.last_message_at_ms = now_ms();
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        break;
                    }
                    Some(Err(e)) => {
                        warn!(target: "deopt.public_ws", connection_id = %session.connection_id, error = %e, "ws_recv_error");
                        break;
                    }
                }
            }
            _ = heartbeat.tick() => {
                if socket.send(Message::Ping(Vec::new())).await.is_err() {
                    break;
                }
            }
            _ = snapshot_tick.tick() => {
                if let Err(e) = emit_periodic_snapshots(&mut socket, &state, &mut session).await {
                    warn!(target: "deopt.public_ws", connection_id = %session.connection_id, error = %e, "periodic_snapshot_failed");
                    break;
                }
            }
            ev = lifecycle_rx.recv() => {
                match ev {
                    Ok(event) => {
                        if let Err(e) = forward_lifecycle_event(&mut socket, &state, &mut session, event).await {
                            warn!(target: "deopt.public_ws", connection_id = %session.connection_id, error = %e, "lifecycle_forward_failed");
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        // The client's lifecycle stream fell behind.
                        // Frontend MUST resync via REST snapshot — the
                        // periodic snapshot tick + the REST endpoints
                        // are the canonical recovery surfaces.
                        warn!(
                            target: "deopt.public_ws",
                            connection_id = %session.connection_id,
                            skipped = skipped,
                            "lifecycle_lagged"
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        // The broadcast channel was dropped — backend is
                        // shutting down. Close the socket cleanly.
                        break;
                    }
                }
            }
        }
    }
    info!(target: "deopt.public_ws", connection_id = %connection_id, "public_ws_close");
}

/// Forward one `LifecycleEvent` to the socket IF (a) the session is
/// authenticated to the event's account AND (b) the corresponding
/// channel has an active subscription on this session. Otherwise the
/// event is silently dropped — no other account's data may ever leak
/// out of a private session.
async fn forward_lifecycle_event(
    socket: &mut WebSocket,
    state: &AppState,
    session: &mut WsSession,
    event: LifecycleEvent,
) -> Result<(), String> {
    // Privacy gate 1: session must be authenticated to the event's account.
    let Some(session_addr) = session.account.as_ref() else {
        return Ok(());
    };
    if !session_addr.0.eq_ignore_ascii_case(&event.account.0) {
        return Ok(());
    }
    // Map the lifecycle channel to the protocol channel.
    let proto_channel = match event.channel {
        LifecycleChannel::AccountOrders => Channel::AccountOrders,
        LifecycleChannel::AccountFills => Channel::AccountFills,
        LifecycleChannel::AccountConditionalOrders => Channel::AccountConditionalOrders,
    };
    // Privacy gate 2: session must have an active subscription for that channel.
    let Some(sub_id) = session
        .subscriptions
        .iter()
        .find(|(_, s)| s.channel == proto_channel)
        .map(|(id, _)| id.clone())
    else {
        return Ok(());
    };
    let seq = if let Some(s) = session.subscriptions.get_mut(&sub_id) {
        let v = s.next_seq;
        s.next_seq = v.saturating_add(1);
        v
    } else {
        return Ok(());
    };
    let push = super::protocol::ServerNotification {
        jsonrpc: "2.0",
        method: "subscription",
        params: super::protocol::SubscriptionPushParams {
            subscription_id: sub_id,
            channel: proto_channel.as_str(),
            seq,
            event_id: format!("evt_{}", Uuid::new_v4()),
            source: "backend",
            chain_id: state.chain_id,
            generated_at_ms: now_ms(),
            instrument_id: None,
            address: Some(session_addr.0.clone()),
            tx_hash: None,
            data: serde_json::json!({
                "type": "lifecycle_delta",
                "emitted_at_ms": event.emitted_at_ms,
                "payload": event.payload,
            }),
        },
    };
    send_text(socket, &push).await
}

async fn send_response(socket: &mut WebSocket, response: &ServerResponse) -> Result<(), String> {
    send_text(socket, response).await
}

async fn send_text<T: serde::Serialize>(socket: &mut WebSocket, value: &T) -> Result<(), String> {
    let text = serde_json::to_string(value).map_err(|e| format!("serialize: {e}"))?;
    socket
        .send(Message::Text(text))
        .await
        .map_err(|e| format!("send: {e}"))
}

async fn send_unsolicited_error(
    socket: &mut WebSocket,
    state: &AppState,
    code: WsErrorCode,
    message: impl Into<String>,
) -> Result<(), String> {
    let resp = ServerResponse::err(None, WsError::new(code, message), make_meta(state));
    send_text(socket, &resp).await
}

async fn emit_periodic_snapshots(
    socket: &mut WebSocket,
    state: &AppState,
    session: &mut WsSession,
) -> Result<(), String> {
    if session.subscriptions.is_empty() {
        return Ok(());
    }
    // Snapshot every subscribed channel once per tick. Snapshots are
    // CHEAP — they read from in-memory stores via the existing handler
    // path — so this is safe at the configured interval default
    // (5 seconds).
    let channels: Vec<(String, super::protocol::Channel)> = session
        .subscriptions
        .values()
        .map(|s| (s.subscription_id.clone(), s.channel))
        .collect();
    for (sub_id, channel) in channels {
        let result = if channel.requires_auth() {
            match session.account.as_ref() {
                Some(addr) => build_snapshot_for_address(state, channel, &addr.0).await,
                None => continue,
            }
        } else {
            build_snapshot(state, channel).await
        };
        let data = match result {
            Ok(v) => v,
            Err(_) => continue,
        };
        let seq = if let Some(s) = session.subscriptions.get_mut(&sub_id) {
            let v = s.next_seq;
            s.next_seq = v.saturating_add(1);
            v
        } else {
            continue;
        };
        let push = super::protocol::ServerNotification {
            jsonrpc: "2.0",
            method: "subscription",
            params: super::protocol::SubscriptionPushParams {
                subscription_id: sub_id,
                channel: channel.as_str(),
                seq,
                event_id: format!("evt_{}", Uuid::new_v4()),
                source: "backend",
                chain_id: state.chain_id,
                generated_at_ms: now_ms(),
                instrument_id: None,
                address: session.account.as_ref().map(|a| a.0.clone()),
                tx_hash: None,
                data,
            },
        };
        send_text(socket, &push).await?;
    }
    Ok(())
}
