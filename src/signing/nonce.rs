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
}
