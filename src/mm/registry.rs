use super::protocol::ServerMessage;
use super::session::{MmSession, PublicSessionSnapshot};
use crate::error::{BackendError, Result};
use crate::types::AccountId;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone, Debug)]
pub struct RegisteredMmSession {
    pub snapshot: PublicSessionSnapshot,
    sender: UnboundedSender<ServerMessage>,
}

impl RegisteredMmSession {
    pub fn account(&self) -> Option<&AccountId> {
        self.snapshot.account.as_ref()
    }
}

#[derive(Clone, Debug, Default)]
pub struct MmSessionRegistry {
    sessions: Arc<Mutex<HashMap<String, RegisteredMmSession>>>,
}

impl MmSessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &self,
        session: &MmSession,
        sender: UnboundedSender<ServerMessage>,
    ) -> Result<()> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| BackendError::Config("MM session registry lock poisoned".to_string()))?;
        sessions.insert(
            session.session_id.clone(),
            RegisteredMmSession {
                snapshot: session.public_snapshot(),
                sender,
            },
        );
        Ok(())
    }

    pub fn update(&self, session: &MmSession) -> Result<()> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| BackendError::Config("MM session registry lock poisoned".to_string()))?;
        if let Some(registered) = sessions.get_mut(&session.session_id) {
            registered.snapshot = session.public_snapshot();
        }
        Ok(())
    }

    pub fn unregister(&self, session_id: &str) -> Result<()> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| BackendError::Config("MM session registry lock poisoned".to_string()))?;
        sessions.remove(session_id);
        Ok(())
    }

    pub fn list_active(&self) -> Result<Vec<PublicSessionSnapshot>> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| BackendError::Config("MM session registry lock poisoned".to_string()))?;
        let mut snapshots = sessions
            .values()
            .map(|session| session.snapshot.clone())
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| left.session_id.cmp(&right.session_id));
        Ok(snapshots)
    }

    pub fn get(&self, session_id: &str) -> Result<Option<PublicSessionSnapshot>> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| BackendError::Config("MM session registry lock poisoned".to_string()))?;
        Ok(sessions
            .get(session_id)
            .map(|session| session.snapshot.clone()))
    }

    pub fn list_by_account(&self, account: &AccountId) -> Result<Vec<PublicSessionSnapshot>> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| BackendError::Config("MM session registry lock poisoned".to_string()))?;
        let mut snapshots = sessions
            .values()
            .filter(|session| session.account() == Some(account))
            .map(|session| session.snapshot.clone())
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| left.session_id.cmp(&right.session_id));
        Ok(snapshots)
    }

    pub fn send_to_session(&self, session_id: &str, message: ServerMessage) -> Result<()> {
        let sender = {
            let sessions = self.sessions.lock().map_err(|_| {
                BackendError::Config("MM session registry lock poisoned".to_string())
            })?;
            sessions
                .get(session_id)
                .map(|session| session.sender.clone())
                .ok_or_else(|| {
                    BackendError::InvalidRfqQuoteState(format!(
                        "MM session {session_id} is not connected"
                    ))
                })?
        };
        sender.send(message).map_err(|_| {
            BackendError::InvalidRfqQuoteState(format!(
                "MM session {session_id} notification channel is closed"
            ))
        })
    }

    pub fn broadcast(&self, message: ServerMessage) -> Result<usize> {
        let senders = {
            let sessions = self.sessions.lock().map_err(|_| {
                BackendError::Config("MM session registry lock poisoned".to_string())
            })?;
            sessions
                .values()
                .map(|session| session.sender.clone())
                .collect::<Vec<_>>()
        };
        let mut sent = 0;
        for sender in senders {
            if sender.send(message.clone()).is_ok() {
                sent += 1;
            }
        }
        Ok(sent)
    }
}
