use deopt_v2_backend::mm::transport::webtransport::{
    read_frame, write_json_frame, MM_GATEWAY_MAX_FRAME_BYTES,
};
use serde_json::{json, Value};
use std::env;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{sleep, timeout, Duration};
use wtransport::tls::Certificate;
use wtransport::{ClientConfig, Endpoint};

const ACCOUNT: &str = "0x0000000000000000000000000000000000000001";
const RFQ_TAKER: &str = "0xc0A76c2A6c6b70C0B065A05E64417886416cc976";
const RFQ_MM_ACCOUNT: &str = "0xbAf0976a00a0DCc84Df5B15d927695c8b014B1c3";
const VALID_SIGNATURE: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[tokio::main]
async fn main() -> deopt_v2_backend::Result<()> {
    let scenario = env::args()
        .nth(1)
        .or_else(|| env::var("MM_WT_SCENARIO").ok())
        .unwrap_or_else(|| "basic".to_string());
    let url = env::var("MM_WT_URL").unwrap_or_else(|_| "https://127.0.0.1:8443/mm".to_string());
    let cert_path = env::var("MM_WT_CERT_PATH")
        .unwrap_or_else(|_| "/tmp/deopt-mm-gateway/cert.pem".to_string());
    let http_base =
        env::var("MM_WT_HTTP_BASE").unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
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
    let connection = endpoint.connect(url.as_str()).await.map_err(|error| {
        deopt_v2_backend::BackendError::Config(format!("WebTransport connect failed: {error}"))
    })?;

    if scenario == "v1c" {
        run_v1c_scenario(&connection, &endpoint, &http_base).await?;
    } else if scenario == "rfq" {
        run_rfq_scenario(&connection, &endpoint, &http_base).await?;
    } else {
        let heartbeat = send_request(
            &connection,
            json!({
                "type": "heartbeat",
                "request_id": "smoke-heartbeat-1",
                "payload": {}
            }),
        )
        .await?;
        print_step("heartbeat", &heartbeat);

        let session = send_request(
            &connection,
            json!({
                "type": "get_session",
                "request_id": "smoke-session-1",
                "payload": {}
            }),
        )
        .await?;
        print_step("get_session", &session);

        connection.close(0_u32.into(), b"smoke complete");
        endpoint.wait_idle().await;
    }
    Ok(())
}

async fn run_rfq_scenario(
    connection: &wtransport::Connection,
    endpoint: &Endpoint<wtransport::endpoint::endpoint_side::Client>,
    http_base: &str,
) -> deopt_v2_backend::Result<()> {
    let http = reqwest::Client::new();
    let taker = env::var("MM_WT_RFQ_TAKER").unwrap_or_else(|_| RFQ_TAKER.to_string());
    let mm_account =
        env::var("MM_WT_RFQ_MM_ACCOUNT").unwrap_or_else(|_| RFQ_MM_ACCOUNT.to_string());
    let heartbeat = send_request(
        connection,
        json!({
            "type": "heartbeat",
            "request_id": "rfq-heartbeat-1",
            "payload": {}
        }),
    )
    .await?;
    print_step("heartbeat", &heartbeat);
    assert_ok(&heartbeat, "heartbeat")?;

    let session = send_request(
        connection,
        json!({
            "type": "get_session",
            "request_id": "rfq-session-1",
            "payload": {}
        }),
    )
    .await?;
    print_step("get_session", &session);
    assert_ok(&session, "get_session")?;

    let created = http_post(
        &http,
        http_base,
        "/rfqs",
        json!({
            "taker": taker,
            "market_id": 1,
            "side": "buy",
            "size_1e8": "100000000",
            "limit_price_1e8": "305000000000",
            "ttl_ms": 30000
        }),
    )
    .await?;
    print_step("http_create_rfq", &created);
    let rfq_id = required_str(&created, &["rfq_id"])?;

    let rfq_request = receive_server_message(connection, "rfq_request").await?;
    print_step("rfq_request", &rfq_request);
    if required_str(&rfq_request, &["payload", "rfq_id"])? != rfq_id {
        return Err(deopt_v2_backend::BackendError::Config(format!(
            "rfq_request rfq_id mismatch: expected {rfq_id}, got {}",
            required_str(&rfq_request, &["payload", "rfq_id"])?
        )));
    }

    let quote = send_request(
        connection,
        json!({
            "type": "rfq_quote",
            "request_id": "rfq-quote-1",
            "payload": {
                "rfq_id": rfq_id,
                "mm_account": mm_account,
                "price_1e8": env::var("MM_WT_RFQ_PRICE_1E8").unwrap_or_else(|_| "300100000000".to_string()),
                "size_1e8": env::var("MM_WT_RFQ_SIZE_1E8").unwrap_or_else(|_| "100000000".to_string()),
                "client_quote_id": env::var("MM_WT_RFQ_CLIENT_QUOTE_ID").unwrap_or_else(|_| "smoke-rfq-quote-1".to_string()),
                "quote_ttl_ms": env::var("MM_WT_RFQ_QUOTE_TTL_MS")
                    .ok()
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(10000)
            }
        }),
    )
    .await?;
    print_step("rfq_quote", &quote);
    assert_ok(&quote, "rfq_quote")?;
    let quote_id = required_str(&quote, &["payload", "quote_id"])?;

    let quotes = http_get(&http, http_base, &format!("/rfqs/{rfq_id}/quotes")).await?;
    print_step("http_quotes", &quotes);
    assert_quote_list_contains(&quotes, &quote_id)?;

    let accepted = http_post(
        &http,
        http_base,
        &format!("/rfqs/{rfq_id}/accept/{quote_id}"),
        json!({}),
    )
    .await?;
    print_step("http_accept_quote", &accepted);

    let accepted_notice = receive_server_message(connection, "rfq_quote_accepted").await?;
    print_step("rfq_quote_accepted", &accepted_notice);
    if required_str(&accepted_notice, &["payload", "quote_id"])? != quote_id {
        return Err(deopt_v2_backend::BackendError::Config(
            "rfq_quote_accepted quote_id mismatch".to_string(),
        ));
    }

    let intents = http_get(&http, http_base, "/execution-intents").await?;
    print_step("execution_intents", &intents);
    let transactions = http_get(&http, http_base, "/executor/transactions").await?;
    print_step("executor_transactions", &transactions);
    let confirmations = http_get(&http, http_base, "/executor/confirmations/status").await?;
    print_step("confirmations", &confirmations);

    connection.close(0_u32.into(), b"rfq smoke complete");
    endpoint.wait_idle().await;
    Ok(())
}

async fn run_v1c_scenario(
    connection: &wtransport::Connection,
    endpoint: &Endpoint<wtransport::endpoint::endpoint_side::Client>,
    http_base: &str,
) -> deopt_v2_backend::Result<()> {
    let http = reqwest::Client::new();
    let nonce_base = nonce_base();

    let health = http_get(&http, http_base, "/health").await?;
    print_step("http_health", &health);
    assert_book(&http, http_base, "initial", &[], &[]).await?;

    let heartbeat = send_request(
        connection,
        json!({
            "type": "heartbeat",
            "request_id": "v1c-heartbeat-1",
            "payload": {}
        }),
    )
    .await?;
    print_step("heartbeat", &heartbeat);

    let session = send_request(
        connection,
        json!({
            "type": "get_session",
            "request_id": "v1c-session-1",
            "payload": {}
        }),
    )
    .await?;
    print_step("get_session", &session);

    let submit = send_request(
        connection,
        json!({
            "type": "submit_order",
            "request_id": "v1c-submit-1",
            "payload": order("submit-1", "buy", "10000000000", nonce_base + 1)
        }),
    )
    .await?;
    print_step("submit_order", &submit);
    assert_ok(&submit, "submit_order")?;
    assert_book(
        &http,
        http_base,
        "after_submit_order",
        &[("10000000000", "100000000")],
        &[],
    )
    .await?;

    let cancel = send_request(
        connection,
        json!({
            "type": "cancel_order",
            "request_id": "v1c-cancel-1",
            "payload": {
                "account": ACCOUNT,
                "market_id": 1,
                "client_order_id": "submit-1"
            }
        }),
    )
    .await?;
    print_step("cancel_order", &cancel);
    assert_ok(&cancel, "cancel_order")?;
    assert_book(&http, http_base, "after_cancel_order", &[], &[]).await?;

    let bulk_submit = send_request(
        connection,
        json!({
            "type": "bulk_submit",
            "request_id": "v1c-bulk-submit-1",
            "payload": {
                "orders": [
                    order("bulk-1", "buy", "10100000000", nonce_base + 2),
                    order("bulk-2", "buy", "10200000000", nonce_base + 3)
                ]
            }
        }),
    )
    .await?;
    print_step("bulk_submit", &bulk_submit);
    assert_ok(&bulk_submit, "bulk_submit")?;
    assert_book(
        &http,
        http_base,
        "after_bulk_submit",
        &[("10200000000", "100000000"), ("10100000000", "100000000")],
        &[],
    )
    .await?;

    let bulk_cancel = send_request(
        connection,
        json!({
            "type": "bulk_cancel",
            "request_id": "v1c-bulk-cancel-1",
            "payload": {
                "cancels": [
                    {"account": ACCOUNT, "market_id": 1, "client_order_id": "bulk-1"},
                    {"account": ACCOUNT, "market_id": 1, "client_order_id": "bulk-2"}
                ]
            }
        }),
    )
    .await?;
    print_step("bulk_cancel", &bulk_cancel);
    assert_ok(&bulk_cancel, "bulk_cancel")?;
    assert_book(&http, http_base, "after_bulk_cancel", &[], &[]).await?;

    let quote_one = send_request(
        connection,
        json!({
            "type": "quote_replace",
            "request_id": "v1c-quote-1",
            "payload": quote_replace("quote1-bid", "10300000000", nonce_base + 4, "quote1-ask", "20300000000", nonce_base + 5)
        }),
    )
    .await?;
    print_step("quote_replace_1", &quote_one);
    assert_ok(&quote_one, "quote_replace_1")?;
    assert_book(
        &http,
        http_base,
        "after_quote_replace_1",
        &[("10300000000", "100000000")],
        &[("20300000000", "100000000")],
    )
    .await?;

    let quote_two = send_request(
        connection,
        json!({
            "type": "quote_replace",
            "request_id": "v1c-quote-2",
            "payload": quote_replace("quote2-bid", "10400000000", nonce_base + 6, "quote2-ask", "20400000000", nonce_base + 7)
        }),
    )
    .await?;
    print_step("quote_replace_2", &quote_two);
    assert_ok(&quote_two, "quote_replace_2")?;
    assert_book(
        &http,
        http_base,
        "after_quote_replace_2",
        &[("10400000000", "100000000")],
        &[("20400000000", "100000000")],
    )
    .await?;

    let cancel_all = send_request(
        connection,
        json!({
            "type": "cancel_all",
            "request_id": "v1c-cancel-all-1",
            "payload": {
                "account": ACCOUNT,
                "market_id": 1
            }
        }),
    )
    .await?;
    print_step("cancel_all", &cancel_all);
    assert_ok(&cancel_all, "cancel_all")?;
    assert_book(&http, http_base, "after_cancel_all", &[], &[]).await?;

    let cod_one = send_request(
        connection,
        json!({
            "type": "submit_order",
            "request_id": "v1c-cod-submit-1",
            "payload": order("cod-1", "buy", "10500000000", nonce_base + 8)
        }),
    )
    .await?;
    print_step("cancel_on_disconnect_submit_1", &cod_one);
    assert_ok(&cod_one, "cancel_on_disconnect_submit_1")?;

    let cod_two = send_request(
        connection,
        json!({
            "type": "submit_order",
            "request_id": "v1c-cod-submit-2",
            "payload": order("cod-2", "buy", "10600000000", nonce_base + 9)
        }),
    )
    .await?;
    print_step("cancel_on_disconnect_submit_2", &cod_two);
    assert_ok(&cod_two, "cancel_on_disconnect_submit_2")?;
    assert_book(
        &http,
        http_base,
        "before_cancel_on_disconnect",
        &[("10600000000", "100000000"), ("10500000000", "100000000")],
        &[],
    )
    .await?;

    let intents_before = http_get(&http, http_base, "/execution-intents").await?;
    print_step("execution_intents_before_disconnect", &intents_before);
    let transactions_before = http_get(&http, http_base, "/executor/transactions").await?;
    print_step("transactions_before_disconnect", &transactions_before);
    let confirmations_before = http_get(&http, http_base, "/executor/confirmations/status").await?;
    print_step("confirmations_before_disconnect", &confirmations_before);

    connection.close(0_u32.into(), b"v1c cancel-on-disconnect");
    endpoint.wait_idle().await;
    wait_for_empty_book(&http, http_base).await?;
    let intents_after = http_get(&http, http_base, "/execution-intents").await?;
    print_step("execution_intents_after_disconnect", &intents_after);
    let transactions_after = http_get(&http, http_base, "/executor/transactions").await?;
    print_step("transactions_after_disconnect", &transactions_after);
    let confirmations_after = http_get(&http, http_base, "/executor/confirmations/status").await?;
    print_step("confirmations_after_disconnect", &confirmations_after);

    if intents_before != intents_after {
        return Err(deopt_v2_backend::BackendError::Config(
            "execution intents changed during cancel-on-disconnect".to_string(),
        ));
    }
    if transactions_before != transactions_after {
        return Err(deopt_v2_backend::BackendError::Config(
            "executor transactions changed during cancel-on-disconnect".to_string(),
        ));
    }
    if confirmations_before["confirmed"] != confirmations_after["confirmed"] {
        return Err(deopt_v2_backend::BackendError::Config(
            "confirmed count changed during cancel-on-disconnect".to_string(),
        ));
    }

    Ok(())
}

async fn receive_server_message(
    connection: &wtransport::Connection,
    expected_type: &str,
) -> deopt_v2_backend::Result<Value> {
    let mut recv = timeout(Duration::from_secs(10), connection.accept_uni())
        .await
        .map_err(|_| {
            deopt_v2_backend::BackendError::Config(format!(
                "timed out waiting for server-initiated {expected_type}"
            ))
        })?
        .map_err(|error| deopt_v2_backend::BackendError::Config(error.to_string()))?;
    let payload = read_frame(&mut recv, MM_GATEWAY_MAX_FRAME_BYTES)
        .await
        .map_err(|error| deopt_v2_backend::BackendError::Config(error.to_string()))?
        .ok_or_else(|| {
            deopt_v2_backend::BackendError::Config(format!(
                "server closed stream without {expected_type} frame"
            ))
        })?;
    let value: Value = serde_json::from_slice(&payload)
        .map_err(|error| deopt_v2_backend::BackendError::Config(error.to_string()))?;
    if value["type"] != expected_type {
        return Err(deopt_v2_backend::BackendError::Config(format!(
            "expected {expected_type}, got {value}"
        )));
    }
    Ok(value)
}

async fn send_request(
    connection: &wtransport::Connection,
    request: serde_json::Value,
) -> deopt_v2_backend::Result<serde_json::Value> {
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .map_err(|error| {
            deopt_v2_backend::BackendError::Config(format!("open_bi request failed: {error}"))
        })?
        .await
        .map_err(|error| {
            deopt_v2_backend::BackendError::Config(format!("open_bi stream failed: {error}"))
        })?;
    write_json_frame(&mut send, &request, MM_GATEWAY_MAX_FRAME_BYTES)
        .await
        .map_err(|error| {
            deopt_v2_backend::BackendError::Config(format!("write request frame failed: {error}"))
        })?;
    if let Err(error) = send.finish().await {
        println!("request_finish_warning:{error}");
    }
    let response = read_frame(&mut recv, MM_GATEWAY_MAX_FRAME_BYTES)
        .await
        .map_err(|error| {
            deopt_v2_backend::BackendError::Config(format!("read response frame failed: {error}"))
        })?
        .ok_or_else(|| {
            deopt_v2_backend::BackendError::Config(
                "MM gateway closed stream without a response frame".to_string(),
            )
        })?;
    serde_json::from_slice(&response)
        .map_err(|error| deopt_v2_backend::BackendError::Config(error.to_string()))
}

async fn http_post(
    client: &reqwest::Client,
    base: &str,
    path: &str,
    payload: Value,
) -> deopt_v2_backend::Result<Value> {
    let response = client
        .post(format!("{base}{path}"))
        .json(&payload)
        .send()
        .await
        .map_err(|error| deopt_v2_backend::BackendError::Config(error.to_string()))?;
    let status = response.status();
    let body = response
        .json::<Value>()
        .await
        .map_err(|error| deopt_v2_backend::BackendError::Config(error.to_string()))?;
    if !status.is_success() {
        return Err(deopt_v2_backend::BackendError::Config(format!(
            "HTTP POST {path} returned {status}: {body}"
        )));
    }
    Ok(body)
}

async fn http_get(
    client: &reqwest::Client,
    base: &str,
    path: &str,
) -> deopt_v2_backend::Result<Value> {
    let response = client
        .get(format!("{base}{path}"))
        .send()
        .await
        .map_err(|error| deopt_v2_backend::BackendError::Config(error.to_string()))?;
    let status = response.status();
    let body = response
        .json::<Value>()
        .await
        .map_err(|error| deopt_v2_backend::BackendError::Config(error.to_string()))?;
    if !status.is_success() {
        return Err(deopt_v2_backend::BackendError::Config(format!(
            "HTTP GET {path} returned {status}: {body}"
        )));
    }
    Ok(body)
}

fn required_str(value: &Value, path: &[&str]) -> deopt_v2_backend::Result<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key).ok_or_else(|| {
            deopt_v2_backend::BackendError::Config(format!("missing field {}", path.join(".")))
        })?;
    }
    current
        .as_str()
        .map(|value| value.to_string())
        .ok_or_else(|| {
            deopt_v2_backend::BackendError::Config(format!(
                "field {} is not a string",
                path.join(".")
            ))
        })
}

fn assert_quote_list_contains(quotes: &Value, quote_id: &str) -> deopt_v2_backend::Result<()> {
    let Some(quotes) = quotes.as_array() else {
        return Err(deopt_v2_backend::BackendError::Config(
            "quotes response is not an array".to_string(),
        ));
    };
    let Some(quote) = quotes
        .iter()
        .find(|quote| quote["quote_id"].as_str() == Some(quote_id))
    else {
        return Err(deopt_v2_backend::BackendError::Config(format!(
            "quote {quote_id} missing from HTTP quote list"
        )));
    };
    if quote["status"] != "active" {
        return Err(deopt_v2_backend::BackendError::Config(format!(
            "quote {quote_id} status is not active: {quote}"
        )));
    }
    if quote["session_id"].as_str().unwrap_or_default().is_empty() {
        return Err(deopt_v2_backend::BackendError::Config(format!(
            "quote {quote_id} missing session_id: {quote}"
        )));
    }
    Ok(())
}

async fn assert_book(
    client: &reqwest::Client,
    base: &str,
    label: &str,
    bids: &[(&str, &str)],
    asks: &[(&str, &str)],
) -> deopt_v2_backend::Result<()> {
    let book = http_get(client, base, "/orderbook/1").await?;
    print_step(label, &book);
    let actual_bids = levels(&book["bids"]);
    let actual_asks = levels(&book["asks"]);
    let expected_bids = levels_from_pairs(bids);
    let expected_asks = levels_from_pairs(asks);
    if actual_bids != expected_bids || actual_asks != expected_asks {
        return Err(deopt_v2_backend::BackendError::Config(format!(
            "{label} orderbook mismatch: bids {actual_bids:?} != {expected_bids:?}, asks {actual_asks:?} != {expected_asks:?}"
        )));
    }
    Ok(())
}

async fn wait_for_empty_book(client: &reqwest::Client, base: &str) -> deopt_v2_backend::Result<()> {
    for attempt in 0..20 {
        let book = http_get(client, base, "/orderbook/1").await?;
        if levels(&book["bids"]).is_empty() && levels(&book["asks"]).is_empty() {
            print_step("after_cancel_on_disconnect", &book);
            return Ok(());
        }
        if attempt == 19 {
            print_step("after_cancel_on_disconnect_timeout", &book);
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    Err(deopt_v2_backend::BackendError::Config(
        "orderbook did not become empty after disconnect".to_string(),
    ))
}

fn order(client_order_id: &str, side: &str, price_1e8: &str, nonce: u64) -> Value {
    json!({
        "market_id": 1,
        "account": ACCOUNT,
        "side": side,
        "price_1e8": price_1e8,
        "size_1e8": "100000000",
        "time_in_force": "gtc",
        "reduce_only": false,
        "post_only": false,
        "client_order_id": client_order_id,
        "nonce": nonce,
        "deadline_ms": 9999999999999i64,
        "signature": VALID_SIGNATURE
    })
}

fn quote_replace(
    bid_client_order_id: &str,
    bid_price_1e8: &str,
    bid_nonce: u64,
    ask_client_order_id: &str,
    ask_price_1e8: &str,
    ask_nonce: u64,
) -> Value {
    json!({
        "market_id": 1,
        "account": ACCOUNT,
        "cancel_previous": true,
        "bid": quote_leg(bid_client_order_id, bid_price_1e8, bid_nonce),
        "ask": quote_leg(ask_client_order_id, ask_price_1e8, ask_nonce)
    })
}

fn quote_leg(client_order_id: &str, price_1e8: &str, nonce: u64) -> Value {
    json!({
        "price_1e8": price_1e8,
        "size_1e8": "100000000",
        "client_order_id": client_order_id,
        "nonce": nonce,
        "deadline_ms": 9999999999999i64,
        "signature": VALID_SIGNATURE
    })
}

fn levels(value: &Value) -> Vec<(String, String)> {
    value
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .map(|level| {
            (
                level["price1e8"].as_str().unwrap_or_default().to_string(),
                level["totalSize1e8"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .collect()
}

fn levels_from_pairs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(price, size)| ((*price).to_string(), (*size).to_string()))
        .collect()
}

fn assert_ok(value: &Value, label: &str) -> deopt_v2_backend::Result<()> {
    if value["ok"] == true {
        Ok(())
    } else {
        Err(deopt_v2_backend::BackendError::Config(format!(
            "{label} failed: {value}"
        )))
    }
}

fn nonce_base() -> u64 {
    env::var("MM_WT_NONCE_BASE")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs().saturating_mul(100))
                .unwrap_or(1_000_000)
        })
}

fn print_step(label: &str, value: &Value) {
    println!("{label}:{}", serde_json::to_string(&value).unwrap());
}
