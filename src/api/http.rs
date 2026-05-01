use crate::engine::EngineState;
use crate::signing::{NonceStore, SignatureVerificationMode};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<Mutex<EngineState>>,
    pub nonces: Arc<Mutex<NonceStore>>,
    pub signature_verification_mode: SignatureVerificationMode,
}

impl AppState {
    pub fn new(engine: EngineState) -> Self {
        Self::with_signature_mode(engine, SignatureVerificationMode::Disabled)
    }

    pub fn with_signature_mode(
        engine: EngineState,
        signature_verification_mode: SignatureVerificationMode,
    ) -> Self {
        Self {
            engine: Arc::new(Mutex::new(engine)),
            nonces: Arc::new(Mutex::new(NonceStore::new())),
            signature_verification_mode,
        }
    }
}
