# ClawNetwork Contract Templates

> Use `reward-vault` and `arena-pool` as starting points for your own contracts.

---

## Quick Start

```bash
# 1. Copy a template
cp -r ../reward-vault ./my-contract
cd my-contract

# 2. Update Cargo.toml
sed -i '' 's/name = "reward-vault"/name = "my-contract"/' Cargo.toml

# 3. Build
cargo build --target wasm32-unknown-unknown --release

# 4. Deploy
../../scripts/deploy-contract.sh my-contract.wasm init '{"owner":"<hex>"}' --network testnet
```

---

## Available Templates

### reward-vault (Reward Distribution)

Best for: platform-to-user CLAW distribution, activity rewards, airdrops.

Key features:
- Platform calls `claim_reward` on behalf of users (users never sign claim txs)
- Per-recipient monotonic nonce prevents replay attacks
- Daily cap per recipient enforced on-chain (UTC day from block timestamp)
- Owner can add/remove authorized platform callers
- Pause/unpause emergency circuit breaker
- `fund()` payable method to replenish the vault

Entry points:
| Method | Caller | Payable | Description |
|--------|--------|---------|-------------|
| `init` | deployer (once) | No | Set owner, daily cap, min games, authorized platforms |
| `claim_reward` | authorized platform | No | Transfer CLAW to a recipient (nonce + daily cap enforced) |
| `add_platform` | owner | No | Authorize a new platform agent address |
| `remove_platform` | owner | No | Revoke a platform agent's authorization |
| `fund` | anyone | Yes | Deposit CLAW into the vault balance |
| `withdraw` | owner | No | Pull CLAW from the vault to the owner |
| `pause` / `unpause` | owner | No | Emergency circuit breaker |
| `cleanup_claims` | owner | No | Remove old claim records to reduce storage |

### arena-pool (Escrow / Game Wallet)

Best for: competitive games, prediction markets, tournaments, any scenario requiring pre-deposit escrow.

Key features:
- Users deposit their own CLAW upfront
- Platform agent controls locking and settlement
- Emergency refund timeout (configurable) provides safety valve
- Fee and burn percentages fixed at init (immutable)
- Conservation invariant: `sum(payouts) + fee + burn == total_pool`

Entry points:
| Method | Caller | Payable | Description |
|--------|--------|---------|-------------|
| `init` | deployer (once) | No | Set owner, platform agent, fee/burn BPS |
| `deposit` | user | Yes | Deposit CLAW into the user's pool balance |
| `withdraw` | user | No | Withdraw CLAW from pool back to wallet |
| `lock_entries` | platform agent | No | Lock entry fees before a match begins |
| `settle_game` | platform agent | No | Distribute pool to winners after match ends |
| `refund_game` | platform agent | No | Refund all players if match is cancelled |
| `refund_game_emergency` | anyone | No | Refund after timeout (~1 hour) -- safety valve |
| `claim_fees` | platform agent | No | Transfer accumulated fees to platform agent |
| `pause` / `unpause` | owner | No | Emergency circuit breaker |
| `cleanup_games` | owner | No | Remove settled/refunded game records |

---

## Contract Project Structure

Every contract follows this layout:

```
my-contract/
  Cargo.toml          # Package config, crate-type = ["cdylib", "rlib"]
  src/
    lib.rs            # WASM entry points (thin wrappers, no business logic)
    types.rs          # Borsh-serializable argument and return types
    logic.rs          # Pure business logic (testable without a VM)
    mock.rs           # (optional) Mock host environment for unit tests
  tests/
    integration.rs    # Integration tests using the mock environment
```

### Why this separation matters

- **lib.rs** contains only `#[no_mangle] pub extern "C" fn` entry points that deserialize args, call logic functions, and set return data. It compiles to WASM only (`#[cfg(target_arch = "wasm32")]`).
- **logic.rs** contains all business logic as pure functions. These run on the native target during `cargo test`, making them fast and debuggable.
- **types.rs** defines all Borsh-serializable structs shared between entry points and logic. Keep these small and versioned.
- **mock.rs** stubs the `claw_sdk::env` host functions so logic tests can run without the VM.

---

## Cargo.toml Template

```toml
[package]
name = "my-contract"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]
# cdylib: produces .wasm for the VM
# rlib: allows `cargo test` to run natively

[dependencies]
claw-sdk = { path = "../../claw-node/crates/sdk" }
borsh = { version = "1", features = ["derive"] }

[dev-dependencies]
# Integration tests run natively (not wasm).
# Mock host functions in tests/integration.rs or src/mock.rs.

[profile.release]
opt-level = "z"       # Optimize for size (smaller wasm)
lto = true            # Link-time optimization
codegen-units = 1     # Single codegen unit for best optimization
panic = "abort"       # No unwinding in wasm
```

---

## Entry Point Boilerplate (src/lib.rs)

```rust
extern crate alloc;

pub mod logic;
pub mod types;

#[cfg(test)]
pub mod mock;

#[cfg(target_arch = "wasm32")]
mod wasm_entry {
    use super::logic;
    use super::types::*;
    use claw_sdk::env;

    claw_sdk::setup_alloc!();

    #[no_mangle]
    pub extern "C" fn init(args_ptr: i32, args_len: i32) {
        claw_sdk::entry!(args_ptr, args_len, |args: InitArgs| {
            let caller = env::get_caller();
            logic::apply_init(caller, args);
            b"ok".to_vec()
        });
    }

    #[no_mangle]
    pub extern "C" fn my_method(args_ptr: i32, args_len: i32) {
        claw_sdk::entry!(args_ptr, args_len, |args: MyMethodArgs| {
            let caller = env::get_caller();
            claw_sdk::require!(args.amount > 0, "amount must be positive");
            logic::apply_my_method(caller, args);
            b"ok".to_vec()
        });
    }
}
```

---

## Types Template (src/types.rs)

```rust
use borsh::{BorshDeserialize, BorshSerialize};

// Storage keys as byte slices for consistency
pub const KEY_VERSION: &[u8] = b"version";
pub const KEY_OWNER: &[u8] = b"owner";
pub const KEY_PAUSED: &[u8] = b"paused";

#[derive(BorshSerialize, BorshDeserialize)]
pub struct InitArgs {
    pub owner: [u8; 32],
    pub some_config: u64,
}

#[derive(BorshSerialize, BorshDeserialize)]
pub struct MyMethodArgs {
    pub amount: u128,
    pub recipient: [u8; 32],
}
```

---

## Logic Template (src/logic.rs)

```rust
use crate::types::*;
use claw_sdk::{env, storage};

pub fn apply_init(caller: [u8; 32], args: InitArgs) {
    // Guard: only callable once
    if env::storage_exists(KEY_VERSION) {
        env::panic_msg("already initialized");
    }

    storage::set_u64(KEY_VERSION, 1);
    storage::set(KEY_OWNER, &args.owner);
    storage::set(KEY_PAUSED, &false);

    env::log(&format!("Contract initialized by {:?}", hex::encode(caller)));
}

pub fn apply_my_method(caller: [u8; 32], args: MyMethodArgs) {
    // Guard: not paused
    let paused: bool = storage::get(KEY_PAUSED).unwrap_or(false);
    if paused {
        env::panic_msg("contract is paused");
    }

    // Guard: authorized caller
    let owner: [u8; 32] = storage::get(KEY_OWNER).expect("not initialized");
    if caller != owner {
        env::panic_msg("unauthorized");
    }

    // Business logic here
    let success = env::transfer(args.recipient, args.amount);
    if !success {
        env::panic_msg("transfer failed: insufficient contract balance");
    }
}
```

---

## Build Instructions

### Prerequisites

```bash
# Install Rust wasm32 target
rustup target add wasm32-unknown-unknown

# (Optional) Install wasm-opt for size optimization
cargo install wasm-opt
```

### Build a single contract

```bash
cd contracts/my-contract
cargo build --target wasm32-unknown-unknown --release

# Output: target/wasm32-unknown-unknown/release/my_contract.wasm
```

### Optimize for size (recommended for mainnet)

```bash
wasm-opt -O3 \
  target/wasm32-unknown-unknown/release/my_contract.wasm \
  -o my_contract_optimized.wasm

# Typical sizes:
#   reward-vault: ~45 KB (unoptimized) -> ~28 KB (optimized)
#   arena-pool:   ~55 KB (unoptimized) -> ~32 KB (optimized)
```

### Build all contracts

```bash
./scripts/build-contracts.sh
```

### Run tests

```bash
# Run all tests for a specific contract
cd contracts/my-contract
cargo test

# Run with output for debugging
cargo test -- --nocapture

# Run a specific test
cargo test test_init_sets_version
```

---

## claw-contract CLI Usage

The `claw-contract` CLI (if installed) provides a higher-level interface:

```bash
# Create a new contract project from template
claw-contract new my-contract --template reward-vault

# Build (wraps cargo build + wasm-opt)
claw-contract build

# Deploy to testnet
claw-contract deploy \
  --wasm target/wasm32-unknown-unknown/release/my_contract.wasm \
  --init-method init \
  --init-args '{"owner":"<hex>","some_config":100}' \
  --network testnet \
  --signer <private-key-hex>

# Call a method on a deployed contract
claw-contract call \
  --contract <contract-address-hex> \
  --method my_method \
  --args '{"amount":1000000000,"recipient":"<hex>"}' \
  --network testnet \
  --signer <private-key-hex>
```

If the CLI is not installed, use the deploy script directly:

```bash
./scripts/deploy-contract.sh <wasm_path> <init_method> <init_args> [--network testnet]
```

---

## Host Functions Reference

These are available to your contract via `claw_sdk::env` and `claw_sdk::storage`:

| Function | Description |
|----------|-------------|
| `env::get_caller()` | 32-byte address of the transaction sender |
| `env::get_contract_address()` | This contract's own 32-byte address |
| `env::get_block_height()` | Current block number |
| `env::get_block_timestamp()` | Current block Unix timestamp (seconds) |
| `env::get_value()` | CLAW sent with this payable call (u128 base units) |
| `env::get_balance(addr)` | CLAW balance of any address (u128) |
| `env::transfer(to, amount)` | Transfer CLAW from contract to `to`; returns bool |
| `env::is_agent_registered(addr)` | True if `addr` is a registered agent |
| `env::get_agent_score(addr)` | Agent Score 0-100 |
| `storage::get(key)` | Read typed value from contract KV store |
| `storage::set(key, value)` | Write typed value to contract KV store |
| `env::storage_exists(key)` | Check if a key exists |
| `env::storage_remove(key)` | Delete a key |
| `env::log(msg)` | Emit a log string (visible in tx receipts) |
| `env::set_return_data(data)` | Set bytes returned to the caller |
| `env::panic_msg(msg)` | Abort execution with error (reverts state) |

Convenience wrappers: `storage::get_u64`, `storage::get_u128`, `storage::set_u64`, `storage::set_u128`.

---

## Common Patterns

### Owner-only guard

```rust
fn require_owner(caller: [u8; 32]) {
    let owner: [u8; 32] = storage::get(KEY_OWNER).expect("not initialized");
    if caller != owner {
        env::panic_msg("unauthorized: owner only");
    }
}
```

### Pause guard

```rust
fn require_not_paused() {
    let paused: bool = storage::get(KEY_PAUSED).unwrap_or(false);
    if paused {
        env::panic_msg("contract is paused");
    }
}
```

### Per-user balance tracking

```rust
fn get_balance(addr: &[u8; 32]) -> u128 {
    let key = format!("bal:{}", hex::encode(addr));
    storage::get_u128(key.as_bytes()).unwrap_or(0)
}

fn set_balance(addr: &[u8; 32], amount: u128) {
    let key = format!("bal:{}", hex::encode(addr));
    storage::set_u128(key.as_bytes(), amount);
}
```

### Monotonic nonce (replay protection)

```rust
fn get_nonce(addr: &[u8; 32]) -> u64 {
    let key = format!("nonce:{}", hex::encode(addr));
    storage::get_u64(key.as_bytes()).unwrap_or(0)
}

fn increment_nonce(addr: &[u8; 32]) -> u64 {
    let current = get_nonce(addr);
    let next = current + 1;
    let key = format!("nonce:{}", hex::encode(addr));
    storage::set_u64(key.as_bytes(), next);
    next
}
```

---

## Versioning and Upgrades

Contracts are immutable once deployed. For upgrade procedures, see [docs/contract-versioning.md](../../docs/contract-versioning.md).

Key points:
- Store a `version` field at init time to identify deployed versions
- Use `pause()` on the old contract before migrating
- Deploy the new version as a fresh contract (new address)
- Update backend environment variables to point to the new address
- User funds in the old contract must be withdrawn by users (Arena Pool) or pulled by the owner (Reward Vault)
