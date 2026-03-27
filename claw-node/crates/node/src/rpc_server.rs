//! JSON-RPC 2.0 server over HTTP with /metrics and /health endpoints.

use axum::{
    extract::{ConnectInfo, State},
    http::{Method, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tower::limit::ConcurrencyLimitLayer;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

use claw_types::transaction::{
    TxType, TokenTransferPayload, TokenMintTransferPayload, ReputationAttestPayload,
    TokenApprovePayload, TokenBurnPayload,
};

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

/// Per-address faucet cooldown tracking.
const FAUCET_COOLDOWN_SECS: u64 = 3600;
static FAUCET_LAST_DRIP: std::sync::OnceLock<Mutex<HashMap<[u8; 32], Instant>>> =
    std::sync::OnceLock::new();

fn faucet_cooldown_map() -> &'static Mutex<HashMap<[u8; 32], Instant>> {
    FAUCET_LAST_DRIP.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Per-IP sliding-window rate limiter for `claw_callContractView`.
/// Each entry stores (window_start, call_count). Window resets after 1 second.
const CONTRACT_VIEW_RATE_LIMIT: u32 = 10;
const CONTRACT_VIEW_RATE_WINDOW: Duration = Duration::from_secs(1);

static CONTRACT_VIEW_RATE: std::sync::OnceLock<Mutex<HashMap<IpAddr, (Instant, u32)>>> =
    std::sync::OnceLock::new();

fn contract_view_rate_map() -> &'static Mutex<HashMap<IpAddr, (Instant, u32)>> {
    CONTRACT_VIEW_RATE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Returns true if the request from `ip` is within the allowed rate limit.
fn check_contract_view_rate(ip: IpAddr) -> bool {
    let mut map = contract_view_rate_map().lock().unwrap();
    let now = Instant::now();
    let entry = map.entry(ip).or_insert((now, 0));
    if now.duration_since(entry.0) >= CONTRACT_VIEW_RATE_WINDOW {
        // Window has elapsed — reset counter.
        *entry = (now, 1);
        true
    } else if entry.1 < CONTRACT_VIEW_RATE_LIMIT {
        entry.1 += 1;
        true
    } else {
        false
    }
}

async fn handle_rpc(
    State(chain): State<Chain>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(req): Json<RpcRequest>,
) -> Json<RpcResponse> {
    if req.jsonrpc != "2.0" {
        return Json(RpcResponse::err(req.id, -32600, "Invalid JSON-RPC version".into()));
    }

    let result = match req.method.as_str() {
        "claw_blockNumber" => {
            let height = chain.get_block_number();
            Ok(serde_json::json!(height))
        }
        "claw_getBlockByNumber" => {
            let height = req.params.get(0)
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            match chain.get_block(height) {
                Some(block) => {
                    // Serialize block with tx hashes included
                    let mut block_json = match serde_json::to_value(&block) {
                        Ok(v) => v,
                        Err(e) => return Json(RpcResponse::err(req.id, -32603, format!("Serialization error: {e}"))),
                    };
                    if let Some(txs) = block_json.get_mut("transactions").and_then(|v| v.as_array_mut()) {
                        for (i, tx_json) in txs.iter_mut().enumerate() {
                            if let Some(tx) = block.transactions.get(i) {
                                let hash = tx.hash();
                                tx_json.as_object_mut().map(|obj| {
                                    obj.insert("hash".to_string(), serde_json::json!(hash));
                                });
                            }
                        }
                    }
                    Ok(block_json)
                }
                None => Ok(Value::Null),
            }
        }
        "claw_getBalance" => {
            let addr = parse_address(&req.params, 0);
            match addr {
                Ok(a) => Ok(serde_json::json!(chain.get_balance(&a).to_string())),
                Err(e) => Err(e),
            }
        }
        "claw_getTokenBalance" => {
            let addr = parse_address(&req.params, 0);
            let token = parse_address(&req.params, 1);
            match (addr, token) {
                (Ok(a), Ok(t)) => Ok(serde_json::json!(chain.get_token_balance(&a, &t).to_string())),
                _ => Err("invalid params".into()),
            }
        }
        "claw_getAgent" => {
            let addr = parse_address(&req.params, 0);
            match addr {
                Ok(a) => match chain.get_agent(&a) {
                    Some(agent) => serde_json::to_value(agent)
                        .map_err(|e| format!("Serialization error: {e}")),
                    None => Ok(Value::Null),
                },
                Err(e) => Err(e),
            }
        }
        "claw_getReputation" => {
            let addr = parse_address(&req.params, 0);
            match addr {
                Ok(a) => serde_json::to_value(chain.get_reputation(&a))
                    .map_err(|e| format!("Serialization error: {e}")),
                Err(e) => Err(e),
            }
        }
        "claw_getServices" => {
            let stype = req.params.get(0).and_then(|v| v.as_str());
            serde_json::to_value(chain.get_services(stype))
                .map_err(|e| format!("Serialization error: {e}"))
        }
        "claw_getTransactionReceipt" => {
            let hash = parse_address(&req.params, 0);
            match hash {
                Ok(h) => match chain.get_tx_receipt(&h) {
                    Some(resp) => {
                        let mut result = serde_json::json!({
                            "blockHeight": resp.block_height,
                            "transactionIndex": resp.transaction_index,
                        });
                        if let Some(receipt) = resp.receipt {
                            result["success"] = serde_json::json!(receipt.success);
                            result["fuelConsumed"] = serde_json::json!(receipt.fuel_consumed);
                            result["fuelLimit"] = serde_json::json!(receipt.fuel_limit);
                            result["returnData"] = serde_json::json!(hex::encode(&receipt.return_data));
                            result["errorMessage"] = serde_json::json!(receipt.error_message);
                            result["events"] = serde_json::json!(
                                receipt.events.iter().map(|e| serde_json::json!({
                                    "topic": e.topic,
                                    "data": hex::encode(&e.data),
                                })).collect::<Vec<_>>()
                            );
                            result["logs"] = serde_json::json!(receipt.logs);
                        }
                        Ok(result)
                    },
                    None => Ok(Value::Null),
                },
                Err(e) => Err(e),
            }
        }
        "claw_getTransactionByHash" => {
            let hash = parse_address(&req.params, 0);
            match hash {
                Ok(h) => match chain.get_tx_by_hash(&h) {
                    Some((tx, block_height, timestamp)) => {
                        let tx_hash = tx.hash();
                        let type_name = tx_type_name(tx.tx_type);
                        let (to, amount) = parse_tx_recipient(&tx);
                        Ok(serde_json::json!({
                            "hash": hex::encode(tx_hash),
                            "txType": tx.tx_type as u8,
                            "typeName": type_name,
                            "from": hex::encode(tx.from),
                            "to": to.map(|addr| hex::encode(addr)),
                            "amount": amount.map(|a| a.to_string()),
                            "nonce": tx.nonce,
                            "blockHeight": block_height,
                            "timestamp": timestamp,
                            "fee": "1000000",
                        }))
                    }
                    None => Ok(Value::Null),
                },
                Err(e) => Err(e),
            }
        }
        "claw_sendTransaction" => {
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
        "claw_getNonce" => {
            let addr = parse_address(&req.params, 0);
            match addr {
                Ok(a) => Ok(serde_json::json!(chain.get_nonce(&a))),
                Err(e) => Err(e),
            }
        }
        "claw_getTokenInfo" => {
            let token = parse_address(&req.params, 0);
            match token {
                Ok(t) => match chain.get_token_info(&t) {
                    Some(info) => serde_json::to_value(info)
                        .map_err(|e| format!("Serialization error: {e}")),
                    None => Ok(Value::Null),
                },
                Err(e) => Err(e),
            }
        }
        "claw_getTransactionsByAddress" => {
            let addr = parse_address(&req.params, 0);
            let limit = req.params.get(1)
                .and_then(|v| v.as_u64())
                .unwrap_or(50)
                .min(200) as usize;
            let offset = req.params.get(2)
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            match addr {
                Ok(a) => {
                    let txs = chain.get_transactions_by_address(&a, limit, offset);
                    let results: Vec<serde_json::Value> = txs.into_iter().map(|(height, _tx_idx, tx, timestamp)| {
                        let (to, amount) = extract_to_and_amount(&tx);
                        serde_json::json!({
                            "hash": hex::encode(tx.hash()),
                            "txType": tx.tx_type,
                            "from": hex::encode(tx.from),
                            "to": to,
                            "amount": amount,
                            "blockHeight": height,
                            "timestamp": timestamp,
                            "nonce": tx.nonce,
                        })
                    }).collect();
                    Ok(serde_json::json!(results))
                }
                Err(e) => Err(e),
            }
        }
        "claw_getContractInfo" => {
            let addr = parse_address(&req.params, 0);
            match addr {
                Ok(a) => match chain.get_contract_info(&a) {
                    Some(instance) => Ok(serde_json::json!({
                        "address": hex::encode(instance.address),
                        "codeHash": hex::encode(instance.code_hash),
                        "creator": hex::encode(instance.creator),
                        "deployedAt": instance.deployed_at,
                    })),
                    None => Ok(Value::Null),
                },
                Err(e) => Err(e),
            }
        }
        "claw_getContractStorage" => {
            let addr = parse_address(&req.params, 0);
            let key_hex = req.params.get(1).and_then(|v| v.as_str()).unwrap_or("");
            let key_bytes = hex::decode(key_hex).map_err(|e| format!("invalid key hex: {e}"));
            match (addr, key_bytes) {
                (Ok(a), Ok(k)) => match chain.get_contract_storage_value(&a, &k) {
                    Some(value) => Ok(serde_json::json!(hex::encode(value))),
                    None => Ok(Value::Null),
                },
                (Err(e), _) | (_, Err(e)) => Err(e),
            }
        }
        "claw_getContractCode" => {
            let addr = parse_address(&req.params, 0);
            match addr {
                Ok(a) => match chain.get_contract_code(&a) {
                    Some(code) => Ok(serde_json::json!({
                        "code": hex::encode(&code),
                        "size": code.len(),
                    })),
                    None => Ok(Value::Null),
                },
                Err(e) => Err(e),
            }
        }
        "claw_getContractEvents" => {
            // params: [contract_hex, from_block?, to_block?]
            let addr = parse_address(&req.params, 0);
            let from_block = req.params.get(1).and_then(|v| v.as_u64()).unwrap_or(0);
            let to_block = req.params.get(2).and_then(|v| v.as_u64()).unwrap_or(u64::MAX);
            match addr {
                Ok(a) => {
                    let events = chain.get_contract_events(&a, from_block, to_block);
                    let json_events: Vec<serde_json::Value> = events.iter().filter_map(|e| {
                        if let claw_types::block::BlockEvent::ContractEvent { contract, tx_index, topic, data } = e {
                            Some(serde_json::json!({
                                "contract": hex::encode(contract),
                                "txIndex": tx_index,
                                "topic": topic,
                                "data": hex::encode(data),
                            }))
                        } else {
                            None
                        }
                    }).collect();
                    Ok(serde_json::json!(json_events))
                }
                Err(e) => Err(e),
            }
        }
        "claw_callContractView" => {
            if !check_contract_view_rate(peer.ip()) {
                return Json(RpcResponse::err(
                    req.id,
                    -32029,
                    "rate limit exceeded: max 10 callContractView requests per second per IP".into(),
                ));
            }
            let addr = parse_address(&req.params, 0);
            let method = req.params.get(1).and_then(|v| v.as_str()).unwrap_or("");
            let args_hex = req.params.get(2).and_then(|v| v.as_str()).unwrap_or("");
            let args = hex::decode(args_hex).map_err(|e| format!("invalid args hex: {e}"));
            match (addr, args) {
                (Ok(a), Ok(arg_bytes)) => match chain.call_contract_view(&a, method, &arg_bytes) {
                    Ok(result) => Ok(serde_json::json!({
                        "returnData": hex::encode(&result.return_data),
                        "fuelConsumed": result.fuel_consumed,
                        "logs": result.logs,
                    })),
                    Err(e) => Err(e),
                },
                (Err(e), _) | (_, Err(e)) => Err(e),
            }
        }
        "claw_getBlockRewards" => {
            let height = req.params.get(0)
                .and_then(|v| v.as_u64())
                .ok_or_else(|| "missing or invalid height param".to_string());
            match height {
                Ok(h) => match chain.get_block(h) {
                    Some(block) => {
                        let rewards: Vec<Value> = block.events.iter().filter_map(|e| {
                            match e {
                                claw_types::BlockEvent::RewardDistributed { recipient, amount, reward_type } => {
                                    Some(serde_json::json!({
                                        "recipient": hex::encode(recipient),
                                        "amount": amount.to_string(),
                                        "rewardType": reward_type,
                                    }))
                                }
                                _ => None, // Skip contract events in block rewards query
                            }
                        }).collect();
                        Ok(serde_json::json!(rewards))
                    }
                    None => Ok(Value::Null),
                },
                Err(e) => Err(e),
            }
        }
        "claw_getStake" => {
            let addr = parse_address(&req.params, 0);
            match addr {
                Ok(a) => Ok(serde_json::json!(chain.get_stake(&a).to_string())),
                Err(e) => Err(e),
            }
        }
        "claw_getUnbonding" => {
            let addr = parse_address(&req.params, 0);
            match addr {
                Ok(a) => {
                    let entries = chain.get_unbonding(&a);
                    let results: Vec<serde_json::Value> = entries
                        .into_iter()
                        .map(|e| {
                            serde_json::json!({
                                "address": hex::encode(e.address),
                                "amount": e.amount.to_string(),
                                "releaseHeight": e.release_height,
                            })
                        })
                        .collect();
                    Ok(serde_json::json!(results))
                }
                Err(e) => Err(e),
            }
        }
        "claw_getAgentScore" => {
            let addr = parse_address(&req.params, 0);
            match addr {
                Ok(a) => {
                    let score = chain.get_agent_score(&a);
                    Ok(serde_json::json!({
                        "total": score.total,
                        "activity": score.activity,
                        "uptime": score.uptime,
                        "block_production": score.block_production,
                        "economic": score.economic,
                        "platform": score.platform,
                        "decay_factor": score.decay_factor,
                    }))
                }
                Err(e) => Err(e),
            }
        }
        "claw_getStakeDelegation" => {
            let addr = parse_address(&req.params, 0);
            match addr {
                Ok(a) => {
                    let owner = chain.get_stake_delegation(&a);
                    Ok(serde_json::json!(owner.map(hex::encode)))
                }
                Err(e) => Err(e),
            }
        }
        "claw_getValidators" => {
            Ok(serde_json::json!(chain.get_validators()))
        }
        "claw_getUserDelegations" => {
            let addr = parse_address(&req.params, 0);
            match addr {
                Ok(a) => Ok(serde_json::json!(chain.get_user_delegations(&a))),
                Err(e) => Err(e),
            }
        }
        "claw_getValidatorDetail" => {
            let addr = parse_address(&req.params, 0);
            match addr {
                Ok(a) => match chain.get_validator_detail(&a) {
                    Some(detail) => Ok(detail),
                    None => Ok(Value::Null),
                },
                Err(e) => Err(e),
            }
        }
        "claw_totalSupply" => {
            Ok(serde_json::json!(chain.get_total_supply_audit()))
        }
        "claw_faucet" => {
            if !FAUCET_ENABLED.get().copied().unwrap_or(false) {
                Err("faucet is disabled on this network".into())
            } else {
                let addr = parse_address(&req.params, 0);
                match addr {
                    Ok(a) => {
                        // Check per-address cooldown
                        let mut map = faucet_cooldown_map().lock().unwrap();
                        if let Some(last) = map.get(&a) {
                            if last.elapsed().as_secs() < FAUCET_COOLDOWN_SECS {
                                let remaining = FAUCET_COOLDOWN_SECS - last.elapsed().as_secs();
                                return Json(RpcResponse::err(
                                    req.id,
                                    -32000,
                                    format!("faucet cooldown: try again in {remaining}s"),
                                ));
                            }
                        }
                        match chain.faucet_drip(&a) {
                            Ok(tx_hash) => {
                                map.insert(a, Instant::now());
                                Ok(serde_json::json!({
                                    "address": hex::encode(a),
                                    "amount": "10000000000",
                                    "txHash": hex::encode(tx_hash),
                                }))
                            }
                            Err(e) => Err(e),
                        }
                    }
                    Err(e) => Err(e),
                }
            }
        }
        "claw_estimateFee" => {
            // Gas fee is currently fixed per transaction.
            // Returns the fee in base units (9 decimals).
            Ok(serde_json::json!({
                "fee": claw_types::state::GAS_FEE.to_string(),
                "unit": "base",
                "description": "Fixed fee per transaction",
            }))
        }
        "claw_getTokenAllowance" => {
            let owner = parse_address(&req.params, 0);
            let spender = parse_address(&req.params, 1);
            let token = parse_address(&req.params, 2);
            match (owner, spender, token) {
                (Ok(o), Ok(s), Ok(t)) => Ok(serde_json::json!(chain.get_token_allowance(&o, &s, &t).to_string())),
                _ => Err("invalid params: expected (owner, spender, tokenId)".into()),
            }
        }
        "claw_getMinerInfo" => {
            let addr = parse_address(&req.params, 0);
            match addr {
                Ok(a) => match chain.get_miner_info(&a) {
                    Some(miner) => serde_json::to_value(miner).map_err(|e| format!("{e}")),
                    None => Ok(Value::Null),
                },
                Err(e) => Err(e),
            }
        }
        "claw_getMiners" => {
            let active = req.params.get(0).and_then(|v| v.as_bool()).unwrap_or(true);
            let limit = req.params.get(1).and_then(|v| v.as_u64()).unwrap_or(100).min(500) as usize;
            let offset = req.params.get(2).and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            serde_json::to_value(chain.get_miners(active, limit, offset)).map_err(|e| format!("{e}"))
        }
        "claw_getMiningStats" => {
            Ok(chain.get_mining_stats())
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

/// Extract the `to` address and `amount` from a transaction payload based on tx_type.
/// Returns (Option<hex_string>, Option<amount_string>).
fn extract_to_and_amount(tx: &claw_types::Transaction) -> (Option<String>, Option<String>) {
    match tx.tx_type {
        TxType::TokenTransfer => {
            match borsh::from_slice::<TokenTransferPayload>(&tx.payload) {
                Ok(p) => (Some(hex::encode(p.to)), Some(p.amount.to_string())),
                Err(_) => (None, None),
            }
        }
        TxType::TokenMintTransfer => {
            match borsh::from_slice::<TokenMintTransferPayload>(&tx.payload) {
                Ok(p) => (Some(hex::encode(p.to)), Some(p.amount.to_string())),
                Err(_) => (None, None),
            }
        }
        TxType::ReputationAttest => {
            match borsh::from_slice::<ReputationAttestPayload>(&tx.payload) {
                Ok(p) => (Some(hex::encode(p.to)), None),
                Err(_) => (None, None),
            }
        }
        TxType::AgentRegister | TxType::TokenCreate | TxType::ServiceRegister
        | TxType::ContractDeploy | TxType::StakeClaim | TxType::PlatformActivityReport => (None, None),
        TxType::TokenApprove => {
            match borsh::from_slice::<TokenApprovePayload>(&tx.payload) {
                Ok(p) => (Some(hex::encode(p.spender)), Some(p.amount.to_string())),
                Err(_) => (None, None),
            }
        }
        TxType::TokenBurn => {
            match borsh::from_slice::<TokenBurnPayload>(&tx.payload) {
                Ok(p) => (None, Some(p.amount.to_string())),
                Err(_) => (None, None),
            }
        }
        TxType::ContractCall => {
            match borsh::from_slice::<claw_types::transaction::ContractCallPayload>(&tx.payload) {
                Ok(p) => (Some(hex::encode(p.contract)), Some(p.value.to_string())),
                Err(_) => (None, None),
            }
        }
        TxType::StakeDeposit => {
            match borsh::from_slice::<claw_types::transaction::StakeDepositPayload>(&tx.payload) {
                Ok(p) => (None, Some(p.amount.to_string())),
                Err(_) => (None, None),
            }
        }
        TxType::StakeWithdraw => {
            match borsh::from_slice::<claw_types::transaction::StakeWithdrawPayload>(&tx.payload) {
                Ok(p) => (None, Some(p.amount.to_string())),
                Err(_) => (None, None),
            }
        }
        TxType::ChangeDelegation
        | TxType::MinerRegister
        | TxType::MinerHeartbeat
        | TxType::ContractUpgradeAnnounce
        | TxType::ContractUpgradeExecute => (None, None),
    }
}

/// Return the human-readable name for a transaction type.
fn tx_type_name(tx_type: claw_types::TxType) -> &'static str {
    match tx_type {
        claw_types::TxType::AgentRegister => "AgentRegister",
        claw_types::TxType::TokenTransfer => "TokenTransfer",
        claw_types::TxType::TokenCreate => "TokenCreate",
        claw_types::TxType::TokenMintTransfer => "TokenMintTransfer",
        claw_types::TxType::ReputationAttest => "ReputationAttest",
        claw_types::TxType::ServiceRegister => "ServiceRegister",
        claw_types::TxType::ContractDeploy => "ContractDeploy",
        claw_types::TxType::ContractCall => "ContractCall",
        claw_types::TxType::StakeDeposit => "StakeDeposit",
        claw_types::TxType::StakeWithdraw => "StakeWithdraw",
        claw_types::TxType::StakeClaim => "StakeClaim",
        claw_types::TxType::PlatformActivityReport => "PlatformActivityReport",
        claw_types::TxType::TokenApprove => "TokenApprove",
        claw_types::TxType::TokenBurn => "TokenBurn",
        claw_types::TxType::ChangeDelegation => "ChangeDelegation",
        claw_types::TxType::MinerRegister => "MinerRegister",
        claw_types::TxType::MinerHeartbeat => "MinerHeartbeat",
        claw_types::TxType::ContractUpgradeAnnounce => "ContractUpgradeAnnounce",
        claw_types::TxType::ContractUpgradeExecute => "ContractUpgradeExecute",
    }
}

/// Extract the recipient address and amount from a transaction payload,
/// based on the transaction type. Returns `(None, None)` for types that
/// have no recipient (AgentRegister, TokenCreate, ServiceRegister).
fn parse_tx_recipient(tx: &claw_types::Transaction) -> (Option<[u8; 32]>, Option<u128>) {
    match tx.tx_type {
        claw_types::TxType::TokenTransfer => {
            // payload = [to: 32 bytes][amount: 16 bytes u128 LE]
            if tx.payload.len() >= 48 {
                let to: [u8; 32] = tx.payload[..32].try_into().unwrap();
                let amount = u128::from_le_bytes(tx.payload[32..48].try_into().unwrap());
                (Some(to), Some(amount))
            } else {
                (None, None)
            }
        }
        claw_types::TxType::TokenMintTransfer => {
            // payload = [tokenId: 32 bytes][to: 32 bytes][amount: 16 bytes u128 LE]
            if tx.payload.len() >= 80 {
                let to: [u8; 32] = tx.payload[32..64].try_into().unwrap();
                let amount = u128::from_le_bytes(tx.payload[64..80].try_into().unwrap());
                (Some(to), Some(amount))
            } else {
                (None, None)
            }
        }
        claw_types::TxType::ReputationAttest => {
            // payload starts with [to: 32 bytes]
            if tx.payload.len() >= 32 {
                let to: [u8; 32] = tx.payload[..32].try_into().unwrap();
                (Some(to), None)
            } else {
                (None, None)
            }
        }
        claw_types::TxType::StakeDeposit => {
            // payload = [amount: 16 bytes u128 LE][validator: 32 bytes][commission_bps: 2 bytes]
            if tx.payload.len() >= 48 {
                let amount = u128::from_le_bytes(tx.payload[..16].try_into().unwrap());
                let validator: [u8; 32] = tx.payload[16..48].try_into().unwrap();
                let to = if validator == [0u8; 32] { None } else { Some(validator) };
                (to, Some(amount))
            } else {
                (None, None)
            }
        }
        claw_types::TxType::StakeWithdraw => {
            // payload = [amount: 16 bytes u128 LE]
            if tx.payload.len() >= 16 {
                let amount = u128::from_le_bytes(tx.payload[..16].try_into().unwrap());
                (None, Some(amount))
            } else {
                (None, None)
            }
        }
        claw_types::TxType::AgentRegister
        | claw_types::TxType::TokenCreate
        | claw_types::TxType::ServiceRegister
        | claw_types::TxType::ContractDeploy
        | claw_types::TxType::StakeClaim
        | claw_types::TxType::PlatformActivityReport
        | claw_types::TxType::TokenApprove
        | claw_types::TxType::TokenBurn
        | claw_types::TxType::ChangeDelegation
        | claw_types::TxType::MinerRegister
        | claw_types::TxType::MinerHeartbeat
        | claw_types::TxType::ContractUpgradeAnnounce
        | claw_types::TxType::ContractUpgradeExecute => (None, None),
        claw_types::TxType::ContractCall => {
            // payload starts with [contract: 32 bytes]
            if tx.payload.len() >= 32 {
                let contract: [u8; 32] = tx.payload[..32].try_into().unwrap();
                (Some(contract), None)
            } else {
                (None, None)
            }
        }
    }
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
    let chain_id = chain.get_chain_id();
    let genesis_hash = chain.get_genesis_hash();

    // Consider unhealthy if last block is older than 60 seconds
    let block_age = now.saturating_sub(last_block_ts);
    let status = if block_age < 60 || height == 0 { "ok" } else { "degraded" };

    Json(serde_json::json!({
        "status": status,
        "version": env!("CARGO_PKG_VERSION"),
        "chain_id": chain_id,
        "genesis_hash": genesis_hash,
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

    let cors = {
        let allow_origin = match std::env::var("CLAW_RPC_CORS_ORIGINS") {
            Ok(val) if val.trim() == "*" => AllowOrigin::any(),
            Ok(val) if !val.is_empty() => {
                let origins: Vec<axum::http::HeaderValue> = val
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();
                AllowOrigin::list(origins)
            }
            // Default: localhost only (safe default for production).
            // Set CLAW_RPC_CORS_ORIGINS=* for any origin, or comma-separated list.
            _ => AllowOrigin::list([
                "http://localhost:3000".parse().unwrap(),
                "http://localhost:9710".parse().unwrap(),
                "http://127.0.0.1:3000".parse().unwrap(),
                "http://127.0.0.1:9710".parse().unwrap(),
            ]),
        };
        CorsLayer::new()
            .allow_origin(allow_origin)
            .allow_methods([Method::GET, Method::POST])
            .allow_headers(Any)
    };

    let app = Router::new()
        .route("/", post(handle_rpc))
        .route("/metrics", get(handle_metrics))
        .route("/health", get(handle_health))
        .layer(ConcurrencyLimitLayer::new(100)) // max 100 concurrent requests
        .layer(cors)
        .with_state(chain);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    let handle = tokio::spawn(async move {
        axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await.ok();
    });

    Ok(handle)
}
