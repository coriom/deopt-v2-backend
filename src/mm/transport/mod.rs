use super::protocol::ServerMessage;
use super::service::MmGatewayService;

pub mod webtransport;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MmTransportError {
    pub message: String,
}

impl MmTransportError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

pub trait MmTransportSession {
    fn session_id(&self) -> &str;
    fn send(&mut self, message: ServerMessage) -> Result<(), MmTransportError>;
    fn close(&mut self, reason: &str) -> Result<(), MmTransportError>;
}

pub trait MmTransportAdapter {
    fn start(&mut self, service: MmGatewayService) -> Result<(), MmTransportError>;
}
