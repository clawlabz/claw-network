//! JSON-RPC server for ClawNetwork.
//!
//! HTTP + WebSocket RPC endpoints for querying chain state
//! and submitting transactions.
//!
//! Includes:
//! - Per-IP rate limiting (default 100 req/s)
//! - Max request body size (256 KB)
//! - Input validation on all RPC methods

use axum::{
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tower_http::cors::CorsLayer;

// --- Constants ---

/// Maximum request body size in bytes (2 MB — hex-encoded 512KB wasm ≈ 1MB hex + overhead).
pub const MAX_REQUEST_BODY_SIZE: usize = 2 * 1024 * 1024;

/// Maximum RPC requests per second per IP.
pub const RATE_LIMIT_PER_SECOND: u32 = 100;

/// Maximum hex-encoded address length (64 hex chars = 32 bytes).
pub const MAX_ADDRESS_HEX_LEN: usize = 64;

/// Maximum transaction JSON size submitted via RPC (1.5 MB — hex-encoded 512KB wasm ≈ 1MB + overhead).
pub const MAX_TX_RPC_SIZE: usize = 1536 * 1024;

// --- Rate limiter ---

/// Simple in-memory per-IP rate limiter using a sliding window.
#[derive(Debug, Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<HashMap<std::net::IpAddr, RateState>>>,
    max_per_second: u32,
}

#[derive(Debug)]
struct RateState {
    count: u32,
    window_start: Instant,
}

impl RateLimiter {
    pub fn new(max_per_second: u32) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            max_per_second,
        }
    }

    /// Returns true if the request is allowed, false if rate-limited.
    pub fn check(&self, ip: std::net::IpAddr) -> bool {
        let mut map = self.inner.lock().unwrap();
        let now = Instant::now();

        let state = map.entry(ip).or_insert(RateState {
            count: 0,
            window_start: now,
        });

        // Reset window if more than 1 second has passed
        if now.duration_since(state.window_start).as_secs() >= 1 {
            state.count = 0;
            state.window_start = now;
        }

        if state.count >= self.max_per_second {
            false
        } else {
            state.count += 1;
            true
        }
    }

    /// Periodically clean up stale entries (call from a background task).
    pub fn cleanup(&self) {
        let mut map = self.inner.lock().unwrap();
        let now = Instant::now();
        map.retain(|_, state| now.duration_since(state.window_start).as_secs() < 60);
    }
}

/// Axum middleware that enforces per-IP rate limiting.
pub async fn rate_limit_middleware(
    State(limiter): State<RateLimiter>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    if !limiter.check(addr.ip()) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(RpcError {
                error: format!("Rate limit exceeded: max {} requests/second", limiter.max_per_second),
            }),
        )
            .into_response();
    }
    next.run(request).await
}

// --- Validation helpers ---

/// Validate a hex-encoded address string (must be exactly 64 hex chars = 32 bytes).
pub fn validate_address_hex(hex_str: &str) -> Result<[u8; 32], String> {
    if hex_str.len() != MAX_ADDRESS_HEX_LEN {
        return Err(format!(
            "address must be exactly {} hex characters, got {}",
            MAX_ADDRESS_HEX_LEN,
            hex_str.len()
        ));
    }
    let bytes = hex::decode(hex_str).map_err(|e| format!("invalid hex: {e}"))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "address must decode to exactly 32 bytes".to_string())?;
    Ok(arr)
}

/// Validate that a serialized transaction doesn't exceed the max size.
pub fn validate_tx_size(tx_json: &[u8]) -> Result<(), String> {
    if tx_json.len() > MAX_TX_RPC_SIZE {
        return Err(format!(
            "transaction too large: {} bytes (max {})",
            tx_json.len(),
            MAX_TX_RPC_SIZE
        ));
    }
    Ok(())
}

// --- RPC types ---

#[derive(Debug, Serialize, Deserialize)]
pub struct RpcError {
    pub error: String,
}

/// Build the RPC router with rate limiting and body size limit.
pub fn build_router() -> Router {
    let limiter = RateLimiter::new(RATE_LIMIT_PER_SECOND);

    Router::new()
        .route("/health", get(health_handler))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_REQUEST_BODY_SIZE))
        .layer(middleware::from_fn_with_state(limiter, rate_limit_middleware))
        .layer(CorsLayer::permissive())
}

async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limiter_allows_under_limit() {
        let limiter = RateLimiter::new(5);
        let ip: std::net::IpAddr = "127.0.0.1".parse().unwrap();
        for _ in 0..5 {
            assert!(limiter.check(ip));
        }
        // 6th should be blocked
        assert!(!limiter.check(ip));
    }

    #[test]
    fn rate_limiter_different_ips_independent() {
        let limiter = RateLimiter::new(2);
        let ip1: std::net::IpAddr = "1.2.3.4".parse().unwrap();
        let ip2: std::net::IpAddr = "5.6.7.8".parse().unwrap();
        assert!(limiter.check(ip1));
        assert!(limiter.check(ip1));
        assert!(!limiter.check(ip1)); // ip1 exhausted
        assert!(limiter.check(ip2)); // ip2 still has budget
    }

    #[test]
    fn validate_address_hex_valid() {
        let hex_str = "a".repeat(64);
        assert!(validate_address_hex(&hex_str).is_ok());
    }

    #[test]
    fn validate_address_hex_wrong_length() {
        assert!(validate_address_hex("abcd").is_err());
    }

    #[test]
    fn validate_address_hex_invalid_chars() {
        let hex_str = "g".repeat(64);
        assert!(validate_address_hex(&hex_str).is_err());
    }

    #[test]
    fn validate_tx_size_within_limit() {
        let data = vec![0u8; MAX_TX_RPC_SIZE];
        assert!(validate_tx_size(&data).is_ok());
    }

    #[test]
    fn validate_tx_size_exceeds_limit() {
        let data = vec![0u8; MAX_TX_RPC_SIZE + 1];
        assert!(validate_tx_size(&data).is_err());
    }
}
