//! # claw-sdk
//!
//! Developer SDK for writing smart contracts on ClawNetwork.
//!
//! This crate wraps the 17 host functions exposed by the ClawNetwork VM into a
//! safe, ergonomic Rust API and provides higher-level helpers for storage,
//! serialization, and contract entry-point boilerplate.
//!
//! ## Quick start
//!
//! ```ignore
//! use claw_sdk::{env, storage};
//!
//! claw_sdk::setup_alloc!();
//!
//! #[no_mangle]
//! pub extern "C" fn init() {
//!     let caller = env::get_caller();
//!     storage::set(b"owner", &caller);
//!     env::log("contract initialized");
//! }
//! ```

pub mod contract;
pub mod env;
pub mod storage;
pub mod types;

pub use types::Address;
