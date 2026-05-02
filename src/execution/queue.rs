use super::{ExecutionIntent, ExecutionIntentStatus};
use uuid::Uuid;

#[derive(Clone, Debug, Default)]
pub struct ExecutionQueue {
    intents: Vec<ExecutionIntent>,
}

impl ExecutionQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, intent: ExecutionIntent) {
        self.intents.push(intent);
    }

    pub fn all(&self) -> Vec<ExecutionIntent> {
        self.intents.clone()
    }

    pub fn update_status(&mut self, intent_id: Uuid, status: ExecutionIntentStatus) -> bool {
        let Some(intent) = self
            .intents
            .iter_mut()
            .find(|intent| intent.intent_id == intent_id)
        else {
            return false;
        };
        intent.status = status;
        true
    }
}
