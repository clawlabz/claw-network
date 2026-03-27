use anyhow::{bail, Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

use crate::tx::SignedTransaction;

// ---- JSON-RPC request/response types ----

#[derive(Serialize)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'a str,
    method: &'a str,
    params: Value,
    id: u64,
}

#[derive(Deserialize, Debug)]
struct JsonRpcResponse {
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<Value>,
}

fn make_client() -> Result<Client> {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("building HTTP client")
}

/// Fetch the current nonce for an account address (hex-encoded public key).
/// Returns 0 if the account has no transactions yet.
pub async fn fetch_nonce(rpc_url: &str, pubkey_hex: &str) -> Result<u64> {
    let client = make_client()?;

    let request = JsonRpcRequest {
        jsonrpc: "2.0",
        method: "claw_getNonce",
        params: json!([pubkey_hex]),
        id: 1,
    };

    let response: JsonRpcResponse = client
        .post(rpc_url)
        .json(&request)
        .send()
        .await
        .context("sending get_nonce request")?
        .json()
        .await
        .context("parsing get_nonce response")?;

    if let Some(err) = response.error {
        bail!("RPC error fetching nonce: {err}");
    }

    let nonce = response.result.and_then(|v| v.as_u64()).unwrap_or(0);
    Ok(nonce)
}

/// Submit a signed transaction to the chain via JSON-RPC.
/// Returns the hex-encoded transaction hash.
pub async fn submit_transaction(rpc_url: &str, tx: &SignedTransaction) -> Result<String> {
    let client = make_client()?;

    // Serialize the transaction to borsh, then hex-encode
    let tx_bytes = serialize_transaction_for_rpc(tx);
    let tx_hex = hex::encode(&tx_bytes);

    let request = JsonRpcRequest {
        jsonrpc: "2.0",
        method: "claw_sendTransaction",
        params: json!([tx_hex]),
        id: 2,
    };

    let response: JsonRpcResponse = client
        .post(rpc_url)
        .json(&request)
        .send()
        .await
        .context("sending submit_transaction request")?
        .json()
        .await
        .context("parsing submit_transaction response")?;

    if let Some(err) = response.error {
        bail!("RPC error submitting transaction: {err}");
    }

    // Node may return bare string hash or {"hash": "..."} object
    let result = response.result.context("submit_transaction response missing result")?;

    let tx_hash = if let Some(s) = result.as_str() {
        s.to_string()
    } else if let Some(h) = result.get("hash").and_then(|h| h.as_str()) {
        h.to_string()
    } else {
        bail!("submit_transaction response: could not extract tx hash from: {result}");
    };

    Ok(tx_hash)
}

/// Poll the node every 2 seconds for up to `max_seconds` until the transaction
/// appears in a block (confirmed).
pub async fn poll_confirmation(rpc_url: &str, tx_hash: &str, max_seconds: u64) -> Result<()> {
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("building HTTP client")?;

    let attempts = max_seconds / 2;
    for _ in 0..attempts {
        tokio::time::sleep(Duration::from_secs(2)).await;

        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            method: "claw_getTransactionByHash",
            params: json!([tx_hash]),
            id: 3,
        };

        let result = client
            .post(rpc_url)
            .json(&request)
            .send()
            .await;

        if let Ok(resp) = result {
            if let Ok(body) = resp.json::<JsonRpcResponse>().await {
                if body.error.is_none() {
                    if let Some(data) = body.result {
                        let confirmed = data.get("block_height").is_some()
                            || data
                                .get("status")
                                .and_then(|s| s.as_str())
                                == Some("confirmed");
                        if confirmed {
                            return Ok(());
                        }
                    }
                }
            }
        }

        print!(".");
        use std::io::Write;
        std::io::stdout().flush().ok();
    }

    println!();
    bail!("transaction not confirmed after {max_seconds}s: {tx_hash}");
}

/// Contract info returned by `claw_getContractInfo`.
#[derive(Debug)]
pub struct ContractInfo {
    pub address: String,
    pub code_hash: String,
    pub creator: String,
    pub deployed_at: u64,
}

/// Fetch contract info (address, codeHash, creator, deployedAt) via RPC.
pub async fn fetch_contract_info(rpc_url: &str, address_hex: &str) -> Result<ContractInfo> {
    let client = make_client()?;

    let request = JsonRpcRequest {
        jsonrpc: "2.0",
        method: "claw_getContractInfo",
        params: json!([address_hex]),
        id: 10,
    };

    let response: JsonRpcResponse = client
        .post(rpc_url)
        .json(&request)
        .send()
        .await
        .context("sending claw_getContractInfo request")?
        .json()
        .await
        .context("parsing claw_getContractInfo response")?;

    if let Some(err) = response.error {
        bail!("RPC error fetching contract info: {err}");
    }

    let result = response.result.context("claw_getContractInfo returned no result")?;
    if result.is_null() {
        bail!("contract not found at address {address_hex}");
    }

    let code_hash = result
        .get("codeHash")
        .and_then(|v| v.as_str())
        .context("codeHash missing from response")?
        .to_string();
    let address = result
        .get("address")
        .and_then(|v| v.as_str())
        .unwrap_or(address_hex)
        .to_string();
    let creator = result
        .get("creator")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let deployed_at = result
        .get("deployedAt")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    Ok(ContractInfo {
        address,
        code_hash,
        creator,
        deployed_at,
    })
}

/// Fetch contract Wasm bytecode via `claw_getContractCode`.
/// Returns the raw bytes of the deployed Wasm.
pub async fn fetch_contract_code(rpc_url: &str, address_hex: &str) -> Result<Vec<u8>> {
    let client = make_client()?;

    let request = JsonRpcRequest {
        jsonrpc: "2.0",
        method: "claw_getContractCode",
        params: json!([address_hex]),
        id: 11,
    };

    let response: JsonRpcResponse = client
        .post(rpc_url)
        .json(&request)
        .send()
        .await
        .context("sending claw_getContractCode request")?
        .json()
        .await
        .context("parsing claw_getContractCode response")?;

    if let Some(err) = response.error {
        bail!("RPC error fetching contract code: {err}");
    }

    let result = response.result.context("claw_getContractCode returned no result")?;
    if result.is_null() {
        bail!("contract code not found at address {address_hex}");
    }

    let code_hex = result
        .get("code")
        .and_then(|v| v.as_str())
        .context("code field missing from response")?;

    let code_bytes = hex::decode(code_hex).context("invalid hex in contract code response")?;
    Ok(code_bytes)
}

/// Borsh-serialize a SignedTransaction in the format the node expects.
///
/// Node Transaction struct layout (borsh):
///   tx_type: u8   (discriminant from #[borsh(use_discriminant = true)])
///   from: [u8; 32]
///   nonce: u64
///   payload: Vec<u8>
///   signature: [u8; 64]
pub fn serialize_transaction_for_rpc(tx: &SignedTransaction) -> Vec<u8> {
    use borsh::BorshSerialize;

    #[derive(BorshSerialize)]
    struct TransactionBorsh<'a> {
        tx_type: u8,
        from: [u8; 32],
        nonce: u64,
        payload: &'a [u8],
        signature: [u8; 64],
    }

    let t = TransactionBorsh {
        tx_type: tx.tx_type as u8,
        from: tx.from,
        nonce: tx.nonce,
        payload: &tx.payload,
        signature: tx.signature,
    };

    borsh::to_vec(&t).expect("transaction serialization cannot fail")
}
