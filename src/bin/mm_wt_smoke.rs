use deopt_v2_backend::mm::transport::webtransport::{
    read_frame, write_json_frame, MM_GATEWAY_MAX_FRAME_BYTES,
};
use serde_json::json;
use std::env;
use wtransport::tls::Certificate;
use wtransport::{ClientConfig, Endpoint};

#[tokio::main]
async fn main() -> deopt_v2_backend::Result<()> {
    let url = env::var("MM_WT_URL").unwrap_or_else(|_| "https://127.0.0.1:8443/mm".to_string());
    let cert_path = env::var("MM_WT_CERT_PATH")
        .unwrap_or_else(|_| "/tmp/deopt-mm-gateway/cert.pem".to_string());
    let cert = Certificate::load_pemfile(&cert_path)
        .await
        .map_err(|error| {
            deopt_v2_backend::BackendError::Config(format!(
                "failed to load MM_WT_CERT_PATH {cert_path}: {error}"
            ))
        })?;
    let config = ClientConfig::builder()
        .with_bind_default()
        .with_server_certificate_hashes([cert.hash()])
        .keep_alive_interval(Some(std::time::Duration::from_secs(3)))
        .build();
    let endpoint = Endpoint::client(config)
        .map_err(|error| deopt_v2_backend::BackendError::Config(error.to_string()))?;
    let connection = endpoint
        .connect(url.as_str())
        .await
        .map_err(|error| deopt_v2_backend::BackendError::Config(error.to_string()))?;

    let heartbeat = send_request(
        &connection,
        json!({
            "type": "heartbeat",
            "request_id": "smoke-heartbeat-1",
            "payload": {}
        }),
    )
    .await?;
    println!("{}", serde_json::to_string(&heartbeat).unwrap());

    let session = send_request(
        &connection,
        json!({
            "type": "get_session",
            "request_id": "smoke-session-1",
            "payload": {}
        }),
    )
    .await?;
    println!("{}", serde_json::to_string(&session).unwrap());

    connection.close(0_u32.into(), b"smoke complete");
    endpoint.wait_idle().await;
    Ok(())
}

async fn send_request(
    connection: &wtransport::Connection,
    request: serde_json::Value,
) -> deopt_v2_backend::Result<serde_json::Value> {
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .map_err(|error| deopt_v2_backend::BackendError::Config(error.to_string()))?
        .await
        .map_err(|error| deopt_v2_backend::BackendError::Config(error.to_string()))?;
    write_json_frame(&mut send, &request, MM_GATEWAY_MAX_FRAME_BYTES)
        .await
        .map_err(|error| deopt_v2_backend::BackendError::Config(error.to_string()))?;
    send.finish()
        .await
        .map_err(|error| deopt_v2_backend::BackendError::Config(error.to_string()))?;
    let response = read_frame(&mut recv, MM_GATEWAY_MAX_FRAME_BYTES)
        .await
        .map_err(|error| deopt_v2_backend::BackendError::Config(error.to_string()))?
        .ok_or_else(|| {
            deopt_v2_backend::BackendError::Config(
                "MM gateway closed stream without a response frame".to_string(),
            )
        })?;
    serde_json::from_slice(&response)
        .map_err(|error| deopt_v2_backend::BackendError::Config(error.to_string()))
}
