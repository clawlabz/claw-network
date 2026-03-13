//! JSON-RPC 2.0 server over HTTP with /metrics and /health endpoints.

use axum::{
    extract::State,
    http::{Method, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
use tower_http::cors::{Any, CorsLayer};

use crate::chain::Chain;
use crate::metrics;

/// Uptime tracking — set once at server start.
static START_TIME: std::sync::OnceLock<u64> = std::sync::OnceLock::new();

#[derive(Deserialize)]
struct RpcRequest {
    jsonrpc: String,
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Serialize)]
struct RpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

#[derive(Serialize)]
struct RpcError {
    code: i32,
    message: String,
}

impl RpcResponse {
    fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn err(id: Value, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(RpcError { code, message }),
        }
    }
}

/// Whether the faucet RPC is enabled (testnet/devnet only).
static FAUCET_ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

async fn handle_rpc(State(chain): State<Chain>, Json(req): Json<RpcRequest>) -> Json<RpcResponse> {
    if req.jsonrpc != "2.0" {
        return Json(RpcResponse::err(req.id, -32600, "Invalid JSON-RPC version".into()));
    }

    let result = match req.method.as_str() {
        "clw_blockNumber" => {
            let height = chain.get_block_number();
            Ok(serde_json::json!(height))
        }
        "clw_getBlockByNumber" => {
            let height = req.params.get(0)
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            match chain.get_block(height) {
                Some(block) => Ok(serde_json::to_value(block).unwrap()),
                None => Ok(Value::Null),
            }
        }
        "clw_getBalance" => {
            let addr = parse_address(&req.params, 0);
            match addr {
                Ok(a) => Ok(serde_json::json!(chain.get_balance(&a).to_string())),
                Err(e) => Err(e),
            }
        }
        "clw_getTokenBalance" => {
            let addr = parse_address(&req.params, 0);
            let token = parse_address(&req.params, 1);
            match (addr, token) {
                (Ok(a), Ok(t)) => Ok(serde_json::json!(chain.get_token_balance(&a, &t).to_string())),
                _ => Err("invalid params".into()),
            }
        }
        "clw_getAgent" => {
            let addr = parse_address(&req.params, 0);
            match addr {
                Ok(a) => match chain.get_agent(&a) {
                    Some(agent) => Ok(serde_json::to_value(agent).unwrap()),
                    None => Ok(Value::Null),
                },
                Err(e) => Err(e),
            }
        }
        "clw_getReputation" => {
            let addr = parse_address(&req.params, 0);
            match addr {
                Ok(a) => Ok(serde_json::to_value(chain.get_reputation(&a)).unwrap()),
                Err(e) => Err(e),
            }
        }
        "clw_getServices" => {
            let stype = req.params.get(0).and_then(|v| v.as_str());
            Ok(serde_json::to_value(chain.get_services(stype)).unwrap())
        }
        "clw_getTransactionReceipt" => {
            let hash = parse_address(&req.params, 0);
            match hash {
                Ok(h) => match chain.get_tx_receipt(&h) {
                    Some((height, index)) => Ok(serde_json::json!({
                        "blockHeight": height,
                        "transactionIndex": index,
                    })),
                    None => Ok(Value::Null),
                },
                Err(e) => Err(e),
            }
        }
        "clw_sendTransaction" => {
            let hex_str = req.params.get(0).and_then(|v| v.as_str());
            match hex_str {
                Some(h) => {
                    match hex::decode(h) {
                        Ok(bytes) => {
                            match borsh::from_slice::<claw_types::Transaction>(&bytes) {
                                Ok(tx) => match chain.submit_tx(tx) {
                                    Ok(hash) => Ok(serde_json::json!(hex::encode(hash))),
                                    Err(e) => Err(e),
                                },
                                Err(e) => Err(format!("decode tx: {e}")),
                            }
                        }
                        Err(e) => Err(format!("invalid hex: {e}")),
                    }
                }
                None => Err("missing tx hex param".into()),
            }
        }
        "clw_getNonce" => {
            let addr = parse_address(&req.params, 0);
            match addr {
                Ok(a) => Ok(serde_json::json!(chain.get_nonce(&a))),
                Err(e) => Err(e),
            }
        }
        "clw_getTokenInfo" => {
            let token = parse_address(&req.params, 0);
            match token {
                Ok(t) => match chain.get_token_info(&t) {
                    Some(info) => Ok(serde_json::to_value(info).unwrap()),
                    None => Ok(Value::Null),
                },
                Err(e) => Err(e),
            }
        }
        "clw_faucet" => {
            if !FAUCET_ENABLED.get().copied().unwrap_or(false) {
                Err("faucet is disabled on this network".into())
            } else {
                let addr = parse_address(&req.params, 0);
                match addr {
                    Ok(a) => match chain.faucet_drip(&a) {
                        Ok(new_bal) => Ok(serde_json::json!({
                            "address": hex::encode(a),
                            "amount": "10000000000",
                            "newBalance": new_bal.to_string(),
                        })),
                        Err(e) => Err(e),
                    },
                    Err(e) => Err(e),
                }
            }
        }
        _ => Err(format!("method not found: {}", req.method)),
    };

    match result {
        Ok(val) => Json(RpcResponse::ok(req.id, val)),
        Err(msg) => Json(RpcResponse::err(req.id, -32000, msg)),
    }
}

fn parse_address(params: &Value, index: usize) -> Result<[u8; 32], String> {
    let hex_str = params
        .get(index)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("missing param at index {index}"))?;
    let bytes = hex::decode(hex_str).map_err(|e| format!("invalid hex: {e}"))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "expected 32 bytes".to_string())?;
    Ok(arr)
}

/// GET /metrics — Prometheus text exposition format.
async fn handle_metrics(State(chain): State<Chain>) -> impl IntoResponse {
    // Update gauges from current chain state before gathering
    metrics::BLOCK_HEIGHT.set(chain.get_block_number() as f64);
    metrics::MEMPOOL_SIZE.set(chain.get_mempool_size() as f64);
    metrics::PEERS_CONNECTED.set(chain.get_p2p_peer_count() as f64);

    let body = metrics::gather();
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}

/// GET /health — Node health/status check.
async fn handle_health(State(chain): State<Chain>) -> Json<Value> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let start = START_TIME.get().copied().unwrap_or(now);
    let uptime_secs = now.saturating_sub(start);

    let height = chain.get_block_number();
    let peer_count = chain.get_p2p_peer_count();
    let mempool_size = chain.get_mempool_size();
    let last_block_ts = chain.get_last_block_timestamp();
    let epoch = chain.get_epoch();

    // Consider unhealthy if last block is older than 60 seconds
    let block_age = now.saturating_sub(last_block_ts);
    let status = if block_age < 60 || height == 0 { "ok" } else { "degraded" };

    Json(serde_json::json!({
        "status": status,
        "version": env!("CARGO_PKG_VERSION"),
        "height": height,
        "epoch": epoch,
        "peer_count": peer_count,
        "mempool_size": mempool_size,
        "last_block_age_secs": block_age,
        "uptime_secs": uptime_secs,
    }))
}

/// Start the RPC server. Returns a JoinHandle.
pub async fn start(chain: Chain, port: u16, faucet_enabled: bool) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    let _ = FAUCET_ENABLED.set(faucet_enabled);

    // Record start time for uptime tracking
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let _ = START_TIME.set(now);

    // Initialize metrics (force lazy statics)
    let _ = &*metrics::BLOCKS_TOTAL;
    let _ = &*metrics::TRANSACTIONS_TOTAL;
    let _ = &*metrics::PEERS_CONNECTED;
    let _ = &*metrics::MEMPOOL_SIZE;
    let _ = &*metrics::BLOCK_HEIGHT;
    let _ = &*metrics::BLOCK_TIME_SECONDS;

    // Set initial height
    metrics::BLOCK_HEIGHT.set(chain.get_block_number() as f64);

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(Any);

    let app = Router::new()
        .route("/", post(handle_rpc))
        .route("/metrics", get(handle_metrics))
        .route("/health", get(handle_health))
        .layer(cors)
        .with_state(chain);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    Ok(handle)
}
