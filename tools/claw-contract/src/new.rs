use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;

/// Validate a contract project name.
/// Must be non-empty and contain no whitespace.
fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("contract name must not be empty");
    }
    if name.chars().any(|c| c.is_whitespace()) {
        bail!("contract name must not contain whitespace: {:?}", name);
    }
    Ok(())
}

/// Create a new contract project scaffold in `parent_dir/<name>/`.
///
/// Directory structure:
/// ```
/// <name>/
/// ├── Cargo.toml   (with claw-sdk dependency, wasm target)
/// ├── src/
/// │   └── lib.rs   (Hello World contract: init + get/set)
/// └── README.md
/// ```
pub fn create_contract_project(parent_dir: &Path, name: &str) -> Result<()> {
    validate_name(name)?;

    let project_dir = parent_dir.join(name);
    if project_dir.exists() {
        bail!(
            "directory already exists: {}",
            project_dir.display()
        );
    }

    // Create directory tree
    let src_dir = project_dir.join("src");
    fs::create_dir_all(&src_dir)
        .with_context(|| format!("creating project directory: {}", project_dir.display()))?;

    // Write Cargo.toml
    let cargo_toml = cargo_toml_content(name);
    fs::write(project_dir.join("Cargo.toml"), cargo_toml)
        .context("writing Cargo.toml")?;

    // Write src/lib.rs
    fs::write(src_dir.join("lib.rs"), lib_rs_content())
        .context("writing src/lib.rs")?;

    // Write README.md
    fs::write(project_dir.join("README.md"), readme_content(name))
        .context("writing README.md")?;

    Ok(())
}

fn cargo_toml_content(name: &str) -> String {
    format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
claw-sdk = {{ git = "https://github.com/clawlabz/claw-network", package = "claw-sdk" }}
borsh = {{ version = "1", features = ["derive"] }}

[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
"#
    )
}

fn lib_rs_content() -> &'static str {
    r#"//! Hello World contract for ClawNetwork.
//!
//! Demonstrates a simple key-value store with:
//!   - `init`: initializes the contract owner
//!   - `set`:  stores a value under the key "data"
//!   - `get`:  retrieves the stored "data" value

use claw_sdk::{env, storage};

claw_sdk::setup_alloc!();

/// Initialize the contract: store the caller as the owner.
#[no_mangle]
pub extern "C" fn init() {
    let caller = env::get_caller();
    storage::set(b"owner", &caller);
    env::log("contract initialized");
}

/// Set the "data" key to the provided value.
/// Only the contract owner may call this.
#[no_mangle]
pub extern "C" fn set(args_ptr: i32, args_len: i32) {
    claw_sdk::entry!(args_ptr, args_len, |value: Vec<u8>| {
        let caller = env::get_caller();
        let owner = storage::get(b"owner").expect("owner not set");
        if caller != owner.as_slice() {
            env::panic_msg("only owner can set data");
        }
        storage::set(b"data", &value);
        env::log("data updated");
        b"ok".to_vec()
    });
}

/// Get the current value stored under "data".
#[no_mangle]
pub extern "C" fn get() {
    match storage::get(b"data") {
        Some(value) => env::set_return_data(&value),
        None => env::set_return_data(b""),
    }
}
"#
}

fn readme_content(name: &str) -> String {
    format!(
        r#"# {name}

A ClawNetwork smart contract.

## Build

```bash
cargo build --target wasm32-unknown-unknown --release
```

## Deploy

```bash
claw-contract deploy target/wasm32-unknown-unknown/release/{name}.wasm \
  --method init \
  --args 0x \
  --rpc https://testnet-rpc.clawlabz.xyz \
  --key <your-private-key-hex>
```

## Call

```bash
claw-contract call <contract-address> get \
  --args 0x \
  --rpc https://testnet-rpc.clawlabz.xyz \
  --key <your-private-key-hex>
```
"#
    )
}
