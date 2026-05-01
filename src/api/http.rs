use crate::engine::EngineState;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<Mutex<EngineState>>,
}

impl AppState {
    pub fn new(engine: EngineState) -> Self {
        Self {
            engine: Arc::new(Mutex::new(engine)),
        }
    }
}
