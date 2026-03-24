//! `claw-test` — In-process test framework for ClawNetwork smart contracts.
//!
//! Provides [`TestEnv`], a lightweight sandbox that wraps [`VmEngine`] with
//! purely in-memory state, so contract developers can run full deploy→call
//! cycles without spinning up a node.
//!
//! # Design
//!
//! * [`TestEnv`] owns all state: balances, contract code, contract storage,
//!   block height/timestamp, and a per-deployer nonce counter.
//! * Each [`deploy`][TestEnv::deploy] compiles the Wasm, derives an address
//!   via [`VmEngine::derive_contract_address`], runs the `init` method, and
//!   commits the resulting storage changes.
//! * Each [`call`][TestEnv::call] looks up the stored Wasm, builds an
//!   [`ExecutionContext`], forwards the call to [`VmEngine::execute`], then
//!   applies storage changes and token transfers back into the env.
//! * The [`TestChainState`] adaptor (private) satisfies the [`ChainState`]
//!   trait by reading from the env's internal maps.
//!
//! # Immutability note
//!
//! The `call` and `deploy` methods take `&mut self` and return new values
//! rather than mutating shared references — there is no hidden aliasing.

use std::collections::BTreeMap;

use claw_vm::{ChainState, ExecutionContext, ExecutionResult, VmEngine};
pub use claw_vm::ContractEvent;

// ---------------------------------------------------------------------------
// Public result type
// ---------------------------------------------------------------------------

/// The outcome of a successful [`TestEnv::call`].
#[derive(Debug, Clone)]
pub struct CallResult {
    /// Raw bytes returned by the contract via `return_data`.
    pub return_data: Vec<u8>,
    /// Structured events emitted by the contract via `emit_event`.
    pub events: Vec<ContractEvent>,
    /// Fuel units consumed by this execution.
    pub fuel_consumed: u64,
}

impl CallResult {
    fn from_execution_result(r: ExecutionResult) -> Self {
        Self {
            return_data: r.return_data,
            events: r.events,
            fuel_consumed: r.fuel_consumed,
        }
    }
}

// ---------------------------------------------------------------------------
// TestEnv
// ---------------------------------------------------------------------------

/// Lightweight in-process environment for testing smart contracts.
///
/// All state is stored in plain Rust collections; there is no database or
/// network involved.
pub struct TestEnv {
    /// CLAW balances per address.
    balances: BTreeMap<[u8; 32], u128>,
    /// Wasm bytecode per deployed contract address.
    contract_code: BTreeMap<[u8; 32], Vec<u8>>,
    /// Contract storage: `(contract_address, key) → value`.
    storage: BTreeMap<([u8; 32], Vec<u8>), Vec<u8>>,
    /// Current block height passed to every execution.
    block_height: u64,
    /// Current block timestamp (Unix seconds) passed to every execution.
    block_timestamp: u64,
    /// Per-address deploy nonce (incremented each time an address deploys).
    deploy_nonces: BTreeMap<[u8; 32], u64>,
}

impl TestEnv {
    // -----------------------------------------------------------------------
    // Construction
    // -----------------------------------------------------------------------

    /// Create a new, empty test environment.
    ///
    /// Block height and timestamp both start at 0; all balances are 0.
    pub fn new() -> Self {
        Self {
            balances: BTreeMap::new(),
            contract_code: BTreeMap::new(),
            storage: BTreeMap::new(),
            block_height: 0,
            block_timestamp: 0,
            deploy_nonces: BTreeMap::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Balance management
    // -----------------------------------------------------------------------

    /// Set the CLAW balance for `address` to `amount`.
    pub fn set_balance(&mut self, address: [u8; 32], amount: u128) {
        self.balances.insert(address, amount);
    }

    /// Return the CLAW balance for `address` (0 if never set).
    pub fn get_balance(&self, address: [u8; 32]) -> u128 {
        self.balances.get(&address).copied().unwrap_or(0)
    }

    // -----------------------------------------------------------------------
    // Time / block
    // -----------------------------------------------------------------------

    /// Return the current block height.
    pub fn block_height(&self) -> u64 {
        self.block_height
    }

    /// Return the current block timestamp.
    pub fn block_timestamp(&self) -> u64 {
        self.block_timestamp
    }

    /// Advance the block height by 1.
    pub fn advance_block(&mut self) {
        self.block_height += 1;
    }

    /// Advance the block height by `n`.
    pub fn advance_blocks(&mut self, n: u64) {
        self.block_height += n;
    }

    /// Set the block timestamp to `ts` (Unix seconds).
    pub fn set_timestamp(&mut self, ts: u64) {
        self.block_timestamp = ts;
    }

    // -----------------------------------------------------------------------
    // Storage read
    // -----------------------------------------------------------------------

    /// Read a value from `contract`'s storage at `key`.
    ///
    /// Returns `None` if the key has never been written.
    pub fn get_storage(&self, contract: [u8; 32], key: &[u8]) -> Option<Vec<u8>> {
        self.storage.get(&(contract, key.to_vec())).cloned()
    }

    // -----------------------------------------------------------------------
    // Deploy
    // -----------------------------------------------------------------------

    /// Deploy a contract.
    ///
    /// Steps:
    /// 1. Validate the Wasm (will reject garbage bytes early).
    /// 2. Derive a deterministic contract address from `(deployer, nonce)`.
    /// 3. Run `init_method` with `init_args` as an ordinary call.
    /// 4. Commit any storage changes produced by `init`.
    /// 5. Store the Wasm so future `call`s can use it.
    ///
    /// Returns the derived contract address on success.
    pub fn deploy(
        &mut self,
        deployer: [u8; 32],
        wasm: &[u8],
        init_method: &str,
        init_args: &[u8],
    ) -> Result<[u8; 32], String> {
        // 1. Pre-validate: reject empty / non-Wasm bytes before computing an address.
        if wasm.is_empty() {
            return Err("deploy: wasm is empty".to_string());
        }

        let engine = VmEngine::new();
        engine
            .validate(wasm)
            .map_err(|e| format!("deploy: validation failed: {e}"))?;

        // 2. Derive address.
        let nonce = self.deploy_nonces.get(&deployer).copied().unwrap_or(0);
        let contract_address = VmEngine::derive_contract_address(&deployer, nonce);

        // 3. Build execution context for init.
        let context = ExecutionContext {
            caller: deployer,
            contract_address,
            block_height: self.block_height,
            block_timestamp: self.block_timestamp,
            value: 0,
            fuel_limit: claw_vm::DEFAULT_FUEL_LIMIT,
            read_only: false,
        };

        // Snapshot current contract storage for this contract (empty at deploy time).
        let storage_snapshot = self.contract_storage_snapshot(contract_address);

        // 4. Run init.
        let chain_state = TestChainState {
            balances: self.balances.clone(),
        };
        let result = engine
            .execute(wasm, init_method, init_args, context, storage_snapshot, &chain_state)
            .map_err(|e| format!("deploy: init execution failed: {e}"))?;

        // 5. Commit storage changes from init.
        self.apply_storage_changes(contract_address, &result.storage_changes);

        // 6. Commit any token transfers from init.
        self.apply_transfers(contract_address, &result.transfers);

        // 7. Store the Wasm code.
        self.contract_code.insert(contract_address, wasm.to_vec());

        // 8. Bump nonce.
        self.deploy_nonces.insert(deployer, nonce + 1);

        Ok(contract_address)
    }

    // -----------------------------------------------------------------------
    // Call
    // -----------------------------------------------------------------------

    /// Call a method on a deployed contract.
    ///
    /// `value` is deducted from `caller`'s balance and added to `contract`'s
    /// balance before execution.  If the caller has insufficient balance the
    /// call is rejected without executing any Wasm.
    ///
    /// Storage changes and token transfers produced by the contract are
    /// applied to the env after a successful execution.
    pub fn call(
        &mut self,
        caller: [u8; 32],
        contract: [u8; 32],
        method: &str,
        args: &[u8],
        value: u128,
    ) -> Result<CallResult, String> {
        // Look up the Wasm code (verifies the contract was deployed).
        let wasm = self
            .contract_code
            .get(&contract)
            .cloned()
            .ok_or_else(|| format!("call: contract {:?} not deployed", contract))?;

        // Check caller balance for the attached value.
        if value > 0 {
            let caller_bal = self.balances.get(&caller).copied().unwrap_or(0);
            if caller_bal < value {
                return Err(format!(
                    "call: insufficient balance (have {caller_bal}, need {value})"
                ));
            }
            // Deduct from caller, credit to contract (before execution — value is
            // visible inside the Wasm via `value_lo`/`value_hi`).
            *self.balances.entry(caller).or_insert(0) -= value;
            *self.balances.entry(contract).or_insert(0) += value;
        }

        // Build context.
        let context = ExecutionContext {
            caller,
            contract_address: contract,
            block_height: self.block_height,
            block_timestamp: self.block_timestamp,
            value,
            fuel_limit: claw_vm::DEFAULT_FUEL_LIMIT,
            read_only: false,
        };

        // Snapshot current storage for this contract.
        let storage_snapshot = self.contract_storage_snapshot(contract);

        let chain_state = TestChainState {
            balances: self.balances.clone(),
        };

        let engine = VmEngine::new();
        let result = engine
            .execute(&wasm, method, args, context, storage_snapshot, &chain_state)
            .map_err(|e| format!("call: execution failed: {e}"))?;

        // Apply storage changes.
        self.apply_storage_changes(contract, &result.storage_changes);

        // Apply contract-initiated token transfers.
        self.apply_transfers(contract, &result.transfers);

        Ok(CallResult::from_execution_result(result))
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Return a `BTreeMap<Vec<u8>, Vec<u8>>` snapshot of the storage for a
    /// single contract — the format expected by `VmEngine::execute`.
    fn contract_storage_snapshot(
        &self,
        contract: [u8; 32],
    ) -> BTreeMap<Vec<u8>, Vec<u8>> {
        self.storage
            .iter()
            .filter_map(|((addr, key), val)| {
                if *addr == contract {
                    Some((key.clone(), val.clone()))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Apply a list of `(key, Option<value>)` storage changes for a contract.
    ///
    /// `None` means delete; `Some(v)` means insert/update.  Immutably builds a
    /// new state by replacing the affected keys — we write into the BTreeMap
    /// rather than cloning the whole thing to avoid unnecessary allocation.
    fn apply_storage_changes(
        &mut self,
        contract: [u8; 32],
        changes: &[(Vec<u8>, Option<Vec<u8>>)],
    ) {
        for (key, maybe_val) in changes {
            match maybe_val {
                Some(val) => {
                    self.storage.insert((contract, key.clone()), val.clone());
                }
                None => {
                    self.storage.remove(&(contract, key.clone()));
                }
            }
        }
    }

    /// Apply token transfers emitted by `contract` during execution.
    ///
    /// Each transfer moves `amount` from the contract's balance to the
    /// recipient.  If the contract has insufficient funds the transfer is
    /// silently skipped (matching the behaviour of the production handler
    /// which guards against underflow).
    fn apply_transfers(&mut self, from: [u8; 32], transfers: &[([u8; 32], u128)]) {
        for (to, amount) in transfers {
            let sender_bal = self.balances.get(&from).copied().unwrap_or(0);
            if sender_bal >= *amount {
                *self.balances.entry(from).or_insert(0) -= amount;
                *self.balances.entry(*to).or_insert(0) += amount;
            }
        }
    }
}

impl Default for TestEnv {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// TestChainState
// ---------------------------------------------------------------------------

/// Minimal [`ChainState`] implementation backed by the env's balance map.
///
/// Agent scores and registration always return 0 / false — they are not
/// relevant for unit tests that do not exercise those host functions.
struct TestChainState {
    balances: BTreeMap<[u8; 32], u128>,
}

impl ChainState for TestChainState {
    fn get_balance(&self, address: &[u8; 32]) -> u128 {
        self.balances.get(address).copied().unwrap_or(0)
    }

    fn get_agent_score(&self, _address: &[u8; 32]) -> u64 {
        0
    }

    fn get_agent_registered(&self, _address: &[u8; 32]) -> bool {
        false
    }

    fn get_contract_storage(&self, _contract: &[u8; 32], _key: &[u8]) -> Option<Vec<u8>> {
        // The engine resolves storage via the snapshot passed to `execute`,
        // not via this trait method (which is only used for cross-contract
        // queries — not yet supported in the test framework).
        None
    }
}
