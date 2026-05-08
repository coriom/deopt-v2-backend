use crate::api::AppState;
use crate::error::{BackendError, Result};
use crate::mm::protocol::{ClientMessage, ErrorCode, ServerMessage};
use crate::mm::rate_limit::{MmGatewayConfig, MmGatewayTransport};
use crate::mm::service::MmGatewayService;
use crate::mm::session::MmSession;
use crate::types::now_ms;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tracing::{info, warn};
use wtransport::endpoint::IncomingSession;
use wtransport::stream::{RecvStream, SendStream};
use wtransport::{Connection, Endpoint, Identity, ServerConfig};

pub const MM_GATEWAY_MAX_FRAME_BYTES: usize = 1_048_576;

#[derive(Debug, Error)]
pub enum MmFrameError {
    #[error("frame I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("frame is oversized: {len} bytes exceeds max {max} bytes")]
    Oversized { len: usize, max: usize },
    #[error("stream ended before frame was complete")]
    UnexpectedEof,
    #[error("frame serialization failed: {0}")]
    Serialize(String),
    #[error("frame JSON decode failed: {0}")]
    Json(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MmGatewayStartup {
    Disabled,
    Enabled {
        bind_addr: SocketAddr,
        cert_path: PathBuf,
        key_path: PathBuf,
    },
}

pub fn validate_webtransport_startup(config: &MmGatewayConfig) -> Result<MmGatewayStartup> {
    if !config.enabled {
        return Ok(MmGatewayStartup::Disabled);
    }

    if config.transport != MmGatewayTransport::WebTransport {
        return Err(BackendError::Config(
            "MM_GATEWAY_TRANSPORT must be webtransport in V1B".to_string(),
        ));
    }

    let cert_path = config.cert_path.as_ref().ok_or_else(|| {
        BackendError::Config(
            "MM_GATEWAY_CERT_PATH is required when MM_GATEWAY_ENABLED=true".to_string(),
        )
    })?;
    let key_path = config.key_path.as_ref().ok_or_else(|| {
        BackendError::Config(
            "MM_GATEWAY_KEY_PATH is required when MM_GATEWAY_ENABLED=true".to_string(),
        )
    })?;
    let bind_addr = format!("{}:{}", config.host, config.port)
        .parse()
        .map_err(|error| {
            BackendError::Config(format!("invalid MM gateway bind address: {error}"))
        })?;

    Ok(MmGatewayStartup::Enabled {
        bind_addr,
        cert_path: PathBuf::from(cert_path),
        key_path: PathBuf::from(key_path),
    })
}

pub async fn spawn_webtransport_gateway(config: MmGatewayConfig, state: AppState) -> Result<()> {
    let MmGatewayStartup::Enabled {
        bind_addr,
        cert_path,
        key_path,
    } = validate_webtransport_startup(&config)?
    else {
        info!("MM WebTransport gateway disabled");
        return Ok(());
    };

    let identity = Identity::load_pemfiles(&cert_path, &key_path)
        .await
        .map_err(|error| {
            BackendError::Config(format!(
                "failed to load MM gateway TLS cert/key from {} and {}: {error}",
                cert_path.display(),
                key_path.display()
            ))
        })?;
    let server_config = ServerConfig::builder()
        .with_bind_address(bind_addr)
        .with_identity(identity)
        .keep_alive_interval(Some(Duration::from_secs(3)))
        .build();
    let endpoint = Endpoint::server(server_config).map_err(|error| {
        BackendError::Config(format!("failed to bind MM gateway UDP listener: {error}"))
    })?;
    let local_addr = endpoint.local_addr().map_err(|error| {
        BackendError::Config(format!("failed to read MM gateway local address: {error}"))
    })?;
    let service = MmGatewayService::new(config.clone(), state);

    tokio::spawn(async move {
        accept_loop(endpoint, service, config).await;
    });

    info!(%local_addr, "MM WebTransport gateway listening");
    Ok(())
}

async fn accept_loop(
    endpoint: Endpoint<wtransport::endpoint::endpoint_side::Server>,
    service: MmGatewayService,
    config: MmGatewayConfig,
) {
    loop {
        let incoming = endpoint.accept().await;
        if endpoint.open_connections() >= config.max_sessions {
            warn!(
                remote_addr = %incoming.remote_address(),
                max_sessions = config.max_sessions,
                "refusing MM WebTransport session because max sessions is reached"
            );
            incoming.refuse();
            continue;
        }

        let service = service.clone();
        let config = config.clone();
        tokio::spawn(async move {
            if let Err(error) = handle_incoming_session(incoming, service, config).await {
                warn!(error = %error, "MM WebTransport session ended with error");
            }
        });
    }
}

async fn handle_incoming_session(
    incoming: IncomingSession,
    service: MmGatewayService,
    config: MmGatewayConfig,
) -> std::result::Result<(), String> {
    let remote_addr = incoming.remote_address();
    let request = incoming.await.map_err(|error| error.to_string())?;
    info!(
        %remote_addr,
        authority = request.authority(),
        path = request.path(),
        "accepted MM WebTransport session request"
    );
    let connection = request.accept().await.map_err(|error| error.to_string())?;
    let connection_id = format!("wt-{}", connection.stable_id());
    let mut session = MmSession::new(
        connection_id,
        now_ms(),
        config.auth_mode,
        config.cancel_on_disconnect,
    );
    let (outbound_sender, mut outbound_receiver) = mpsc::unbounded_channel();
    service
        .register_session(&session, outbound_sender)
        .map_err(|error| error.to_string())?;

    loop {
        tokio::select! {
            stream = connection.accept_bi() => {
                match stream {
                    Ok((send, recv)) => {
                        if let Err(error) = handle_bi_stream(send, recv, &service, &mut session).await {
                            warn!(
                                session_id = %session.session_id,
                                error = %error,
                                "MM WebTransport stream handling failed"
                            );
                        }
                    }
                    Err(error) => {
                        warn!(session_id = %session.session_id, error = %error, "MM WebTransport accept_bi failed");
                        break;
                    }
                }
            }
            outbound = outbound_receiver.recv() => {
                let Some(message) = outbound else {
                    break;
                };
                if let Err(error) = send_server_message(&connection, &message).await {
                    warn!(
                        session_id = %session.session_id,
                        error = %error,
                        "MM WebTransport server-initiated message failed"
                    );
                }
            }
            closed = connection.closed() => {
                info!(session_id = %session.session_id, error = %closed, "MM WebTransport session closed");
                break;
            }
        }
    }

    let cancelled = service.cancel_on_disconnect(&mut session).await;
    if cancelled > 0 {
        info!(
            session_id = %session.session_id,
            cancelled,
            "completed MM cancel-on-disconnect"
        );
    }
    if let Err(error) = service.unregister_session(&session.session_id) {
        warn!(
            session_id = %session.session_id,
            error = %error,
            "failed to unregister MM session"
        );
    }

    Ok(())
}

async fn handle_bi_stream(
    mut send: SendStream,
    mut recv: RecvStream,
    service: &MmGatewayService,
    session: &mut MmSession,
) -> std::result::Result<(), String> {
    let response = match read_frame(&mut recv, MM_GATEWAY_MAX_FRAME_BYTES).await {
        Ok(Some(frame)) => match decode_client_message(&frame) {
            Ok(message) => {
                let response = service.handle_message(session, message, now_ms()).await;
                if let Err(error) = service.update_session(session) {
                    warn!(
                        session_id = %session.session_id,
                        error = %error,
                        "failed to update MM session registry"
                    );
                }
                response
            }
            Err(response) => *response,
        },
        Ok(None) => return Ok(()),
        Err(MmFrameError::Oversized { .. }) => {
            ServerMessage::error("", ErrorCode::BadRequest, "frame exceeds maximum size")
        }
        Err(error) => {
            return Err(error.to_string());
        }
    };

    write_json_frame(&mut send, &response, MM_GATEWAY_MAX_FRAME_BYTES)
        .await
        .map_err(|error| error.to_string())?;
    send.finish().await.map_err(|error| error.to_string())?;
    Ok(())
}

async fn send_server_message(
    connection: &Connection,
    message: &ServerMessage,
) -> std::result::Result<(), String> {
    let mut send = connection
        .open_uni()
        .await
        .map_err(|error| error.to_string())?
        .await
        .map_err(|error| error.to_string())?;
    write_json_frame(&mut send, message, MM_GATEWAY_MAX_FRAME_BYTES)
        .await
        .map_err(|error| error.to_string())?;
    send.finish().await.map_err(|error| error.to_string())
}

pub fn encode_frame(
    payload: &[u8],
    max_frame_bytes: usize,
) -> std::result::Result<Vec<u8>, MmFrameError> {
    if payload.len() > max_frame_bytes {
        return Err(MmFrameError::Oversized {
            len: payload.len(),
            max: max_frame_bytes,
        });
    }
    let len = u32::try_from(payload.len()).map_err(|_| MmFrameError::Oversized {
        len: payload.len(),
        max: u32::MAX as usize,
    })?;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

pub fn encode_json_frame<T: Serialize>(
    value: &T,
    max_frame_bytes: usize,
) -> std::result::Result<Vec<u8>, MmFrameError> {
    let payload =
        serde_json::to_vec(value).map_err(|error| MmFrameError::Serialize(error.to_string()))?;
    encode_frame(&payload, max_frame_bytes)
}

pub fn decode_frame_payload(
    frame: &[u8],
    max_frame_bytes: usize,
) -> std::result::Result<&[u8], MmFrameError> {
    if frame.len() < 4 {
        return Err(MmFrameError::UnexpectedEof);
    }
    let len = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
    if len > max_frame_bytes {
        return Err(MmFrameError::Oversized {
            len,
            max: max_frame_bytes,
        });
    }
    if frame.len() < 4 + len {
        return Err(MmFrameError::UnexpectedEof);
    }
    Ok(&frame[4..4 + len])
}

pub fn decode_json_frame<T: DeserializeOwned>(
    frame: &[u8],
    max_frame_bytes: usize,
) -> std::result::Result<T, MmFrameError> {
    let payload = decode_frame_payload(frame, max_frame_bytes)?;
    serde_json::from_slice(payload).map_err(|error| MmFrameError::Json(error.to_string()))
}

pub async fn read_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
    max_frame_bytes: usize,
) -> std::result::Result<Option<Vec<u8>>, MmFrameError> {
    let mut header = [0_u8; 4];
    match read_exact_or_eof(reader, &mut header).await? {
        ReadOutcome::EofBeforeAnyByte => return Ok(None),
        ReadOutcome::Complete => {}
    }

    let len = u32::from_be_bytes(header) as usize;
    if len > max_frame_bytes {
        return Err(MmFrameError::Oversized {
            len,
            max: max_frame_bytes,
        });
    }
    let mut payload = vec![0_u8; len];
    match read_exact_or_eof(reader, &mut payload).await? {
        ReadOutcome::Complete => Ok(Some(payload)),
        ReadOutcome::EofBeforeAnyByte => Err(MmFrameError::UnexpectedEof),
    }
}

pub async fn write_json_frame<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    value: &T,
    max_frame_bytes: usize,
) -> std::result::Result<(), MmFrameError> {
    let frame = encode_json_frame(value, max_frame_bytes)?;
    writer.write_all(&frame).await?;
    writer.flush().await?;
    Ok(())
}

fn decode_client_message(frame: &[u8]) -> std::result::Result<ClientMessage, Box<ServerMessage>> {
    let value = serde_json::from_slice::<serde_json::Value>(frame).map_err(|_| {
        Box::new(ServerMessage::error(
            "",
            ErrorCode::BadRequest,
            "invalid JSON client message",
        ))
    })?;
    let request_id = value
        .get("request_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    serde_json::from_value::<ClientMessage>(value).map_err(|error| {
        let code = if error.to_string().contains("unknown message type") {
            ErrorCode::UnknownMessageType
        } else {
            ErrorCode::BadRequest
        };
        Box::new(ServerMessage::error(
            request_id,
            code,
            "invalid MM gateway client message",
        ))
    })
}

enum ReadOutcome {
    Complete,
    EofBeforeAnyByte,
}

async fn read_exact_or_eof<R: AsyncRead + Unpin>(
    reader: &mut R,
    mut buffer: &mut [u8],
) -> std::result::Result<ReadOutcome, MmFrameError> {
    let mut read_any = false;
    while !buffer.is_empty() {
        match reader.read(buffer).await {
            Ok(0) if read_any => return Err(MmFrameError::UnexpectedEof),
            Ok(0) => return Ok(ReadOutcome::EofBeforeAnyByte),
            Ok(bytes_read) => {
                read_any = true;
                let tmp = buffer;
                buffer = &mut tmp[bytes_read..];
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(MmFrameError::Io(error)),
        }
    }
    Ok(ReadOutcome::Complete)
}
