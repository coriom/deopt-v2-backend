use crate::api::AppState;
use crate::error::{BackendError, Result};
use crate::types::{AccountId, MarketId, TimestampMs};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MmPermissionsConfig {
    pub enabled: bool,
    pub require_persistence: bool,
}

impl MmPermissionsConfig {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            require_persistence: true,
        }
    }

    pub fn enabled_in_memory_for_tests() -> Self {
        Self {
            enabled: true,
            require_persistence: false,
        }
    }

    pub fn validate_startup(&self, persistence_enabled: bool) -> Result<()> {
        if self.enabled && self.require_persistence && !persistence_enabled {
            return Err(BackendError::Config(
                "MM permissions require persistence enabled when MM_PERMISSIONS_REQUIRE_PERSISTENCE=true"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MmAccountPermissions {
    pub mm_account: AccountId,
    pub enabled: bool,
    pub label: Option<String>,
    pub can_submit_perp_orders: bool,
    pub can_quote_perp_rfq: bool,
    pub can_quote_option_rfq: bool,
    pub can_submit_option_orders: bool,
    pub created_at_ms: TimestampMs,
    pub updated_at_ms: TimestampMs,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MmProductPermission {
    pub id: String,
    pub mm_account: AccountId,
    pub market_id: Option<MarketId>,
    pub option_series_id: Option<String>,
    pub enabled: bool,
    pub created_at_ms: TimestampMs,
    pub updated_at_ms: TimestampMs,
}

#[derive(Clone, Debug, Default)]
pub struct MmPermissionsStore {
    accounts: BTreeMap<String, MmAccountPermissions>,
    product_permissions: BTreeMap<String, MmProductPermission>,
}

impl MmPermissionsStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert_account(&mut self, account: MmAccountPermissions) {
        self.accounts
            .insert(account_key(&account.mm_account), account);
    }

    pub fn insert_product_permission(&mut self, permission: MmProductPermission) {
        self.product_permissions
            .insert(permission.id.clone(), permission);
    }

    pub fn get_account(&self, account: &AccountId) -> Option<MmAccountPermissions> {
        self.accounts.get(&account_key(account)).cloned()
    }

    pub fn list_accounts(&self) -> Vec<MmAccountPermissions> {
        self.accounts.values().cloned().collect()
    }

    pub fn product_permissions_for_account(&self, account: &AccountId) -> Vec<MmProductPermission> {
        let key = account_key(account);
        self.product_permissions
            .values()
            .filter(|permission| account_key(&permission.mm_account) == key)
            .cloned()
            .collect()
    }

    pub fn list_product_permissions(&self) -> Vec<MmProductPermission> {
        self.product_permissions.values().cloned().collect()
    }
}

pub async fn check_mm_enabled(state: &AppState, account: &AccountId) -> Result<()> {
    if !state.mm_permissions_config.enabled {
        return Ok(());
    }
    load_enabled_account(state, account).await.map(|_| ())
}

pub async fn check_can_quote_perp_rfq(
    state: &AppState,
    account: &AccountId,
    market_id: MarketId,
) -> Result<()> {
    if !state.mm_permissions_config.enabled {
        return Ok(());
    }
    let permissions = load_enabled_account(state, account).await?;
    if !permissions.can_quote_perp_rfq {
        return Err(permission_denied(
            account,
            "MM account lacks can_quote_perp_rfq",
        ));
    }
    let scopes = product_permissions_for_account(state, account).await?;
    if !market_scope_allows(&scopes, market_id) {
        return Err(permission_denied(
            account,
            format!("MM account is not allowed for market_id {market_id}"),
        ));
    }
    Ok(())
}

pub async fn check_can_quote_option_rfq(
    state: &AppState,
    account: &AccountId,
    option_series_id: &str,
) -> Result<()> {
    if !state.mm_permissions_config.enabled {
        return Ok(());
    }
    let permissions = load_enabled_account(state, account).await?;
    if !permissions.can_quote_option_rfq {
        return Err(permission_denied(
            account,
            "MM account lacks can_quote_option_rfq",
        ));
    }
    let scopes = product_permissions_for_account(state, account).await?;
    if !option_series_scope_allows(&scopes, option_series_id) {
        return Err(permission_denied(
            account,
            format!("MM account is not allowed for option_series_id {option_series_id}"),
        ));
    }
    Ok(())
}

pub async fn check_can_submit_perp_order(
    state: &AppState,
    account: &AccountId,
    market_id: MarketId,
) -> Result<()> {
    if !state.mm_permissions_config.enabled {
        return Ok(());
    }
    let permissions = load_enabled_account(state, account).await?;
    if !permissions.can_submit_perp_orders {
        return Err(permission_denied(
            account,
            "MM account lacks can_submit_perp_orders",
        ));
    }
    let scopes = product_permissions_for_account(state, account).await?;
    if !market_scope_allows(&scopes, market_id) {
        return Err(permission_denied(
            account,
            format!("MM account is not allowed for market_id {market_id}"),
        ));
    }
    Ok(())
}

pub async fn list_permission_accounts(state: &AppState) -> Result<Vec<MmAccountPermissions>> {
    if let Some(repository) = state.repository.clone() {
        return repository.list_mm_permission_accounts().await;
    }
    Ok(state
        .mm_permissions
        .lock()
        .map_err(|_| BackendError::Config("MM permissions store lock poisoned".to_string()))?
        .list_accounts())
}

pub async fn list_product_permissions(state: &AppState) -> Result<Vec<MmProductPermission>> {
    if let Some(repository) = state.repository.clone() {
        return repository.list_mm_product_permissions().await;
    }
    Ok(state
        .mm_permissions
        .lock()
        .map_err(|_| BackendError::Config("MM permissions store lock poisoned".to_string()))?
        .list_product_permissions())
}

async fn load_enabled_account(
    state: &AppState,
    account: &AccountId,
) -> Result<MmAccountPermissions> {
    let permissions = load_account(state, account)
        .await?
        .ok_or_else(|| permission_denied(account, "MM account is not permissioned"))?;
    if !permissions.enabled {
        return Err(permission_denied(account, "MM account is disabled"));
    }
    Ok(permissions)
}

async fn load_account(
    state: &AppState,
    account: &AccountId,
) -> Result<Option<MmAccountPermissions>> {
    if let Some(repository) = state.repository.clone() {
        return repository.get_mm_permission_account(account).await;
    }
    Ok(state
        .mm_permissions
        .lock()
        .map_err(|_| BackendError::Config("MM permissions store lock poisoned".to_string()))?
        .get_account(account))
}

async fn product_permissions_for_account(
    state: &AppState,
    account: &AccountId,
) -> Result<Vec<MmProductPermission>> {
    if let Some(repository) = state.repository.clone() {
        return repository
            .list_mm_product_permissions_for_account(account)
            .await;
    }
    Ok(state
        .mm_permissions
        .lock()
        .map_err(|_| BackendError::Config("MM permissions store lock poisoned".to_string()))?
        .product_permissions_for_account(account))
}

fn market_scope_allows(permissions: &[MmProductPermission], market_id: MarketId) -> bool {
    let mut market_scope_configured = false;
    for permission in permissions {
        if permission.option_series_id.is_some() {
            continue;
        }
        market_scope_configured = true;
        if permission.enabled && permission.market_id.is_none() {
            return true;
        }
        if permission.enabled && permission.market_id == Some(market_id) {
            return true;
        }
    }
    !market_scope_configured
}

fn option_series_scope_allows(permissions: &[MmProductPermission], option_series_id: &str) -> bool {
    let mut option_scope_configured = false;
    for permission in permissions {
        if permission.market_id.is_some() {
            continue;
        }
        option_scope_configured = true;
        if permission.enabled && permission.option_series_id.is_none() {
            return true;
        }
        if permission.enabled
            && permission
                .option_series_id
                .as_deref()
                .is_some_and(|allowed| allowed.eq_ignore_ascii_case(option_series_id))
        {
            return true;
        }
    }
    !option_scope_configured
}

fn permission_denied(account: &AccountId, reason: impl Into<String>) -> BackendError {
    BackendError::MmPermissionDenied(format!("{}: {}", account.0, reason.into()))
}

fn account_key(account: &AccountId) -> String {
    account.0.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> AccountId {
        AccountId::new("0x0000000000000000000000000000000000000001")
    }

    #[test]
    fn market_scope_allows_all_when_unconfigured() {
        assert!(market_scope_allows(&[], 1));
    }

    #[test]
    fn market_scope_requires_match_when_configured() {
        let permissions = vec![MmProductPermission {
            id: "scope-1".to_string(),
            mm_account: account(),
            market_id: Some(1),
            option_series_id: None,
            enabled: true,
            created_at_ms: 1,
            updated_at_ms: 1,
        }];

        assert!(market_scope_allows(&permissions, 1));
        assert!(!market_scope_allows(&permissions, 2));
    }

    #[test]
    fn option_scope_requires_match_when_configured() {
        let permissions = vec![MmProductPermission {
            id: "scope-1".to_string(),
            mm_account: account(),
            market_id: None,
            option_series_id: Some("0xabc".to_string()),
            enabled: true,
            created_at_ms: 1,
            updated_at_ms: 1,
        }];

        assert!(option_series_scope_allows(&permissions, "0xABC"));
        assert!(!option_series_scope_allows(&permissions, "0xdef"));
    }
}
