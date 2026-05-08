use crate::error::{BackendError, Result};
use crate::types::AccountId;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Default)]
pub struct NonceStore {
    used: HashMap<AccountId, HashSet<u64>>,
}

impl NonceStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reserve(&mut self, account: &AccountId, nonce: u64) -> Result<()> {
        if nonce == 0 {
            return Err(BackendError::InvalidNonce);
        }

        let account_nonces = self.used.entry(account.clone()).or_default();
        if !account_nonces.insert(nonce) {
            return Err(BackendError::NonceAlreadyUsed);
        }
        Ok(())
    }

    pub fn reserve_next(&mut self, account: &AccountId) -> Result<u64> {
        let account_nonces = self.used.entry(account.clone()).or_default();
        let mut nonce = 1u64;
        while account_nonces.contains(&nonce) {
            nonce = nonce.checked_add(1).ok_or(BackendError::InvalidNonce)?;
        }
        account_nonces.insert(nonce);
        Ok(nonce)
    }
}
