use crate::types::{AccountId, TimestampMs};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::str::FromStr;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    Disabled,
    WalletChallenge,
}

impl FromStr for AuthMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "disabled" => Ok(Self::Disabled),
            "wallet_challenge" => Ok(Self::WalletChallenge),
            other => Err(format!("unsupported MM_GATEWAY_AUTH_MODE: {other}")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MmSession {
    pub session_id: String,
    pub connection_id: String,
    pub account: Option<AccountId>,
    pub authenticated: bool,
    pub auth_mode: AuthMode,
    pub challenge_account: Option<AccountId>,
    pub challenge_nonce: Option<String>,
    pub challenge_issued_at_ms: Option<TimestampMs>,
    pub challenge_expires_at_ms: Option<TimestampMs>,
    pub connected_at_ms: TimestampMs,
    pub last_heartbeat_at_ms: TimestampMs,
    pub cancel_on_disconnect: bool,
    pub open_client_order_ids: BTreeSet<String>,
    pub quote_client_order_ids: BTreeSet<String>,
    pub messages_in_current_window: u32,
    pub window_started_at_ms: TimestampMs,
    pub in_flight_requests: u32,
}

impl MmSession {
    pub fn new(
        connection_id: impl Into<String>,
        now_ms: TimestampMs,
        auth_mode: AuthMode,
        cancel_on_disconnect: bool,
    ) -> Self {
        Self::with_ids(
            Uuid::new_v4().to_string(),
            connection_id,
            now_ms,
            auth_mode,
            cancel_on_disconnect,
        )
    }

    pub fn with_ids(
        session_id: impl Into<String>,
        connection_id: impl Into<String>,
        now_ms: TimestampMs,
        auth_mode: AuthMode,
        cancel_on_disconnect: bool,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            connection_id: connection_id.into(),
            account: None,
            authenticated: false,
            auth_mode,
            challenge_account: None,
            challenge_nonce: None,
            challenge_issued_at_ms: None,
            challenge_expires_at_ms: None,
            connected_at_ms: now_ms,
            last_heartbeat_at_ms: now_ms,
            cancel_on_disconnect,
            open_client_order_ids: BTreeSet::new(),
            quote_client_order_ids: BTreeSet::new(),
            messages_in_current_window: 0,
            window_started_at_ms: now_ms,
            in_flight_requests: 0,
        }
    }

    pub fn bind_account(&mut self, account: AccountId) {
        self.account = Some(account);
        self.authenticated = true;
        self.clear_challenge();
    }

    pub fn set_challenge(
        &mut self,
        account: AccountId,
        nonce: String,
        issued_at_ms: TimestampMs,
        expires_at_ms: TimestampMs,
    ) {
        self.challenge_account = Some(account);
        self.challenge_nonce = Some(nonce);
        self.challenge_issued_at_ms = Some(issued_at_ms);
        self.challenge_expires_at_ms = Some(expires_at_ms);
    }

    pub fn clear_challenge(&mut self) {
        self.challenge_account = None;
        self.challenge_nonce = None;
        self.challenge_issued_at_ms = None;
        self.challenge_expires_at_ms = None;
    }

    pub fn clear_expired_challenge(&mut self, now_ms: TimestampMs) {
        if self
            .challenge_expires_at_ms
            .is_some_and(|expires_at_ms| now_ms > expires_at_ms)
        {
            self.clear_challenge();
        }
    }

    pub fn challenge_active(&self) -> bool {
        !self.authenticated
            && self.challenge_nonce.is_some()
            && self.challenge_expires_at_ms.is_some()
    }

    pub fn update_heartbeat(&mut self, now_ms: TimestampMs) {
        self.last_heartbeat_at_ms = now_ms;
    }

    pub fn heartbeat_timed_out(&self, now_ms: TimestampMs, timeout_ms: u64) -> bool {
        now_ms.saturating_sub(self.last_heartbeat_at_ms) > timeout_ms as i64
    }

    pub fn register_open_client_order_id(&mut self, client_order_id: impl Into<String>) -> bool {
        self.open_client_order_ids.insert(client_order_id.into())
    }

    pub fn unregister_open_client_order_id(&mut self, client_order_id: &str) -> bool {
        self.open_client_order_ids.remove(client_order_id)
    }

    pub fn clear_open_client_order_ids(&mut self) -> Vec<String> {
        let ids = self.open_client_order_ids.iter().cloned().collect();
        self.open_client_order_ids.clear();
        ids
    }

    pub fn register_quote_client_order_id(&mut self, client_order_id: impl Into<String>) -> bool {
        self.quote_client_order_ids.insert(client_order_id.into())
    }

    pub fn unregister_quote_client_order_id(&mut self, client_order_id: &str) -> bool {
        self.quote_client_order_ids.remove(client_order_id)
    }

    pub fn plan_cancel_on_disconnect(&self) -> CancelOnDisconnectPlan {
        let client_order_ids = if self.cancel_on_disconnect {
            self.open_client_order_ids.iter().cloned().collect()
        } else {
            Vec::new()
        };
        CancelOnDisconnectPlan {
            session_id: self.session_id.clone(),
            account: self.account.clone(),
            client_order_ids,
        }
    }

    pub fn increment_in_flight(&mut self) {
        self.in_flight_requests = self.in_flight_requests.saturating_add(1);
    }

    pub fn decrement_in_flight(&mut self) {
        self.in_flight_requests = self.in_flight_requests.saturating_sub(1);
    }

    pub fn public_snapshot(&self) -> PublicSessionSnapshot {
        PublicSessionSnapshot {
            session_id: self.session_id.clone(),
            connection_id: self.connection_id.clone(),
            account: self.account.clone(),
            authenticated: self.authenticated,
            auth_mode: self.auth_mode,
            challenge_active: self.challenge_active(),
            challenge_expires_at_ms: self.challenge_expires_at_ms,
            connected_at_ms: self.connected_at_ms,
            last_heartbeat_at_ms: self.last_heartbeat_at_ms,
            cancel_on_disconnect: self.cancel_on_disconnect,
            open_client_order_ids: self.open_client_order_ids.iter().cloned().collect(),
            messages_in_current_window: self.messages_in_current_window,
            window_started_at_ms: self.window_started_at_ms,
            in_flight_requests: self.in_flight_requests,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CancelOnDisconnectPlan {
    pub session_id: String,
    pub account: Option<AccountId>,
    pub client_order_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PublicSessionSnapshot {
    pub session_id: String,
    pub connection_id: String,
    pub account: Option<AccountId>,
    pub authenticated: bool,
    pub auth_mode: AuthMode,
    pub challenge_active: bool,
    pub challenge_expires_at_ms: Option<TimestampMs>,
    pub connected_at_ms: TimestampMs,
    pub last_heartbeat_at_ms: TimestampMs,
    pub cancel_on_disconnect: bool,
    pub open_client_order_ids: Vec<String>,
    pub messages_in_current_window: u32,
    pub window_started_at_ms: TimestampMs,
    pub in_flight_requests: u32,
}
