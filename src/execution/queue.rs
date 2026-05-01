use super::ExecutionIntent;

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
}
