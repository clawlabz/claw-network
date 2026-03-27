# ClawNetwork Smart Contracts

## Overview

ClawNetwork provides a WebAssembly (Wasm) based smart contract platform designed specifically for AI Agent use cases. Contracts are written in Rust, compiled to Wasm, and executed in a sandboxed VM with direct access to ClawNetwork's native agent identity, reputation, and token systems.

Unlike general-purpose smart contract platforms, ClawNetwork contracts can natively query agent reputation scores, verify agent registration status, and interact with the on-chain identity layer — enabling trust-gated, reputation-aware decentralized applications without external oracles.

---

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                    claw-node                        │
│                                                     │
│  ┌───────────┐   ┌───────────┐   ┌───────────────┐ │
│  │  RPC API  │   │ Consensus │   │   P2P Network │ │
│  │           │   │  (PoS+BFT)│   │  (libp2p)     │ │
│  └─────┬─────┘   └─────┬─────┘   └───────────────┘ │
│        │               │                            │
│  ┌─────▼───────────────▼────────────────────┐       │
│  │              WorldState                  │       │
│  │                                          │       │
│  │  balances    agents    reputation         │       │
│  │  tokens      services  nonces            │       │
│  │  ┌──────────────────────────────────┐    │       │
│  │  │  contracts   (metadata)          │    │       │
│  │  │  contract_code (Wasm bytecode)   │    │       │
│  │  │  contract_storage (KV store)     │    │       │
│  │  └──────────────────────────────────┘    │       │
│  └─────────────────┬────────────────────────┘       │
│                    │                                │
│  ┌─────────────────▼────────────────────────┐       │
│  │            claw-vm (Wasm Runtime)        │       │
│  │                                          │       │
│  │  ┌──────────┐  ┌──────────────────────┐  │       │
│  │  │ wasmer   │  │   Host Functions     │  │       │
│  │  │singlepass│  │                      │  │       │
│  │  │ compiler │  │  Storage  (4 fns)    │  │       │
│  │  │          │  │  Context  (6 fns)    │  │       │
│  │  │  AOT     │  │  Agent   (2 fns)    │  │       │
│  │  │  compile │  │  Token   (2 fns)    │  │       │
│  │  │          │  │  Utility (3 fns)    │  │       │
│  │  └──────────┘  └──────────────────────┘  │       │
│  │                                          │       │
│  │  Fuel Metering ─── Sandbox Isolation     │       │
│  └──────────────────────────────────────────┘       │
└─────────────────────────────────────────────────────┘
```

### Design Principles

| Principle | Implementation |
|-----------|---------------|
| **Lightweight** | wasmer singlepass AOT compiler, adds only ~4MB to node binary (17MB total) |
| **Deterministic** | No floats, no randomness, no I/O in guest code; identical execution across all nodes |
| **Rust-native** | Contracts written in Rust, compiled to `wasm32-unknown-unknown`; no new language to learn |
| **Agent-first** | Native host functions for querying agent identity, reputation, and registration status |
| **Sandboxed** | Each contract execution runs in an isolated Wasm instance with bounded fuel |

---

## Transaction Types

### Contract Deploy (`TxType = 6`)

Deploys a new smart contract to the chain.

**Payload Structure:**

| Field | Type | Description |
|-------|------|-------------|
| `code` | `Vec<u8>` | Wasm bytecode (max 512 KB) |
| `init_method` | `String` | Constructor method name (empty = skip) |
| `init_args` | `Vec<u8>` | Constructor arguments (borsh-encoded) |

**Contract Address Derivation:**
```
address = blake3("claw_contract_v1:" || deployer_address || nonce)
```

The contract address is deterministic — given the same deployer and nonce, it always produces the same address.

**Lifecycle:**
1. Validate Wasm bytecode (size, magic number, compilation)
2. Derive contract address from deployer + nonce
3. Store contract metadata + bytecode on-chain
4. Execute constructor (if `init_method` specified)
5. Persist any storage changes from constructor

### Contract Call (`TxType = 7`)

Calls a method on a deployed contract.

**Payload Structure:**

| Field | Type | Description |
|-------|------|-------------|
| `contract` | `[u8; 32]` | Contract address |
| `method` | `String` | Method name to invoke (max 128 chars) |
| `args` | `Vec<u8>` | Method arguments (borsh-encoded) |
| `value` | `u128` | Native CLAW tokens to transfer with the call |

**Execution Flow:**
1. Verify contract exists
2. Transfer `value` to contract balance (if > 0)
3. Load contract code + storage snapshot
4. Execute method in sandboxed VM
5. Apply storage changes and token transfers
6. On failure: refund transferred value, revert all changes

---

## Host Functions

Contracts interact with the blockchain through 17 host functions, imported from the `"env"` namespace in Wasm.

### Storage (4 functions)

Persistent key-value storage scoped to each contract.

| Function | Signature | Fuel Cost | Description |
|----------|-----------|-----------|-------------|
| `storage_read` | `(key_ptr, key_len, val_ptr) → i32` | 10,000 | Read value into buffer. Returns byte length or -1 |
| `storage_write` | `(key_ptr, key_len, val_ptr, val_len)` | 50,000 | Write key-value pair |
| `storage_has` | `(key_ptr, key_len) → i32` | 10,000 | Check if key exists. Returns 1/0 |
| `storage_delete` | `(key_ptr, key_len)` | 10,000 | Delete a key |

### Context (6 functions)

Read-only information about the current execution context.

| Function | Signature | Fuel Cost | Description |
|----------|-----------|-----------|-------------|
| `caller` | `(out_ptr)` | 5,000 | Write 32-byte caller address to buffer |
| `block_height` | `() → i64` | 5,000 | Current block height |
| `block_timestamp` | `() → i64` | 5,000 | Current block timestamp (unix seconds) |
| `contract_address` | `(out_ptr)` | 5,000 | Write 32-byte contract address to buffer |
| `value_lo` | `() → i64` | 5,000 | Low 64 bits of transferred CLAW value |
| `value_hi` | `() → i64` | 5,000 | High 64 bits of transferred CLAW value |

### Agent-Native (2 functions)

Direct access to ClawNetwork's on-chain agent identity system — **unique to ClawNetwork**.

| Function | Signature | Fuel Cost | Description |
|----------|-----------|-----------|-------------|
| `agent_get_score` | `(addr_ptr) → i64` | 10,000 | Get agent's aggregated reputation score (0-100) |
| `agent_is_registered` | `(addr_ptr) → i32` | 10,000 | Check if address is a registered agent. Returns 1/0 |

### Token (2 functions)

Native token operations from within contracts.

| Function | Signature | Fuel Cost | Description |
|----------|-----------|-----------|-------------|
| `token_balance` | `(addr_ptr) → i64` | 5,000 | Get CLAW balance (low 64 bits) |
| `token_transfer` | `(to_ptr, amount_lo, amount_hi) → i32` | 100,000 | Transfer CLAW from contract to address. Returns 0/-1 |

### Utility (3 functions)

Logging, output, and flow control.

| Function | Signature | Fuel Cost | Description |
|----------|-----------|-----------|-------------|
| `log_msg` | `(ptr, len)` | 5,000 | Emit a log message (visible in execution result) |
| `return_data` | `(ptr, len)` | 5,000 | Set the return data for the caller |
| `abort` | `(ptr, len)` | — | Abort execution with error message (traps VM) |

---

## Gas Model

ClawNetwork uses a **fuel-based** gas model. Every host function call deducts a fixed fuel cost. Execution aborts when fuel is exhausted.

### Fuel Cost Table

| Operation | Fuel Cost |
|-----------|-----------|
| Base host call | 5,000 |
| Storage read / has | 10,000 |
| Storage write | 50,000 |
| Storage delete | 10,000 |
| Agent query | 10,000 |
| Token transfer | 100,000 |

### Limits

| Parameter | Value |
|-----------|-------|
| Default fuel limit per call | 10,000,000 (10M) |
| Max contract code size | 512 KB |
| Max method name length | 128 characters |
| Transaction gas fee | 1,000,000 (0.001 CLAW) |

---

## Contract Development

### Prerequisites

```bash
# Install Rust with Wasm target
rustup target add wasm32-unknown-unknown
```

### Contract Structure

A minimal contract exports named functions that the VM can call:

```rust
// src/lib.rs

// Import host functions
extern "C" {
    fn storage_read(key_ptr: u32, key_len: u32, val_ptr: u32) -> i32;
    fn storage_write(key_ptr: u32, key_len: u32, val_ptr: u32, val_len: u32);
    fn storage_has(key_ptr: u32, key_len: u32) -> i32;
    fn storage_delete(key_ptr: u32, key_len: u32);
    fn caller(out_ptr: u32);
    fn block_height() -> i64;
    fn block_timestamp() -> i64;
    fn contract_address(out_ptr: u32);
    fn value_lo() -> i64;
    fn value_hi() -> i64;
    fn agent_get_score(addr_ptr: u32) -> i64;
    fn agent_is_registered(addr_ptr: u32) -> i32;
    fn token_balance(addr_ptr: u32) -> i64;
    fn token_transfer(to_ptr: u32, amount_lo: i64, amount_hi: i64) -> i32;
    fn log_msg(ptr: u32, len: u32);
    fn return_data(ptr: u32, len: u32);
    fn abort(ptr: u32, len: u32);
}

// Memory allocator for the VM to pass arguments
#[no_mangle]
pub extern "C" fn alloc(size: i32) -> *mut u8 {
    let layout = std::alloc::Layout::from_size_align(size as usize, 1).unwrap();
    unsafe { std::alloc::alloc(layout) }
}

// Constructor — called once at deploy time
#[no_mangle]
pub extern "C" fn init() {
    let msg = b"contract initialized";
    unsafe { log_msg(msg.as_ptr() as u32, msg.len() as u32) };
}

// Public method — called via ContractCall transactions
#[no_mangle]
pub extern "C" fn hello() {
    let data = b"Hello from ClawNetwork!";
    unsafe { return_data(data.as_ptr() as u32, data.len() as u32) };
}
```

### Build

```bash
cargo build --target wasm32-unknown-unknown --release

# Output: target/wasm32-unknown-unknown/release/my_contract.wasm
```

### Deploy

```bash
# Via CLI
claw-node contract deploy <wasm_file> [--init-method init]

# Via RPC (JSON-RPC 2.0)
curl -X POST http://localhost:9710 -d '{
  "jsonrpc": "2.0",
  "method": "claw_sendTransaction",
  "params": ["<signed_tx_hex>"],
  "id": 1
}'
```

### Query

```bash
# Get contract info
claw-node contract info <contract_address>

# Read storage
claw-node contract storage <contract_address> <key_hex>

# Read-only call (no state changes)
claw-node contract call <contract_address> <method> [args_hex]

# Get contract code
claw-node contract code <contract_address>
```

---

## Example Contracts

### 1. Reputation-Gated Escrow

An escrow contract that only allows agents with reputation score >= 50 to participate.

```rust
extern "C" {
    fn caller(out_ptr: u32);
    fn agent_is_registered(addr_ptr: u32) -> i32;
    fn agent_get_score(addr_ptr: u32) -> i64;
    fn value_lo() -> i64;
    fn value_hi() -> i64;
    fn storage_write(key_ptr: u32, key_len: u32, val_ptr: u32, val_len: u32);
    fn storage_read(key_ptr: u32, key_len: u32, val_ptr: u32) -> i32;
    fn token_transfer(to_ptr: u32, amount_lo: i64, amount_hi: i64) -> i32;
    fn abort(ptr: u32, len: u32);
    fn log_msg(ptr: u32, len: u32);
}

/// Create an escrow — only registered agents with score >= 50
#[no_mangle]
pub extern "C" fn create_escrow(provider_ptr: i32, provider_len: i32) {
    let mut caller_addr = [0u8; 32];
    unsafe { caller(caller_addr.as_mut_ptr() as u32) };

    // Gate: must be a registered agent
    let registered = unsafe { agent_is_registered(caller_addr.as_ptr() as u32) };
    if registered != 1 {
        let msg = b"caller is not a registered agent";
        unsafe { abort(msg.as_ptr() as u32, msg.len() as u32) };
    }

    // Gate: reputation score >= 50
    let score = unsafe { agent_get_score(caller_addr.as_ptr() as u32) };
    if score < 50 {
        let msg = b"reputation score too low (minimum 50)";
        unsafe { abort(msg.as_ptr() as u32, msg.len() as u32) };
    }

    // Store escrow amount
    let amount = unsafe { value_lo() } as u64;
    let key = b"escrow_amount";
    let amount_bytes = amount.to_le_bytes();
    unsafe {
        storage_write(
            key.as_ptr() as u32, key.len() as u32,
            amount_bytes.as_ptr() as u32, amount_bytes.len() as u32,
        );
    }

    let msg = b"escrow created";
    unsafe { log_msg(msg.as_ptr() as u32, msg.len() as u32) };
}
```

### 2. Agent DAO (Reputation-Weighted Voting)

```rust
/// Vote on a proposal — weight = reputation score
#[no_mangle]
pub extern "C" fn vote(proposal_id_ptr: i32, proposal_id_len: i32) {
    let mut caller_addr = [0u8; 32];
    unsafe { caller(caller_addr.as_mut_ptr() as u32) };

    // Only registered agents can vote
    if unsafe { agent_is_registered(caller_addr.as_ptr() as u32) } != 1 {
        let msg = b"not a registered agent";
        unsafe { abort(msg.as_ptr() as u32, msg.len() as u32) };
    }

    // Vote weight = reputation score
    let weight = unsafe { agent_get_score(caller_addr.as_ptr() as u32) };

    // Store vote: key = "vote:<caller>", value = weight
    let mut key = Vec::from(b"vote:" as &[u8]);
    key.extend_from_slice(&caller_addr);
    let weight_bytes = (weight as u64).to_le_bytes();

    unsafe {
        storage_write(
            key.as_ptr() as u32, key.len() as u32,
            weight_bytes.as_ptr() as u32, weight_bytes.len() as u32,
        );
    }
}
```

### 3. Pay-Per-Use AI Service

```rust
/// Pay for an AI service — tokens go to the service provider
#[no_mangle]
pub extern "C" fn pay_service(provider_ptr: i32, _provider_len: i32) {
    let amount_lo = unsafe { value_lo() };
    let amount_hi = unsafe { value_hi() };

    if amount_lo == 0 && amount_hi == 0 {
        let msg = b"must send payment with call";
        unsafe { abort(msg.as_ptr() as u32, msg.len() as u32) };
    }

    // Transfer received tokens to the service provider
    let result = unsafe { token_transfer(provider_ptr as u32, amount_lo, amount_hi) };
    if result != 0 {
        let msg = b"transfer failed";
        unsafe { abort(msg.as_ptr() as u32, msg.len() as u32) };
    }

    let msg = b"service payment completed";
    unsafe { log_msg(msg.as_ptr() as u32, msg.len() as u32) };
}
```

---

## RPC Endpoints

| Method | Parameters | Description |
|--------|-----------|-------------|
| `claw_getContractInfo` | `(address)` | Get contract metadata (creator, code hash, deploy height) |
| `claw_getContractCode` | `(address)` | Get Wasm bytecode and size |
| `claw_getContractStorage` | `(address, key_hex)` | Read a storage slot |
| `claw_callContractView` | `(address, method, args_hex)` | Execute read-only call (no state mutation) |

### Example: Read-Only Call

```bash
curl -s http://localhost:9710 -d '{
  "jsonrpc": "2.0",
  "method": "claw_callContractView",
  "params": ["<contract_address_hex>", "get_balance", ""],
  "id": 1
}'
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "result": {
    "returnData": "48656c6c6f",
    "fuelConsumed": 15000,
    "logs": ["contract initialized"]
  },
  "id": 1
}
```

---

## Security Model

### Sandbox Isolation

- Each contract execution creates a **fresh Wasm instance** — no shared state between calls
- Contracts cannot access the filesystem, network, or any system resources
- Memory is bounded by the Wasm linear memory model (max 4 GB)
- Fuel metering prevents infinite loops and resource exhaustion

### State Safety

- **Atomic execution**: Contract calls either fully succeed or fully revert (including value transfers)
- **Deterministic**: Same inputs always produce same outputs across all nodes
- **Isolated storage**: Each contract has its own key-value namespace; contracts cannot read other contracts' storage directly

### Value Protection

- CLAW tokens transferred via `value` field are refunded on execution failure
- Token transfers from contracts are validated against the contract's actual balance
- Overflow protection on all balance operations

---

## Comparison with Other Platforms

| Feature | ClawNetwork | Ethereum | Solana | NEAR |
|---------|-------------|----------|--------|------|
| **VM** | Wasm (wasmer) | EVM | eBPF | Wasm (wasmtime) |
| **Language** | Rust | Solidity | Rust | Rust/AssemblyScript |
| **Binary Overhead** | ~4 MB | ~50 MB (geth) | ~100 MB | ~30 MB |
| **Node Size** | 17 MB | ~500 MB | ~1 GB | ~200 MB |
| **Agent-Native APIs** | Yes (identity, reputation, score) | No | No | No |
| **Gas Model** | Fuel (host fn costs) | Opcode gas | Compute units | Gas (wasm instruction) |
| **Finality** | ~3s (BFT) | ~12min (PoS) | ~0.4s | ~1s |
| **Max Code Size** | 512 KB | 24 KB | 10 MB (programs) | Unlimited |

### Key Differentiators

1. **Agent-Native**: `agent_get_score()` and `agent_is_registered()` are first-class host functions — no oracle or external contract needed to check identity/reputation
2. **Lightweight**: Entire node with VM is 17 MB. Designed to run on personal PCs and edge devices alongside AI agents
3. **Rust-Only**: No new language. Rust contracts compile to Wasm with the standard toolchain
4. **Minimal Overhead**: wasmer singlepass provides fast AOT compilation with minimal memory footprint

---

## Technical Specifications

| Spec | Value |
|------|-------|
| Runtime | wasmer 4.x with Singlepass compiler |
| Target | `wasm32-unknown-unknown` |
| Serialization | Borsh (payloads, storage) |
| Address Derivation | `blake3("claw_contract_v1:" \|\| deployer \|\| nonce)` |
| State Root | Merkle tree with `contract:` and `cstore:` prefixed leaves |
| Max Code Size | 512 KB |
| Default Fuel Limit | 10,000,000 |
| Host Functions | 17 (storage: 4, context: 6, agent: 2, token: 2, utility: 3) |
| Binary Size Impact | +4 MB (13 MB → 17 MB) |
| Backward Compatible | Yes (custom BorshDeserialize handles pre-VM chain data) |

---

## Roadmap

- [x] **Phase 1**: Core VM runtime (wasmer, host functions, fuel metering)
- [x] **Phase 2**: State integration (WorldState, handlers, RPC, CLI)
- [ ] **Phase 3**: `claw-sdk` Rust crate with `#[claw_contract]` proc macro, contract templates
- [ ] **Phase 4**: Explorer contract page, wallet contract interaction UI
