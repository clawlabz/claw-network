# ClawNetwork Smart Contract Developer Guide

This guide covers everything you need to write, build, deploy, and interact with smart contracts on ClawNetwork. No prior ClawNetwork experience is assumed.

---

## Table of Contents

1. [Overview](#overview)
2. [Quick Start](#quick-start)
3. [SDK Reference](#sdk-reference)
   - [Macros](#macros)
   - [Host Functions (env module)](#host-functions-env-module)
   - [Typed Storage Helpers (storage module)](#typed-storage-helpers-storage-module)
   - [Address Type](#address-type)
4. [Fuel Costs](#fuel-costs)
5. [Building Contracts](#building-contracts)
6. [Deploying Contracts](#deploying-contracts)
7. [Calling Contracts](#calling-contracts)
8. [Reading Contract State](#reading-contract-state)
9. [Security Best Practices](#security-best-practices)
10. [Limitations](#limitations)
11. [Testing Contracts](#testing-contracts)
12. [Example Contracts](#example-contracts)
    - [Reward Vault](#reward-vault)
    - [Arena Pool](#arena-pool)
13. [Appendix: Transaction Types](#appendix-transaction-types)

---

## Overview

ClawNetwork smart contracts are WebAssembly (Wasm) modules compiled from Rust. Key properties:

- **Language**: Rust, compiled to `wasm32-unknown-unknown`
- **SDK**: `claw-sdk` provides macros, host function bindings, and storage helpers
- **Serialization**: Arguments and return values use [Borsh](https://borsh.io/) binary encoding
- **Immutability**: Once deployed, a contract's code cannot be changed. Use a versioning or proxy pattern if upgradability is required
- **Execution model**: Each contract call is a transaction on-chain. The VM deducts fuel per host function call. If fuel runs out, execution is aborted and state changes are rolled back
- **Token**: Contracts hold and transfer CLAW, the native token (9 decimal places, base unit = nano-CLAW)
- **VM**: ClawNetwork uses [Wasmer](https://wasmer.io/) with the `singlepass` compiler for deterministic execution

---

## Quick Start

### Prerequisites

1. **Rust toolchain** (1.75 or later):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

2. **Wasm build target**:

```bash
rustup target add wasm32-unknown-unknown
```

3. **A ClawNetwork RPC endpoint**:
   - Mainnet: `https://rpc.clawlabz.xyz`
   - Testnet: `https://testnet-rpc.clawlabz.xyz`

### Hello World Contract

This minimal contract stores an owner on deployment and exposes a greeting that includes the owner's address.

**Directory structure:**

```
hello-world/
  Cargo.toml
  src/
    lib.rs
```

**`Cargo.toml`:**

```toml
[package]
name = "hello-world"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
claw-sdk = { git = "https://github.com/clawlabz/claw-network", subdir = "claw-node/crates/sdk" }
borsh = { version = "1", features = ["derive"] }

[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
```

**`src/lib.rs`:**

```rust
use borsh::{BorshDeserialize, BorshSerialize};
use claw_sdk::{env, storage};

// Required: sets up the Wasm allocator export expected by the ClawNetwork VM.
claw_sdk::setup_alloc!();

// Argument structs for each entry point — must derive BorshDeserialize.
#[derive(BorshDeserialize, BorshSerialize)]
pub struct InitArgs {
    pub owner: [u8; 32],
}

#[derive(BorshDeserialize, BorshSerialize)]
pub struct EmptyArgs {}

// Entry point: called once at deploy time to initialize the contract.
#[no_mangle]
pub extern "C" fn init(args_ptr: i32, args_len: i32) {
    claw_sdk::entry!(args_ptr, args_len, |args: InitArgs| {
        // Guard against double-initialization.
        claw_sdk::require!(
            !storage::exists(b"owner"),
            "already initialized"
        );
        storage::set(b"owner", &args.owner);
        env::log("hello-world: initialized");
        vec![] // return empty bytes (no return value)
    });
}

// Entry point: a simple getter that returns the owner address.
#[no_mangle]
pub extern "C" fn get_owner(args_ptr: i32, args_len: i32) {
    claw_sdk::entry!(args_ptr, args_len, |_args: EmptyArgs| {
        let owner: [u8; 32] = storage::get(b"owner")
            .expect("contract not initialized");
        borsh::to_vec(&owner).unwrap()
    });
}

// Entry point: update the owner. Only the current owner may call this.
#[no_mangle]
pub extern "C" fn set_owner(args_ptr: i32, args_len: i32) {
    claw_sdk::entry!(args_ptr, args_len, |args: InitArgs| {
        let caller = env::get_caller();
        let owner: [u8; 32] = storage::get(b"owner")
            .expect("contract not initialized");
        claw_sdk::require!(caller == owner, "caller is not the owner");
        storage::set(b"owner", &args.owner);
        env::log("hello-world: owner updated");
        vec![]
    });
}
```

**Build:**

```bash
cargo build --target wasm32-unknown-unknown --release
```

Output: `target/wasm32-unknown-unknown/release/hello_world.wasm`

**Deploy (using `claw_sendTransaction` JSON-RPC):**

See the [Deploying Contracts](#deploying-contracts) section for the full transaction format and signing process.

---

## SDK Reference

The `claw-sdk` crate is the only dependency you need for contract development. It exposes three modules:

| Module | Purpose |
|--------|---------|
| `claw_sdk::env` | Host function wrappers (read chain state, transfer tokens, log) |
| `claw_sdk::storage` | Typed Borsh read/write helpers built on top of `env` |
| `claw_sdk::types` | The `Address` type alias (`[u8; 32]`) |

And three macros exported at the crate root: `setup_alloc!`, `entry!`, and `require!`.

---

### Macros

#### `setup_alloc!()`

**Required in every contract.** Exports an `alloc` function that the ClawNetwork VM uses to copy arguments into the Wasm module's linear memory before calling an entry point.

Call it exactly once, at the top level of your `lib.rs`:

```rust
claw_sdk::setup_alloc!();
```

Without this, the VM cannot pass arguments to your entry points and will fail at call time.

> **Note for `no_std` contracts**: The `setup_alloc!` macro references `std::alloc::alloc`, which is available even on `wasm32-unknown-unknown`. If you are building a `no_std` crate (e.g., using `#![no_std]` with `extern crate alloc`), inline the allocator export manually:
>
> ```rust
> #[no_mangle]
> pub extern "C" fn alloc(size: i32) -> *mut u8 {
>     let layout = core::alloc::Layout::from_size_align(size as usize, 1).unwrap();
>     unsafe { alloc::alloc::alloc(layout) }
> }
> ```

---

#### `entry!(args_ptr, args_len, |args: ArgsType| { ... })`

Deserializes Borsh-encoded arguments, runs the closure, and sets the return data.

**Signature:**

```rust
macro_rules! entry {
    ($ptr:expr, $len:expr, |$args:ident: $ty:ty| $body:block) => { ... }
}
```

**Usage pattern — every entry point looks like this:**

```rust
#[no_mangle]
pub extern "C" fn my_method(args_ptr: i32, args_len: i32) {
    claw_sdk::entry!(args_ptr, args_len, |args: MyArgs| {
        // args is your deserialized struct
        // the block must evaluate to Vec<u8> (the return bytes)
        // return vec![] for void methods
        vec![]
    });
}
```

**Rules:**
- The args type `MyArgs` must implement `BorshDeserialize`
- The block must return `Vec<u8>`. For methods with a return value, borsh-encode it: `borsh::to_vec(&my_value).unwrap()`
- For methods with no return value, return `vec![]`
- If deserialization fails (caller passed wrong args), the macro panics and aborts the transaction

---

#### `require!(condition, "error message")`

Aborts execution with an error message if `condition` is false. Equivalent to an assertion that rolls back all state changes made so far in the current call.

```rust
// Abort if the caller is not the stored owner.
claw_sdk::require!(caller == owner, "caller is not the owner");

// Abort if the amount is zero.
claw_sdk::require!(amount > 0, "amount must be positive");

// Abort if the vault has enough balance.
claw_sdk::require!(balance >= amount, "insufficient vault balance");
```

Internally calls `env::panic_msg(msg)`, which calls the `abort` host function. Execution terminates immediately — no code after `require!` runs on failure.

---

### Host Functions (env module)

All host functions are in `claw_sdk::env`. Each call deducts fuel from the execution budget (see [Fuel Costs](#fuel-costs)).

---

#### `env::get_caller() -> [u8; 32]`

Returns the 32-byte Ed25519 public key of the transaction sender (the address that signed the `ContractCall` transaction).

**Fuel cost:** 5,000

```rust
let caller: [u8; 32] = env::get_caller();
```

Use this for authorization checks:

```rust
let owner: [u8; 32] = storage::get(b"owner").expect("not initialized");
claw_sdk::require!(env::get_caller() == owner, "not the owner");
```

---

#### `env::get_block_height() -> u64`

Returns the current block height.

**Fuel cost:** 5,000

```rust
let height: u64 = env::get_block_height();
env::log(&format!("executing at block {}", height));
```

---

#### `env::get_block_timestamp() -> u64`

Returns the current block timestamp as Unix seconds (seconds since 1970-01-01T00:00:00Z).

**Fuel cost:** 5,000

```rust
let ts: u64 = env::get_block_timestamp();
let day: u64 = ts / 86_400; // UTC day number
```

---

#### `env::get_contract_address() -> [u8; 32]`

Returns this contract's own 32-byte address. Use this to check the contract's own CLAW balance.

**Fuel cost:** 5,000

```rust
let self_addr = env::get_contract_address();
let balance = env::get_balance(&self_addr);
claw_sdk::require!(balance >= payout, "insufficient contract balance");
```

---

#### `env::get_value() -> u128`

Returns the amount of CLAW (in nano-CLAW) attached to the current call via the `ContractCallPayload.value` field. Returns `0` if no value was sent.

**Fuel cost:** 5,000 (two internal calls, 5,000 each, but exposed as one SDK function)

```rust
// In a "payable" method that accepts deposits:
#[no_mangle]
pub extern "C" fn deposit(args_ptr: i32, args_len: i32) {
    claw_sdk::entry!(args_ptr, args_len, |_args: EmptyArgs| {
        let received = env::get_value();
        claw_sdk::require!(received > 0, "must send CLAW to deposit");
        // Record the deposit in storage...
        vec![]
    });
}
```

---

#### `env::get_balance(address: &[u8; 32]) -> u128`

Returns the CLAW balance of any address (in nano-CLAW, i.e. 1 CLAW = 1_000_000_000 nano-CLAW).

**Fuel cost:** 5,000 (two internal calls for lo/hi words, 5,000 each)

```rust
let addr: [u8; 32] = env::get_caller();
let balance: u128 = env::get_balance(&addr);
// 1 CLAW = 1_000_000_000 nano-CLAW
```

---

#### `env::transfer(to: &[u8; 32], amount: u128) -> bool`

Transfers `amount` nano-CLAW from the contract's balance to `to`. Returns `true` on success, `false` if the transfer could not be queued (e.g., `amount == 0`).

**Fuel cost:** 100,000

**Important**: Transfers are collected during execution and applied atomically when the transaction commits. If execution aborts after `transfer` is called (e.g., a subsequent `require!` fails), the transfer is also rolled back.

Always follow the Checks-Effects-Interactions pattern: write state changes to storage **before** calling `transfer`.

```rust
// CORRECT: state written before transfer (CEI pattern)
storage::set_u128(b"claimed", new_total);        // EFFECT
let ok = env::transfer(&recipient, amount);       // INTERACTION
claw_sdk::require!(ok, "token transfer failed");

// WRONG: transfer before writing state (reentrancy risk, not applicable here
// but bad habit)
let ok = env::transfer(&recipient, amount);
storage::set_u128(b"claimed", new_total);
```

---

#### `env::is_agent_registered(address: &[u8; 32]) -> bool`

Returns `true` if `address` is a registered agent on ClawNetwork.

**Fuel cost:** 10,000

```rust
let caller = env::get_caller();
claw_sdk::require!(
    env::is_agent_registered(&caller),
    "caller must be a registered agent"
);
```

---

#### `env::get_agent_score(address: &[u8; 32]) -> u64`

Returns the reputation score (0–100) of a registered agent. Returns `0` for unregistered addresses.

**Fuel cost:** 10,000

```rust
let score = env::get_agent_score(&caller);
claw_sdk::require!(score >= 50, "reputation score too low");
```

---

#### `env::storage_get(key: &[u8]) -> Option<Vec<u8>>`

Reads raw bytes from contract storage for the given key. Returns `None` if the key does not exist or if the value exceeds the 16 KB internal read buffer.

**Fuel cost:** 10,000

Prefer the typed `storage::get<T>()` wrapper unless you need raw bytes.

```rust
// Raw bytes read
let raw: Option<Vec<u8>> = env::storage_get(b"my_key");
```

---

#### `env::storage_set(key: &[u8], value: &[u8])`

Writes raw bytes to contract storage. Keys and values can be any byte sequence up to the 64 KB host buffer limit.

**Fuel cost:** 50,000

Prefer `storage::set<T>()` for typed values.

```rust
// Mark a flag
env::storage_set(b"paused", &[1u8]);
// Clear a flag
env::storage_set(b"paused", &[0u8]);
```

---

#### `env::storage_exists(key: &[u8]) -> bool`

Returns `true` if a key exists in contract storage.

**Fuel cost:** 10,000

```rust
if !storage::exists(b"initialized") {
    // first-run setup
}
```

---

#### `env::storage_remove(key: &[u8])`

Deletes a key from contract storage. No-op if the key does not exist.

**Fuel cost:** 10,000

```rust
env::storage_remove(b"deprecated_key");
```

---

#### `env::log(msg: &str)`

Emits a log message. Messages appear in the node's trace logs (target `claw_vm`) and in the return value of `claw_callContractView` RPC calls. A maximum of 100 log entries are kept per execution; additional calls are silently dropped.

**Fuel cost:** 5,000

```rust
env::log("contract initialized");
env::log(&format!("transferred {} to {:?}", amount, recipient));
```

---

#### `env::panic_msg(msg: &str) -> !`

Aborts execution with an error message. This function never returns. All state changes and pending transfers from the current execution are discarded.

**Note:** Prefer `require!` over calling `panic_msg` directly — `require!` reads more clearly at the call site.

```rust
if something_is_wrong {
    env::panic_msg("something went wrong: detailed reason");
}
```

---

#### `env::set_return_data(data: &[u8])`

Sets the raw return bytes for this execution. The `entry!` macro calls this automatically when your closure returns a non-empty `Vec<u8>`. You do not need to call this directly in normal usage.

**Fuel cost:** 5,000

---

### Typed Storage Helpers (storage module)

`claw_sdk::storage` provides Borsh-based typed read/write wrappers on top of `env::storage_*`. These are the recommended way to read and write structured data.

---

#### `storage::get<T: BorshDeserialize>(key: &[u8]) -> Option<T>`

Reads a value from storage and deserializes it from Borsh. Returns `None` if the key does not exist or deserialization fails.

```rust
use borsh::{BorshDeserialize, BorshSerialize};

#[derive(BorshDeserialize, BorshSerialize)]
struct Config {
    fee_bps: u16,
    paused: bool,
}

let config: Option<Config> = storage::get(b"config");
```

---

#### `storage::set<T: BorshSerialize>(key: &[u8], value: &T)`

Serializes `value` to Borsh and writes it to storage.

```rust
let config = Config { fee_bps: 100, paused: false };
storage::set(b"config", &config);
```

---

#### `storage::remove(key: &[u8])`

Deletes a key from storage. Delegates to `env::storage_remove`.

```rust
storage::remove(b"stale_record");
```

---

#### `storage::exists(key: &[u8]) -> bool`

Returns `true` if the key exists. Delegates to `env::storage_exists`.

```rust
if !storage::exists(b"owner") {
    env::panic_msg("contract not initialized");
}
```

---

#### `storage::get_u64(key: &[u8]) -> Option<u64>`

Reads a `u64` stored as 8 little-endian bytes.

```rust
let nonce: u64 = storage::get_u64(b"nonce").unwrap_or(0);
```

---

#### `storage::set_u64(key: &[u8], value: u64)`

Writes a `u64` as 8 little-endian bytes.

```rust
storage::set_u64(b"nonce", nonce + 1);
```

---

#### `storage::get_u128(key: &[u8]) -> Option<u128>`

Reads a `u128` stored as 16 little-endian bytes.

```rust
let daily_claimed: u128 = storage::get_u128(b"claimed").unwrap_or(0);
```

---

#### `storage::set_u128(key: &[u8], value: u128)`

Writes a `u128` as 16 little-endian bytes.

```rust
storage::set_u128(b"claimed", daily_claimed + amount);
```

---

### Address Type

`claw_sdk::types::Address` is a type alias for `[u8; 32]`, representing a 32-byte Ed25519 public key. It is re-exported as `claw_sdk::Address`.

```rust
use claw_sdk::Address;

let owner: Address = env::get_caller();
storage::set(b"owner", &owner);
```

---

## Fuel Costs

Every host function call deducts fuel from the execution budget. The default limit is **10,000,000 fuel per execution**.

| Operation | Fuel Cost |
|-----------|-----------|
| `storage_get` / `storage_exists` / `storage_remove` | 10,000 |
| `storage_set` | 50,000 |
| `get_caller` | 5,000 |
| `get_block_height` | 5,000 |
| `get_block_timestamp` | 5,000 |
| `get_contract_address` | 5,000 |
| `get_value` (lo + hi combined) | 5,000 |
| `get_balance` (lo + hi combined) | 5,000 |
| `log` | 5,000 |
| `set_return_data` | 5,000 |
| `agent_is_registered` | 10,000 |
| `agent_get_score` | 10,000 |
| `transfer` | 100,000 |

**Budget estimates for common patterns:**

| Pattern | Approximate Fuel |
|---------|-----------------|
| Read 1 storage key + check caller | 15,000 |
| Read 5 storage keys + 1 write | 100,000 |
| 1 token transfer with 3 storage writes | 250,000 |
| 10 token transfers | 1,000,000 |
| Heavy computation with 20 storage ops | ~600,000 |

The 10,000,000 fuel limit is sufficient for complex contract logic. If you are approaching the limit, reduce the number of storage operations and avoid loops over unbounded collections.

---

## Building Contracts

### Cargo.toml Template

```toml
[package]
name = "my-contract"
version = "0.1.0"
edition = "2021"

[lib]
# cdylib = dynamic library for Wasm export
# rlib = Rust library for native unit tests
crate-type = ["cdylib", "rlib"]

[dependencies]
claw-sdk = { git = "https://github.com/clawlabz/claw-network", subdir = "claw-node/crates/sdk" }
borsh = { version = "1", features = ["derive"] }

# Optional: hex encoding for address display
hex = "0.4"

[profile.release]
opt-level = "z"      # optimize for size
lto = true           # link-time optimization
codegen-units = 1    # better optimization, slower compile
panic = "abort"      # smaller binary, no unwinding
```

**Why `crate-type = ["cdylib", "rlib"]`**: The `cdylib` produces the deployable `.wasm` file. The `rlib` allows importing the contract as a library in your native test suite so you can unit-test logic without a Wasm runtime.

### Build Command

```bash
cargo build --target wasm32-unknown-unknown --release
```

### Output Location

```
target/wasm32-unknown-unknown/release/my_contract.wasm
```

Note: hyphens in the package name become underscores in the file name (e.g., `my-contract` → `my_contract.wasm`).

### Build All Contracts (Helper Script)

From the repository root, a convenience script builds all contracts:

```bash
./scripts/build-contracts.sh
```

This iterates over all contracts in `contracts/`, builds each one, and reports the output size.

### Checking Wasm Size

```bash
wc -c target/wasm32-unknown-unknown/release/my_contract.wasm
```

The maximum deployable size is **512 KB**. Typical contracts compile to 50–200 KB with release optimizations. If you exceed the limit, consider:
- Removing unused dependencies
- Setting `opt-level = "z"` (already shown in the template)
- Splitting logic into multiple contracts (no cross-contract calls, but you can split state across independently deployed contracts)

---

## Deploying Contracts

Contract deployment is a `ContractDeploy` transaction (type `6`). The transaction payload is a Borsh-encoded `ContractDeployPayload`.

### ContractDeployPayload Fields

| Field | Type | Description |
|-------|------|-------------|
| `code` | `Vec<u8>` | The compiled `.wasm` bytecode |
| `init_method` | `String` | Name of the constructor entry point (e.g., `"init"`). Use `""` to skip the constructor |
| `init_args` | `Vec<u8>` | Borsh-encoded constructor arguments. Use `vec![]` if `init_method` is `""` |

### Transaction Structure

All transactions share this outer envelope (Borsh-encoded):

```
Transaction {
    tx_type:   u8          // 6 for ContractDeploy
    from:      [u8; 32]    // deployer's Ed25519 public key
    nonce:     u64         // must equal current account nonce + 1
    payload:   Vec<u8>     // borsh(ContractDeployPayload)
    signature: [u8; 64]    // Ed25519 signature over (tx_type || from || nonce || payload)
}
```

The **signed bytes** are: `tx_type (1 byte) || from (32 bytes) || nonce (8 bytes, little-endian) || payload bytes`.

### Getting the Current Nonce

Before building any transaction, fetch the sender's current nonce:

```bash
curl -s https://rpc.clawlabz.xyz \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "claw_getNonce",
    "params": ["<your-address-hex>"]
  }'
# Response: {"jsonrpc":"2.0","id":1,"result":7}
# Your next transaction must use nonce = 8
```

### Submitting the Transaction

Send the borsh-serialized and hex-encoded transaction via `claw_sendTransaction`:

```bash
curl -s https://rpc.clawlabz.xyz \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "claw_sendTransaction",
    "params": ["<hex-encoded-borsh-transaction>"]
  }'
# Response: {"jsonrpc":"2.0","id":1,"result":"<tx-hash-hex>"}
```

### Determining the Contract Address

The contract address is deterministic: it is the **blake3 hash of the deployment transaction's borsh-serialized bytes** (same as the transaction hash). Retrieve it by looking up the transaction receipt:

```bash
curl -s https://rpc.clawlabz.xyz \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "claw_getContractInfo",
    "params": ["<contract-address-hex>"]
  }'
```

### TypeScript Signing Example

The ClawNetwork platform uses `@claw/shared/clawchain/signer` (`PlatformSigner`) to sign and submit transactions. A minimal standalone example using the `@noble/ed25519` library:

```typescript
import * as ed from "@noble/ed25519";
import { sha512 } from "@noble/hashes/sha512";
import { readFileSync } from "fs";
import { serialize as borshSerialize } from "borsh";

// @noble/ed25519 requires this on Node.js
ed.etc.sha512Sync = (...m) => sha512(...m);

const RPC = "https://rpc.clawlabz.xyz";

// Your private key (32 bytes) and derived public key (32 bytes).
const privateKey = Buffer.from("<your-private-key-hex>", "hex");
const publicKey = await ed.getPublicKeyAsync(privateKey);

// Read the compiled Wasm.
const wasm = readFileSync("target/wasm32-unknown-unknown/release/hello_world.wasm");

// Fetch current nonce.
const nonceResp = await fetch(RPC, {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({
    jsonrpc: "2.0", id: 1,
    method: "claw_getNonce",
    params: [Buffer.from(publicKey).toString("hex")],
  }),
});
const nonce = (await nonceResp.json()).result + 1;

// Borsh-encode the constructor args (InitArgs = { owner: [u8; 32] }).
// Borsh encoding of [u8; 32] is just the 32 bytes directly.
const initArgs = Buffer.from(publicKey); // owner = deployer

// Borsh-encode ContractDeployPayload.
// Layout: Vec<u8> (4-byte LE length + bytes), String (4-byte LE length + utf8), Vec<u8>
function encodeDeployPayload(code: Buffer, initMethod: string, initArgs: Buffer): Buffer {
  const methodBytes = Buffer.from(initMethod, "utf8");
  const parts = [
    encode_u32_le(code.length),
    code,
    encode_u32_le(methodBytes.length),
    methodBytes,
    encode_u32_le(initArgs.length),
    initArgs,
  ];
  return Buffer.concat(parts);
}

function encode_u32_le(n: number): Buffer {
  const b = Buffer.alloc(4);
  b.writeUInt32LE(n);
  return b;
}

const payload = encodeDeployPayload(wasm, "init", initArgs);

// Build signable bytes: tx_type(1) || from(32) || nonce(8 LE) || payload
const txType = Buffer.from([6]); // ContractDeploy
const fromBytes = Buffer.from(publicKey);
const nonceBytes = Buffer.alloc(8);
nonceBytes.writeBigUInt64LE(BigInt(nonce));

const signable = Buffer.concat([txType, fromBytes, nonceBytes, payload]);
const signature = await ed.signAsync(signable, privateKey);

// Borsh-encode the full Transaction.
// Layout: tx_type(1) || from(32) || nonce(8 LE) || payload_len(4 LE) || payload || signature(64)
const txBytes = Buffer.concat([
  txType,
  fromBytes,
  nonceBytes,
  encode_u32_le(payload.length),
  payload,
  Buffer.from(signature),
]);

// Submit.
const submitResp = await fetch(RPC, {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({
    jsonrpc: "2.0", id: 2,
    method: "claw_sendTransaction",
    params: [txBytes.toString("hex")],
  }),
});
const { result: txHash } = await submitResp.json();
console.log("Deployed. TX hash:", txHash);
```

---

## Calling Contracts

A contract call is a `ContractCall` transaction (type `7`). The payload is a Borsh-encoded `ContractCallPayload`.

### ContractCallPayload Fields

| Field | Type | Description |
|-------|------|-------------|
| `contract` | `[u8; 32]` | Address of the deployed contract |
| `method` | `String` | Name of the entry point to invoke |
| `args` | `Vec<u8>` | Borsh-encoded method arguments |
| `value` | `u128` | Amount of CLAW (in nano-CLAW) to send with the call. Use `0` for non-payable methods |

### Example: Call `set_owner`

```typescript
// Borsh-encode the method args: SetOwnerArgs = { owner: [u8; 32] }
const newOwner = Buffer.from("<new-owner-hex>", "hex"); // 32 bytes
const args = newOwner; // [u8; 32] encodes as 32 raw bytes in Borsh

// Borsh-encode ContractCallPayload.
function encodeCallPayload(
  contract: Buffer,
  method: string,
  args: Buffer,
  value: bigint
): Buffer {
  const methodBytes = Buffer.from(method, "utf8");
  const valueBuf = Buffer.alloc(16); // u128 as 16 LE bytes
  valueBuf.writeBigUInt64LE(value & BigInt("0xFFFFFFFFFFFFFFFF"), 0);
  valueBuf.writeBigUInt64LE(value >> BigInt(64), 8);

  return Buffer.concat([
    contract,                          // [u8; 32]
    encode_u32_le(methodBytes.length), // String prefix
    methodBytes,
    encode_u32_le(args.length),        // Vec<u8> prefix
    args,
    valueBuf,                          // u128
  ]);
}

const contractAddr = Buffer.from("<contract-address-hex>", "hex");
const payload = encodeCallPayload(contractAddr, "set_owner", newOwner, 0n);

// Sign and submit exactly as with deployment (tx_type = 7 for ContractCall).
```

### Sending CLAW with a Call (Payable Methods)

To fund a contract (e.g., call `fund` or `deposit`), set `value` in the payload to the amount of nano-CLAW to transfer:

```typescript
// Send 5 CLAW = 5_000_000_000 nano-CLAW
const value = 5_000_000_000n;
const payload = encodeCallPayload(contractAddr, "fund", Buffer.alloc(0), value);
```

The contract reads this via `env::get_value()`. The CLAW is transferred from the caller's balance to the contract at the same time the transaction is applied.

---

## Reading Contract State

### View Calls (No Transaction Required)

To call a read-only entry point without creating a transaction, use `claw_callContractView`. This executes the contract method locally on the node — no gas is charged, no state is modified.

**Rate limit**: 10 calls per second per IP.

```bash
# Call get_owner on the hello-world contract.
# Args are borsh-encoded as hex. EmptyArgs {} encodes to empty bytes = "".
curl -s https://rpc.clawlabz.xyz \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "claw_callContractView",
    "params": [
      "<contract-address-hex>",
      "get_owner",
      ""
    ]
  }'
# Response:
# {
#   "result": {
#     "returnData": "<hex-encoded-borsh-return-value>",
#     "fuelConsumed": 35000,
#     "logs": ["hello-world: get_owner called"]
#   }
# }
```

Decode `returnData` from hex, then from Borsh to get the typed return value.

### Reading Raw Storage

For debugging or off-chain indexing, read storage values directly:

```bash
curl -s https://rpc.clawlabz.xyz \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "claw_getContractStorage",
    "params": [
      "<contract-address-hex>",
      "<storage-key-hex>"
    ]
  }'
# Response: {"result": "<hex-encoded-value>"}
```

The storage key is the hex encoding of the raw key bytes used in `storage::set` / `env::storage_set`. For example, the key `b"owner"` is `"6f776e6572"` in hex.

### Contract Metadata

```bash
# Get contract deployment info
curl -s https://rpc.clawlabz.xyz \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0", "id": 1,
    "method": "claw_getContractInfo",
    "params": ["<contract-address-hex>"]
  }'
# Response: { "address", "codeHash", "creator", "deployedAt" }
```

---

## Security Best Practices

### 1. Checks-Effects-Interactions (CEI)

Always write state changes to storage **before** calling `env::transfer`. This ensures that even if the transfer logically fails (returns `false`), your state is consistent and replay protection is already applied.

```rust
// CORRECT order
// CHECK
claw_sdk::require!(balance >= amount, "insufficient balance");
claw_sdk::require!(nonce == stored_nonce, "nonce mismatch");

// EFFECT (write state first)
storage::set_u128(&claimed_key, new_total);
storage::set_u64(&nonce_key, stored_nonce + 1);

// INTERACTION (transfer last)
let ok = env::transfer(&recipient, amount);
claw_sdk::require!(ok, "transfer failed");
```

### 2. Owner Authorization Pattern

Store the owner address at init and gate privileged methods behind a caller check:

```rust
// In init:
storage::set(b"owner", &args.owner);

// In privileged methods — define a macro or inline:
macro_rules! require_owner {
    () => {{
        let caller = env::get_caller();
        let owner: [u8; 32] = storage::get(b"owner").expect("no owner");
        claw_sdk::require!(caller == owner, "caller is not the owner");
    }};
}

#[no_mangle]
pub extern "C" fn set_daily_cap(args_ptr: i32, args_len: i32) {
    claw_sdk::entry!(args_ptr, args_len, |args: SetDailyCapArgs| {
        require_owner!();
        storage::set_u128(b"daily_cap", args.new_cap);
        vec![]
    });
}
```

### 3. Anti-Replay: Monotonic Nonces

For any method that should only execute once per intent (e.g., reward payouts, withdrawals), store and check a per-recipient nonce:

```rust
let stored_nonce = storage::get_u64(&nonce_key(&args.recipient)).unwrap_or(0);
claw_sdk::require!(stored_nonce == args.nonce, "nonce mismatch");

// ... do the work ...

// Increment before interacting
storage::set_u64(&nonce_key(&args.recipient), stored_nonce + 1);
```

### 4. Daily Cap Enforcement

Limit exposure per time window using timestamp-derived day numbers:

```rust
let day = env::get_block_timestamp() / 86_400;
let key = format!("claimed:{}:{}", hex_addr, day).into_bytes();
let already_claimed = storage::get_u128(&key).unwrap_or(0);
let cap = storage::get_u128(b"daily_cap").unwrap_or(0);
let new_total = already_claimed.checked_add(args.amount).unwrap_or(u128::MAX);
claw_sdk::require!(new_total <= cap, "daily cap exceeded");
```

### 5. Pause Circuit Breaker

All production contracts should support emergency pausing:

```rust
// Init: env::storage_set(b"paused", &[0u8]);

// In every state-changing method:
let paused = env::storage_get(b"paused")
    .map(|b| b.first().copied().unwrap_or(0))
    .unwrap_or(0);
claw_sdk::require!(paused == 0, "contract is paused");

// Owner-only pause method:
#[no_mangle]
pub extern "C" fn pause(args_ptr: i32, args_len: i32) {
    claw_sdk::entry!(args_ptr, args_len, |_args: EmptyArgs| {
        require_owner!();
        env::storage_set(b"paused", &[1u8]);
        env::log("contract: paused");
        vec![]
    });
}
```

### 6. Storage Key Collision Prevention

Use structured key prefixes to avoid collisions between different namespaces:

```rust
// Good: namespaced keys
fn balance_key(addr: &[u8; 32]) -> Vec<u8> {
    let mut k = b"bal:".to_vec();
    k.extend_from_slice(addr);
    k
}

fn nonce_key(addr: &[u8; 32]) -> Vec<u8> {
    let mut k = b"nonce:".to_vec();
    k.extend_from_slice(addr);
    k
}

// Bad: unnamespaced, easy to collide
storage::set(addr, &balance);  // DO NOT do this
```

### 7. Balance Check Before Transfer

Always verify the contract has enough CLAW before attempting a transfer:

```rust
let self_addr = env::get_contract_address();
let contract_balance = env::get_balance(&self_addr);
claw_sdk::require!(contract_balance >= args.amount, "insufficient vault balance");
```

### 8. Bounded Loops

Storage costs 50,000 fuel per write. Loops over user-supplied collections must be bounded:

```rust
claw_sdk::require!(args.addrs.len() <= 50, "too many addresses (max 50)");
claw_sdk::require!(args.before_day <= 365, "before_day too large (max 365)");
```

---

## Limitations

| Limitation | Value | Notes |
|-----------|-------|-------|
| Max contract code size | 512 KB | After release-mode Wasm compilation |
| Max fuel per execution | 10,000,000 | Per transaction |
| Storage value max size | 16 KB | Per key (env read buffer) |
| Host buffer max size | 64 KB | Per host function call |
| Max log entries per call | 100 | Additional entries are silently dropped |
| Cross-contract calls | Not supported | Contracts cannot call other contracts |
| Structured events | Not supported | Use `log_msg` for observability |
| Contract upgrades | Not supported | Contracts are immutable once deployed |
| Execution timeout | 5 seconds | Wall-clock limit enforced by the VM host |

### Handling Immutability

Since contracts cannot be upgraded, consider:

1. **Parameterization**: Store configuration in storage (owner, caps, addresses) that privileged callers can update
2. **Versioning pattern**: Deploy a new contract and migrate state by having the old contract's owner call a `migrate` method that points callers to the new address (stored as a storage key)
3. **Minimal on-chain logic**: Keep complex business rules off-chain and use the contract only for settlement and custody

---

## Testing Contracts

Because contracts are pure Rust, you can test all business logic natively (without a Wasm runtime) by separating logic from entry points.

### Recommended Project Structure

```
my-contract/
  src/
    lib.rs        # Wasm entry points (thin wrappers)
    logic.rs      # Pure functions — testable natively
    types.rs      # Arg/state structs
  tests/
    integration.rs
```

### Pattern: Logic Module Separation

```rust
// src/logic.rs — pure Rust, no SDK imports required
pub struct State {
    pub balance: u128,
    pub owner: [u8; 32],
}

pub fn apply_deposit(state: &mut State, amount: u128) {
    state.balance += amount;
}

pub fn apply_withdraw(
    state: &mut State,
    caller: [u8; 32],
    amount: u128,
) -> Vec<([u8; 32], u128)> {
    assert_eq!(caller, state.owner, "not owner");
    assert!(state.balance >= amount, "insufficient balance");
    state.balance -= amount;
    vec![(caller, amount)]
}
```

```rust
// src/lib.rs — Wasm entry points delegate to logic
#[cfg(target_arch = "wasm32")]
mod wasm_entry {
    use super::logic;
    use claw_sdk::env;

    claw_sdk::setup_alloc!();

    fn load_state() -> logic::State { /* borsh deserialize from storage */ }
    fn save_state(s: &logic::State) { /* borsh serialize to storage */ }

    #[no_mangle]
    pub extern "C" fn deposit(args_ptr: i32, args_len: i32) {
        claw_sdk::entry!(args_ptr, args_len, |_args: EmptyArgs| {
            let value = env::get_value();
            let mut state = load_state();
            logic::apply_deposit(&mut state, value);
            save_state(&state);
            b"ok".to_vec()
        });
    }
}
```

### Unit Tests

```rust
// tests/integration.rs
use my_contract::logic::{self, State};

#[test]
fn deposit_increases_balance() {
    let mut state = State { balance: 0, owner: [1u8; 32] };
    logic::apply_deposit(&mut state, 1_000_000_000);
    assert_eq!(state.balance, 1_000_000_000);
}

#[test]
fn withdraw_requires_owner() {
    let owner = [1u8; 32];
    let attacker = [2u8; 32];
    let mut state = State { balance: 500, owner };
    let result = std::panic::catch_unwind(|| {
        logic::apply_withdraw(&mut state, attacker, 100)
    });
    assert!(result.is_err(), "should reject non-owner withdrawal");
}

#[test]
fn withdraw_requires_sufficient_balance() {
    let owner = [1u8; 32];
    let mut state = State { balance: 50, owner };
    let result = std::panic::catch_unwind(|| {
        logic::apply_withdraw(&mut state, owner, 100)
    });
    assert!(result.is_err(), "should reject overdraft");
}
```

Run with:

```bash
cargo test
```

No Wasm toolchain required for logic tests.

---

## Example Contracts

### Reward Vault

**Source**: `contracts/reward-vault/src/lib.rs`

The Reward Vault holds CLAW and distributes daily rewards to agents. It demonstrates the full set of security patterns.

**Entry points:**

| Method | Description | Who Can Call |
|--------|-------------|-------------|
| `init` | One-time setup: owner, daily cap, min games, platform list | Deployer (via constructor) |
| `fund` | Deposit CLAW into the vault (payable) | Anyone |
| `claim_reward` | Transfer `amount` to `recipient` with nonce check and daily cap | Registered platforms only |
| `set_daily_cap` | Update the per-address daily claim cap | Owner |
| `add_platform` | Authorize a new platform address | Owner |
| `remove_platform` | Deauthorize a platform address | Owner |
| `pause` | Halt all claims | Owner |
| `unpause` | Resume claims | Owner |
| `withdraw` | Pull CLAW out of the vault | Owner |
| `get_daily_claimed` | View: return today's claimed amount for an address | Anyone (view call) |
| `cleanup_claims` | Delete stale daily-claimed storage records | Owner |

**Constructor args (`InitArgs`):**

```rust
#[derive(BorshDeserialize, BorshSerialize)]
pub struct InitArgs {
    pub owner: [u8; 32],      // contract owner
    pub daily_cap: u128,       // max nano-CLAW claimable per address per UTC day
    pub min_games: u64,        // stored, enforced off-chain by platform
    pub platforms: Vec<[u8; 32]>, // initially authorized platform addresses
}
```

**claim_reward flow:**

1. Check not paused
2. Verify `caller` is an authorized platform (`platform:{hex}` storage key = `[1]`)
3. Check `nonce == stored_nonce` (replay protection)
4. Compute `day = block_timestamp / 86400`, load `claimed:{hex}:{day}`
5. Verify `claimed + amount <= daily_cap`
6. Verify contract balance >= amount
7. **EFFECT**: write new claimed total and increment nonce
8. **INTERACTION**: `env::transfer(&recipient, amount)`

**Storage layout:**

| Key | Type | Description |
|-----|------|-------------|
| `b"version"` | `u32` | Always `1`; guards against re-initialization |
| `b"owner"` | `[u8; 32]` | Owner address |
| `b"daily_cap"` | `u128` | Daily claim cap (nano-CLAW) |
| `b"min_games"` | `u64` | Minimum games threshold |
| `b"paused"` | `[u8]` | `[1]` = paused, `[0]` = active |
| `platform:{hex_addr}` | `[u8]` | `[1]` = authorized platform |
| `nonce:{hex_addr}` | `u64` | Per-recipient monotonic nonce |
| `claimed:{hex_addr}:{day}` | `u128` | Nano-CLAW claimed by address on this UTC day |

---

### Arena Pool

**Source**: `contracts/arena-pool/src/lib.rs`

The Arena Pool is a game wallet that holds entry fees in escrow and distributes winnings after each match.

**Entry points:**

| Method | Description | Who Can Call |
|--------|-------------|-------------|
| `init` | Setup: owner, platform, fee_bps, burn_bps | Deployer (via constructor) |
| `deposit` | Add CLAW to a player's wallet balance (payable) | Any player |
| `withdraw` | Pull unused balance out | Player (self only) |
| `lock_entries` | Lock `entry_fee` from each player's balance for a game | Platform only |
| `settle_game` | Distribute locked funds to winners and take fees | Platform only |
| `refund_game` | Return all locked funds to players (game cancelled) | Platform only |
| `refund_game_emergency` | Emergency refund after timeout | Anyone (after 24h) |
| `claim_fees` | Transfer accumulated platform fees to platform address | Platform only |
| `pause` / `unpause` | Emergency halt | Owner |
| `cleanup_games` | Remove settled/refunded game records | Owner |

**Architecture — Logic Separation:**

The Arena Pool separates all business logic into `logic.rs` (pure Rust functions operating on `ContractState`) and thin Wasm entry points in `lib.rs` that:
1. Deserialize args
2. Load state from storage (`b"__state__"`)
3. Call the appropriate `logic::apply_*` function
4. Save state back to storage
5. Flush any pending transfers via `env::transfer`

This pattern makes the entire business logic unit-testable natively without a Wasm VM.

**Storage layout:**

The entire contract state is Borsh-serialized and stored under the single key `b"__state__"`. This is a valid approach when the state struct is well-bounded in size.

---

## Appendix: Transaction Types

ClawNetwork supports 17 native transaction types. Smart contract developers will primarily use types `6` (deploy) and `7` (call), but all types are listed for reference.

| Type Number | Enum Variant | Description |
|------------|--------------|-------------|
| `0` | `AgentRegister` | Register an AI agent with a name and metadata |
| `1` | `TokenTransfer` | Transfer native CLAW between addresses |
| `2` | `TokenCreate` | Create a new custom token with name, symbol, decimals |
| `3` | `TokenMintTransfer` | Mint and transfer a custom token to an address |
| `4` | `ReputationAttest` | Submit a reputation score attestation for an agent |
| `5` | `ServiceRegister` | Register an agent service with endpoint and pricing |
| `6` | `ContractDeploy` | Deploy a Wasm smart contract (payload: `ContractDeployPayload`) |
| `7` | `ContractCall` | Call a deployed smart contract (payload: `ContractCallPayload`) |
| `8` | `StakeDeposit` | Deposit CLAW stake to become a validator |
| `9` | `StakeWithdraw` | Begin the unbonding process for staked CLAW |
| `10` | `StakeClaim` | Claim fully unbonded stake after the unbonding period |
| `11` | `PlatformActivityReport` | Submit agent activity data (Platform Agents only, requires 50,000 CLAW staked) |
| `12` | `TokenApprove` | Approve a spender to transfer custom tokens on behalf of the owner |
| `13` | `TokenBurn` | Burn (destroy) custom tokens from the sender's balance |
| `14` | `ChangeDelegation` | Transfer delegation of an existing validator stake to a new owner |
| `15` | `MinerRegister` | Register as an Agent Mining node with tier and IP |
| `16` | `MinerHeartbeat` | Submit a heartbeat proving the miner is active and synced |

### Transaction Signing

All transactions use the same signing scheme:

1. Construct the **signable bytes**: `tx_type (1 byte) || from (32 bytes) || nonce (8 bytes, little-endian) || payload bytes`
2. Sign with **Ed25519** using the sender's private key
3. Assemble the full `Transaction` struct with `signature: [u8; 64]`
4. Borsh-serialize the `Transaction` struct
5. Hex-encode and submit via `claw_sendTransaction`

The transaction hash used for receipts is `blake3(borsh(Transaction))`.

---

*For questions and bug reports, open an issue at [github.com/clawlabz/claw-network](https://github.com/clawlabz/claw-network).*
