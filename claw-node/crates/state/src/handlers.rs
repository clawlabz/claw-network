//! Transaction handlers — one per TxType.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use borsh::BorshDeserialize;
use claw_types::block::BlockEvent;
use claw_types::receipt::{ReceiptEvent, TransactionReceipt};
use claw_types::state::*;
use claw_types::transaction::*;

use crate::error::StateError;
use crate::world::{
    WorldState, MAX_CATEGORY_LEN, MAX_DESCRIPTION_LEN, MAX_ENDPOINT_LEN, MAX_MEMO_LEN,
    MAX_METADATA_ENTRIES, MAX_NAME_LEN, MAX_SYMBOL_LEN,
};

/// Maximum time allowed for a single contract execution (deploy constructor or call).
/// Prevents infinite loops in pure Wasm computation that bypass host-function fuel metering.
const CONTRACT_EXECUTION_TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum number of VM executions that may run concurrently.
const VM_CONCURRENCY_LIMIT: usize = 16;

/// Global counter tracking how many VM threads are currently executing.
/// Using a simple Mutex<usize> counter as a counting semaphore.
static VM_CONCURRENCY_COUNTER: std::sync::OnceLock<Arc<Mutex<usize>>> =
    std::sync::OnceLock::new();

fn vm_concurrency_counter() -> &'static Arc<Mutex<usize>> {
    VM_CONCURRENCY_COUNTER.get_or_init(|| Arc::new(Mutex::new(0)))
}

/// RAII guard that decrements the VM concurrency counter on drop.
struct VmPermitGuard {
    counter: Arc<Mutex<usize>>,
}

impl Drop for VmPermitGuard {
    fn drop(&mut self) {
        let mut count = self.counter.lock().expect("VM concurrency counter mutex poisoned");
        *count = count.saturating_sub(1);
    }
}

/// Lightweight owned snapshot of chain state, allowing the execution thread to
/// be fully detached (no borrowed references).
struct ChainStateSnapshot {
    balances: std::collections::BTreeMap<[u8; 32], u128>,
    agent_scores: std::collections::BTreeMap<[u8; 32], u64>,
    registered_agents: std::collections::BTreeSet<[u8; 32]>,
    contract_storage: std::collections::BTreeMap<[u8; 32], std::collections::BTreeMap<Vec<u8>, Vec<u8>>>,
    /// Contract bytecode cache for cross-contract calls.
    contract_code: std::collections::BTreeMap<[u8; 32], Vec<u8>>,
}

impl ChainStateSnapshot {
    /// Capture chain state needed for VM execution.
    ///
    /// Balances are captured from the full `WorldState` (passed as a concrete
    /// reference) so the VM can query the balance of **any** address — not just
    /// caller and contract_address. The `addresses` slice is still used to
    /// pre-populate agent scores and registered-agent data, which are only needed
    /// for the execution context participants.
    fn capture(state: &dyn claw_vm::ChainState, addresses: &[[u8; 32]]) -> Self {
        let mut agent_scores = std::collections::BTreeMap::new();
        let mut registered_agents = std::collections::BTreeSet::new();
        for addr in addresses {
            agent_scores.insert(*addr, state.get_agent_score(addr));
            if state.get_agent_registered(addr) {
                registered_agents.insert(*addr);
            }
        }
        Self {
            // Empty: get_balance falls through to the full-state lookup below.
            // Populated by capture_from_world when called from handlers.
            balances: std::collections::BTreeMap::new(),
            agent_scores,
            registered_agents,
            contract_storage: std::collections::BTreeMap::new(),
            contract_code: std::collections::BTreeMap::new(),
        }
    }

    /// Capture a full snapshot from a `WorldState`, cloning the complete
    /// balances map so the detached thread can query any address.
    fn capture_from_world(state: &crate::world::WorldState, addresses: &[[u8; 32]]) -> Self {
        let mut snapshot = Self::capture(state, addresses);
        // Clone the full balances map — this is the key fix:
        // previously only caller+contract were captured, causing any third-party
        // balance query (e.g., Arena Pool checking a player's balance) to return 0.
        snapshot.balances = state.balances.clone();
        // Clone contract code for cross-contract call support.
        snapshot.contract_code = state.contract_code.clone();
        snapshot
    }
}

impl claw_vm::ChainState for ChainStateSnapshot {
    fn get_balance(&self, address: &[u8; 32]) -> u128 {
        self.balances.get(address).copied().unwrap_or(0)
    }
    fn get_agent_score(&self, address: &[u8; 32]) -> u64 {
        self.agent_scores.get(address).copied().unwrap_or(0)
    }
    fn get_agent_registered(&self, address: &[u8; 32]) -> bool {
        self.registered_agents.contains(address)
    }
    fn get_contract_storage(&self, contract: &[u8; 32], key: &[u8]) -> Option<Vec<u8>> {
        self.contract_storage
            .get(contract)
            .and_then(|m| m.get(key).cloned())
    }
    fn get_contract_code(&self, contract: &[u8; 32]) -> Option<Vec<u8>> {
        self.contract_code.get(contract).cloned()
    }
}

/// Execute a contract in a detached thread with a timeout.
///
/// The VM's fuel system only meters host function calls. Pure Wasm computation
/// (loops, arithmetic) is unmetered, so a malicious contract could infinite-loop.
/// This wrapper runs the execution in a detached `std::thread::spawn` and
/// abandons the thread on timeout. Unlike `thread::scope`, a detached thread
/// does not block the caller — on timeout, the caller returns immediately with
/// an error while the orphaned thread eventually terminates (wasmer memory is
/// freed when the thread drops).
fn execute_with_timeout(
    _engine: &claw_vm::VmEngine,
    code: &[u8],
    method: &str,
    args: &[u8],
    ctx: claw_vm::ExecutionContext,
    storage: std::collections::BTreeMap<Vec<u8>, Vec<u8>>,
    world: &WorldState,
) -> Result<claw_vm::ExecutionResult, claw_vm::VmError> {
    let (tx, rx) = std::sync::mpsc::channel();

    // Snapshot the FULL balance map from WorldState so the VM can query any
    // address's balance (e.g. a player's balance from inside an Arena Pool contract).
    // Previously only caller+contract were captured, causing third-party lookups
    // to return 0 silently.
    let snapshot = ChainStateSnapshot::capture_from_world(world, &[ctx.caller, ctx.contract_address]);
    let code = code.to_vec();
    let method = method.to_string();
    let args = args.to_vec();

    // Acquire a concurrency permit before spawning. If the limit is already
    // reached, reject immediately rather than queuing unbounded threads.
    {
        let counter = vm_concurrency_counter();
        let mut count = counter.lock().expect("VM concurrency counter mutex poisoned");
        if *count >= VM_CONCURRENCY_LIMIT {
            return Err(claw_vm::VmError::ExecutionFailed(
                "VM concurrency limit reached".to_string(),
            ));
        }
        *count += 1;
    }
    // Clone the Arc so the guard can be moved into the thread.
    let permit_counter = Arc::clone(vm_concurrency_counter());

    // Build contract code map and Arc<dyn ChainState> for cross-contract calls.
    let contract_code_map = std::sync::Arc::new(snapshot.contract_code.clone());
    let chain_state_arc: std::sync::Arc<dyn claw_vm::ChainState> = std::sync::Arc::new(snapshot);

    std::thread::spawn(move || {
        // Guard releases the permit when this thread completes or panics.
        let _permit = VmPermitGuard { counter: permit_counter };
        let engine = claw_vm::VmEngine::new();
        let result = engine.execute_with_cross_calls(
            &code, &method, &args, ctx, storage,
            chain_state_arc, contract_code_map,
        );
        // Ignore send error — receiver may have timed out and been dropped
        let _ = tx.send(result);
    });

    rx.recv_timeout(CONTRACT_EXECUTION_TIMEOUT)
        .unwrap_or(Err(claw_vm::VmError::ExecutionFailed(
            format!("contract execution timed out after {}s", CONTRACT_EXECUTION_TIMEOUT.as_secs()),
        )))
}

/// AgentRegister: register a new agent identity.
pub fn handle_agent_register(state: &mut WorldState, tx: &Transaction) -> Result<(), StateError> {
    let payload = AgentRegisterPayload::try_from_slice(&tx.payload)
        .map_err(|e| StateError::PayloadDeserialize(e.to_string()))?;

    if state.agents.contains_key(&tx.from) {
        return Err(StateError::AgentAlreadyRegistered);
    }

    if payload.name.is_empty() || payload.name.len() > MAX_NAME_LEN {
        return Err(StateError::NameTooLong {
            len: payload.name.len(),
            max: MAX_NAME_LEN,
        });
    }

    if payload.metadata.len() > MAX_METADATA_ENTRIES {
        return Err(StateError::MetadataTooLarge {
            len: payload.metadata.len(),
            max: MAX_METADATA_ENTRIES,
        });
    }

    state.agents.insert(
        tx.from,
        AgentIdentity {
            address: tx.from,
            name: payload.name,
            metadata: payload.metadata,
            registered_at: state.block_height,
        },
    );

    Ok(())
}

/// Safe add helper: returns BalanceOverflow if addition would overflow u128.
#[inline]
fn safe_add(balance: u128, amount: u128) -> Result<u128, StateError> {
    balance
        .checked_add(amount)
        .ok_or(StateError::BalanceOverflow { amount, balance })
}

/// TokenTransfer: transfer native CLAW.
pub fn handle_token_transfer(state: &mut WorldState, tx: &Transaction) -> Result<(), StateError> {
    let payload = TokenTransferPayload::try_from_slice(&tx.payload)
        .map_err(|e| StateError::PayloadDeserialize(e.to_string()))?;

    if payload.amount == 0 {
        return Err(StateError::ZeroAmount);
    }

    let sender_bal = state.balances.get(&tx.from).copied().unwrap_or(0);
    if sender_bal < payload.amount {
        return Err(StateError::InsufficientBalance {
            need: payload.amount,
            have: sender_bal,
        });
    }

    // Overflow check on recipient balance
    let recipient_bal = state.balances.get(&payload.to).copied().unwrap_or(0);
    safe_add(recipient_bal, payload.amount)?;

    *state.balances.entry(tx.from).or_insert(0) -= payload.amount;
    *state.balances.entry(payload.to).or_insert(0) += payload.amount;

    Ok(())
}

/// TokenCreate: create a new custom token.
pub fn handle_token_create(state: &mut WorldState, tx: &Transaction) -> Result<(), StateError> {
    let payload = TokenCreatePayload::try_from_slice(&tx.payload)
        .map_err(|e| StateError::PayloadDeserialize(e.to_string()))?;

    if !state.agents.contains_key(&tx.from) {
        return Err(StateError::AgentNotRegistered);
    }

    if payload.total_supply == 0 {
        return Err(StateError::ZeroSupply);
    }

    if payload.name.is_empty() || payload.name.len() > MAX_NAME_LEN {
        return Err(StateError::NameTooLong {
            len: payload.name.len(),
            max: MAX_NAME_LEN,
        });
    }

    if payload.symbol.is_empty() || payload.symbol.len() > MAX_SYMBOL_LEN {
        return Err(StateError::SymbolTooLong {
            len: payload.symbol.len(),
            max: MAX_SYMBOL_LEN,
        });
    }

    // Token ID = blake3(sender || name || nonce)
    let mut id_input = Vec::new();
    id_input.extend_from_slice(&tx.from);
    id_input.extend_from_slice(payload.name.as_bytes());
    id_input.extend_from_slice(&tx.nonce.to_le_bytes());
    let token_id: [u8; 32] = *blake3::hash(&id_input).as_bytes();

    if state.tokens.contains_key(&token_id) {
        return Err(StateError::TokenAlreadyExists);
    }

    state.tokens.insert(
        token_id,
        TokenDef {
            id: token_id,
            name: payload.name,
            symbol: payload.symbol,
            decimals: payload.decimals,
            total_supply: payload.total_supply,
            issuer: tx.from,
        },
    );

    // Credit entire supply to issuer
    state
        .token_balances
        .insert((tx.from, token_id), payload.total_supply);

    Ok(())
}

/// TokenMintTransfer: transfer a custom token.
///
/// If the sender does not hold enough tokens but has an allowance from a token
/// holder, the allowance is consumed instead (ERC-20 style `transferFrom`).
pub fn handle_token_mint_transfer(
    state: &mut WorldState,
    tx: &Transaction,
) -> Result<(), StateError> {
    let payload = TokenMintTransferPayload::try_from_slice(&tx.payload)
        .map_err(|e| StateError::PayloadDeserialize(e.to_string()))?;

    if payload.amount == 0 {
        return Err(StateError::ZeroAmount);
    }

    if payload.token_id == NATIVE_TOKEN_ID {
        return Err(StateError::NativeTokenIdForCustom);
    }

    if !state.tokens.contains_key(&payload.token_id) {
        return Err(StateError::TokenNotFound);
    }

    let sender_bal = state
        .token_balances
        .get(&(tx.from, payload.token_id))
        .copied()
        .unwrap_or(0);

    if sender_bal < payload.amount {
        return Err(StateError::InsufficientBalance {
            need: payload.amount,
            have: sender_bal,
        });
    }

    // Overflow check on recipient token balance
    let recipient_bal = state
        .token_balances
        .get(&(payload.to, payload.token_id))
        .copied()
        .unwrap_or(0);
    safe_add(recipient_bal, payload.amount)?;

    *state
        .token_balances
        .entry((tx.from, payload.token_id))
        .or_insert(0) -= payload.amount;
    *state
        .token_balances
        .entry((payload.to, payload.token_id))
        .or_insert(0) += payload.amount;

    Ok(())
}

/// ReputationAttest: unconditionally rejected.
///
/// DEPRECATED: This transaction type is disabled. All callers must migrate to
/// PlatformActivityReport (tx type 11).
pub fn handle_reputation_attest(
    _state: &mut WorldState,
    _tx: &Transaction,
) -> Result<(), StateError> {
    Err(StateError::StakeError(
        "ReputationAttest is deprecated — use PlatformActivityReport".into(),
    ))
}

/// ServiceRegister: register or update a service.
pub fn handle_service_register(
    state: &mut WorldState,
    tx: &Transaction,
) -> Result<(), StateError> {
    let payload = ServiceRegisterPayload::try_from_slice(&tx.payload)
        .map_err(|e| StateError::PayloadDeserialize(e.to_string()))?;

    if !state.agents.contains_key(&tx.from) {
        return Err(StateError::AgentNotRegistered);
    }

    if payload.service_type.is_empty() || payload.service_type.len() > MAX_NAME_LEN {
        return Err(StateError::NameTooLong {
            len: payload.service_type.len(),
            max: MAX_NAME_LEN,
        });
    }

    if payload.description.len() > MAX_DESCRIPTION_LEN {
        return Err(StateError::DescriptionTooLong {
            len: payload.description.len(),
            max: MAX_DESCRIPTION_LEN,
        });
    }

    if payload.endpoint.len() > MAX_ENDPOINT_LEN {
        return Err(StateError::EndpointTooLong {
            len: payload.endpoint.len(),
            max: MAX_ENDPOINT_LEN,
        });
    }

    let key = (tx.from, payload.service_type.clone());

    state.services.insert(
        key,
        ServiceEntry {
            provider: tx.from,
            service_type: payload.service_type,
            description: payload.description,
            price_token: payload.price_token,
            price_amount: payload.price_amount,
            endpoint: payload.endpoint,
            active: payload.active,
        },
    );

    Ok(())
}

/// ContractDeploy: deploy a new smart contract.
///
/// Returns `(block_events, receipt)`:
/// - `block_events`: `BlockEvent::ContractEvent` entries produced during constructor execution.
/// - `receipt`: `TransactionReceipt` with execution details.
pub fn handle_contract_deploy(
    state: &mut WorldState,
    tx: &Transaction,
    tx_index: u32,
) -> Result<(Vec<BlockEvent>, TransactionReceipt), StateError> {
    let payload = ContractDeployPayload::try_from_slice(&tx.payload)
        .map_err(|e| StateError::PayloadDeserialize(e.to_string()))?;

    // Validate code size
    if payload.code.len() > claw_vm::MAX_CONTRACT_CODE_SIZE {
        return Err(StateError::ContractCodeTooLarge {
            size: payload.code.len(),
            max: claw_vm::MAX_CONTRACT_CODE_SIZE,
        });
    }

    // Derive contract address from deployer + nonce
    let nonce = state.nonces.get(&tx.from).copied().unwrap_or(0);
    let contract_address = claw_vm::VmEngine::derive_contract_address(&tx.from, nonce);

    // Check not already deployed
    if state.contracts.contains_key(&contract_address) {
        return Err(StateError::ContractAlreadyExists);
    }

    // Validate the Wasm module
    let engine = claw_vm::VmEngine::new();
    engine
        .validate(&payload.code)
        .map_err(|e| StateError::ContractExecutionFailed(e.to_string()))?;

    // Prepare contract metadata (do NOT insert into state yet)
    let code_hash = *blake3::hash(&payload.code).as_bytes();
    let instance = claw_vm::ContractInstance {
        address: contract_address,
        code_hash,
        creator: tx.from,
        deployed_at: state.block_height,
        admin: tx.from,
        previous_code_hash: None,
        upgrade_height: None,
        pending_upgrade: None,
    };

    // If init_method is specified, run the constructor and validate all
    // side-effects BEFORE committing anything to state.
    if !payload.init_method.is_empty() {
        let ctx = claw_vm::ExecutionContext::new_top_level(
            tx.from,
            contract_address,
            state.block_height,
            state.block_timestamp,
            0,
            claw_vm::DEFAULT_FUEL_LIMIT,
            false,
        );

        // Empty storage for a freshly deployed contract
        let storage = std::collections::BTreeMap::new();

        let result = execute_with_timeout(
                &engine,
                &payload.code,
                &payload.init_method,
                &payload.init_args,
                ctx,
                storage,
                state,
            )
            .map_err(|e| StateError::ContractExecutionFailed(e.to_string()))?;

        // Validate ALL transfers before applying any state changes.
        // Track cumulative spend to catch insufficient balance across multiple transfers.
        {
            let mut cumulative_spend: u128 = 0;
            let contract_bal = state.balances.get(&contract_address).copied().unwrap_or(0);
            for (_, amount) in &result.transfers {
                cumulative_spend = cumulative_spend.saturating_add(*amount);
                if contract_bal < cumulative_spend {
                    return Err(StateError::ContractTransferInsufficientBalance {
                        need: *amount,
                        have: contract_bal.saturating_sub(cumulative_spend.saturating_sub(*amount)),
                    });
                }
            }
        }

        // Build receipt from execution result
        let tx_hash = tx.hash();
        let receipt = TransactionReceipt {
            tx_hash,
            success: true,
            fuel_consumed: result.fuel_consumed,
            fuel_limit: claw_vm::DEFAULT_FUEL_LIMIT,
            return_data: result.return_data.clone(),
            error_message: None,
            events: result
                .events
                .iter()
                .map(|e| ReceiptEvent {
                    topic: e.topic.clone(),
                    data: e.data.clone(),
                })
                .collect(),
            logs: result.logs.clone(),
        };

        // Convert VM events to BlockEvent entries (collect before mutable borrows below)
        let contract_events: Vec<BlockEvent> = result
            .events
            .iter()
            .map(|e| BlockEvent::ContractEvent {
                contract: contract_address,
                tx_index,
                topic: e.topic.clone(),
                data: e.data.clone(),
            })
            .collect();

        // --- Atomic commit: everything succeeded, apply all changes ---

        // 1. Register contract
        state.contracts.insert(contract_address, instance);
        state
            .contract_code
            .insert(contract_address, payload.code.clone());

        // 2. Apply storage changes
        for (key, value) in result.storage_changes {
            match value {
                Some(v) => {
                    state.contract_storage.insert((contract_address, key), v);
                }
                None => {
                    state.contract_storage.remove(&(contract_address, key));
                }
            }
        }

        // 3. Apply token transfers
        for (to, amount) in result.transfers {
            *state.balances.entry(contract_address).or_insert(0) -= amount;
            *state.balances.entry(to).or_insert(0) += amount;
        }

        Ok((contract_events, receipt))
    } else {
        // No constructor — just register the contract with a minimal receipt
        state.contracts.insert(contract_address, instance);
        state
            .contract_code
            .insert(contract_address, payload.code.clone());

        let receipt = TransactionReceipt {
            tx_hash: tx.hash(),
            success: true,
            fuel_consumed: 0,
            fuel_limit: 0,
            return_data: Vec::new(),
            error_message: None,
            events: Vec::new(),
            logs: Vec::new(),
        };

        Ok((Vec::new(), receipt))
    }
}

/// ContractCall: call a method on a deployed smart contract.
///
/// Returns `(block_events, receipt)`:
/// - `block_events`: `BlockEvent::ContractEvent` entries produced during execution.
/// - `receipt`: `TransactionReceipt` with execution details.
pub fn handle_contract_call(
    state: &mut WorldState,
    tx: &Transaction,
    tx_index: u32,
) -> Result<(Vec<BlockEvent>, TransactionReceipt), StateError> {
    let payload = ContractCallPayload::try_from_slice(&tx.payload)
        .map_err(|e| StateError::PayloadDeserialize(e.to_string()))?;

    // Check contract exists
    if !state.contracts.contains_key(&payload.contract) {
        return Err(StateError::ContractNotFound(hex::encode(payload.contract)));
    }

    // Validate method name
    if payload.method.is_empty() || payload.method.len() > 128 {
        return Err(StateError::InvalidContractMethod(payload.method.clone()));
    }

    // Transfer value to contract if specified
    if payload.value > 0 {
        let caller_bal = state.balances.get(&tx.from).copied().unwrap_or(0);
        if caller_bal < payload.value {
            return Err(StateError::InsufficientBalance {
                need: payload.value,
                have: caller_bal,
            });
        }
        *state.balances.entry(tx.from).or_insert(0) -= payload.value;
        *state.balances.entry(payload.contract).or_insert(0) += payload.value;
    }

    // Get contract code (clone to avoid borrow conflict)
    let code = state
        .contract_code
        .get(&payload.contract)
        .ok_or_else(|| StateError::ContractNotFound(hex::encode(payload.contract)))?
        .clone();

    // Build storage snapshot for this contract
    let storage: std::collections::BTreeMap<Vec<u8>, Vec<u8>> = state
        .contract_storage
        .iter()
        .filter(|((addr, _), _)| addr == &payload.contract)
        .map(|((_, key), value)| (key.clone(), value.clone()))
        .collect();

    let ctx = claw_vm::ExecutionContext::new_top_level(
        tx.from,
        payload.contract,
        state.block_height,
        state.block_timestamp,
        payload.value,
        claw_vm::DEFAULT_FUEL_LIMIT,
        false,
    );

    let engine = claw_vm::VmEngine::new();
    let result = execute_with_timeout(
        &engine, &code, &payload.method, &payload.args, ctx, storage, state,
    )
        .map_err(|e| {
            // Refund value on execution failure
            if payload.value > 0 {
                *state.balances.entry(payload.contract).or_insert(0) -= payload.value;
                *state.balances.entry(tx.from).or_insert(0) += payload.value;
            }
            StateError::ContractExecutionFailed(e.to_string())
        })?;

    // Validate ALL transfers before applying any state changes.
    // Track cumulative spend to catch insufficient balance across multiple transfers.
    {
        let mut cumulative_spend: u128 = 0;
        let contract_bal = state.balances.get(&payload.contract).copied().unwrap_or(0);
        for (_, amount) in &result.transfers {
            cumulative_spend = cumulative_spend.saturating_add(*amount);
            if contract_bal < cumulative_spend {
                // Refund value sent to contract before returning error
                if payload.value > 0 {
                    *state.balances.entry(payload.contract).or_insert(0) -= payload.value;
                    *state.balances.entry(tx.from).or_insert(0) += payload.value;
                }
                return Err(StateError::ContractTransferInsufficientBalance {
                    need: *amount,
                    have: contract_bal.saturating_sub(cumulative_spend.saturating_sub(*amount)),
                });
            }
        }
    }

    // Build receipt from execution result
    let tx_hash = tx.hash();
    let receipt = TransactionReceipt {
        tx_hash,
        success: true,
        fuel_consumed: result.fuel_consumed,
        fuel_limit: claw_vm::DEFAULT_FUEL_LIMIT,
        return_data: result.return_data.clone(),
        error_message: None,
        events: result
            .events
            .iter()
            .map(|e| ReceiptEvent {
                topic: e.topic.clone(),
                data: e.data.clone(),
            })
            .collect(),
        logs: result.logs.clone(),
    };

    // Convert VM events to BlockEvent entries (collect before mutable borrows below)
    let contract_events: Vec<BlockEvent> = result
        .events
        .iter()
        .map(|e| BlockEvent::ContractEvent {
            contract: payload.contract,
            tx_index,
            topic: e.topic.clone(),
            data: e.data.clone(),
        })
        .collect();

    // --- Atomic commit: all validations passed, apply all changes ---

    // 1. Apply storage changes
    for (key, value) in result.storage_changes {
        match value {
            Some(v) => {
                state
                    .contract_storage
                    .insert((payload.contract, key), v);
            }
            None => {
                state
                    .contract_storage
                    .remove(&(payload.contract, key));
            }
        }
    }

    // 2. Apply token transfers
    for (to, amount) in result.transfers {
        *state.balances.entry(payload.contract).or_insert(0) -= amount;
        *state.balances.entry(to).or_insert(0) += amount;
    }

    Ok((contract_events, receipt))
}

/// Maximum action_type length for platform reports.
const MAX_ACTION_TYPE_LEN: usize = claw_types::state::MAX_ACTION_TYPE_LEN;

///// Epoch length in blocks (must match claw_consensus::EPOCH_LENGTH).
const EPOCH_LENGTH: u64 = 100;

/// Maximum action_count allowed per ActivityEntry in a PlatformActivityReport.
/// Prevents compromised platform agents from inflating Agent Scores via u32::MAX.
const MAX_ACTION_COUNT_PER_ENTRY: u32 = 10_000;

/// PlatformActivityReport: submit on-chain activity data from a platform.
///
/// Only Platform Agents (registered agents with >= 50,000 CLAW staked) can submit.
/// Each Platform Agent can submit at most once per epoch (100 blocks).
pub fn handle_platform_activity_report(
    state: &mut WorldState,
    tx: &Transaction,
) -> Result<(), StateError> {
    let payload = PlatformActivityReportPayload::try_from_slice(&tx.payload)
        .map_err(|e| StateError::PayloadDeserialize(e.to_string()))?;

    // Submitter must be a registered agent
    if !state.agents.contains_key(&tx.from) {
        return Err(StateError::AgentNotRegistered);
    }

    // Submitter must have >= 50,000 CLAW staked (Platform Agent threshold)
    let stake = state.stakes.get(&tx.from).copied().unwrap_or(0);
    if stake < claw_types::state::PLATFORM_AGENT_MIN_STAKE {
        return Err(StateError::PlatformStakeTooLow {
            need: claw_types::state::PLATFORM_AGENT_MIN_STAKE,
            have: stake,
        });
    }

    // Limit entries per report
    if payload.reports.len() > claw_types::state::MAX_ACTIVITY_ENTRIES {
        return Err(StateError::TooManyActivityEntries {
            len: payload.reports.len(),
            max: claw_types::state::MAX_ACTIVITY_ENTRIES,
        });
    }

    // Each Platform Agent can submit once per epoch
    let current_epoch = state.block_height / EPOCH_LENGTH;
    if state.platform_report_tracker.contains_key(&(tx.from, current_epoch)) {
        return Err(StateError::PlatformReportAlreadySubmitted);
    }

    // Check for duplicate agents in the report
    {
        let mut seen_agents = std::collections::BTreeSet::new();
        for entry in &payload.reports {
            if !seen_agents.insert(entry.agent) {
                return Err(StateError::DuplicateAgentInReport(
                    hex::encode(entry.agent),
                ));
            }
        }
    }

    // Validate each entry
    for entry in &payload.reports {
        if entry.action_type.len() > MAX_ACTION_TYPE_LEN {
            return Err(StateError::ActionTypeTooLong {
                len: entry.action_type.len(),
                max: MAX_ACTION_TYPE_LEN,
            });
        }
        if entry.action_count > MAX_ACTION_COUNT_PER_ENTRY {
            return Err(StateError::ActionCountTooHigh {
                count: entry.action_count,
                max: MAX_ACTION_COUNT_PER_ENTRY,
            });
        }
        if !state.agents.contains_key(&entry.agent) {
            return Err(StateError::AgentNotRegistered);
        }
    }

    // Apply: aggregate platform activity for each reported agent
    for entry in &payload.reports {
        let agg = state.platform_activity.entry(entry.agent).or_default();
        agg.total_actions = agg.total_actions.saturating_add(entry.action_count as u64);
        agg.platform_count = agg.platform_count.saturating_add(1);
    }

    // Mark this reporter as having submitted for this epoch
    state.platform_report_tracker.insert((tx.from, current_epoch), true);

    Ok(())
}

/// TokenApprove: set allowance for a spender on a custom token.
pub fn handle_token_approve(
    state: &mut WorldState,
    tx: &Transaction,
) -> Result<(), StateError> {
    let payload = TokenApprovePayload::try_from_slice(&tx.payload)
        .map_err(|e| StateError::PayloadDeserialize(e.to_string()))?;

    if payload.token_id == NATIVE_TOKEN_ID {
        return Err(StateError::NativeTokenIdForCustom);
    }

    if !state.tokens.contains_key(&payload.token_id) {
        return Err(StateError::TokenNotFound);
    }

    if payload.spender == tx.from {
        return Err(StateError::SelfApproval);
    }

    let key = (tx.from, payload.spender, payload.token_id);
    if payload.amount == 0 {
        // Revoke approval
        state.token_allowances.remove(&key);
    } else {
        state.token_allowances.insert(key, payload.amount);
    }

    Ok(())
}

/// TokenBurn: destroy tokens from the sender's balance, reducing total supply.
pub fn handle_token_burn(
    state: &mut WorldState,
    tx: &Transaction,
) -> Result<(), StateError> {
    let payload = TokenBurnPayload::try_from_slice(&tx.payload)
        .map_err(|e| StateError::PayloadDeserialize(e.to_string()))?;

    if payload.amount == 0 {
        return Err(StateError::ZeroAmount);
    }

    if payload.token_id == NATIVE_TOKEN_ID {
        return Err(StateError::NativeTokenIdForCustom);
    }

    let token = state.tokens.get(&payload.token_id)
        .ok_or(StateError::TokenNotFound)?;

    // Check sender has enough tokens
    let sender_bal = state
        .token_balances
        .get(&(tx.from, payload.token_id))
        .copied()
        .unwrap_or(0);

    if sender_bal < payload.amount {
        return Err(StateError::InsufficientBalance {
            need: payload.amount,
            have: sender_bal,
        });
    }

    // Check total_supply won't underflow
    if token.total_supply < payload.amount {
        return Err(StateError::BurnExceedsSupply {
            burn: payload.amount,
            supply: token.total_supply,
        });
    }

    // Deduct from sender balance
    *state
        .token_balances
        .entry((tx.from, payload.token_id))
        .or_insert(0) -= payload.amount;

    // Reduce total supply
    if let Some(token_def) = state.tokens.get_mut(&payload.token_id) {
        token_def.total_supply -= payload.amount;
    }

    Ok(())
}

/// Minimum stake required to become a validator (10,000 CLAW with 9 decimals).
const MIN_STAKE: u128 = 10_000_000_000_000;

/// StakeDeposit: lock CLAW as validator stake.
///
/// Supports delegated staking: if `payload.validator` is non-zero, stake is
/// recorded under the validator address (so they appear in the validator set),
/// while the delegation mapping tracks the owner for reward routing.
///
/// Backward compatibility: old payloads (48 bytes: 16 + 32) without commission_bps
/// are accepted and default to commission_bps = 10000 (validator keeps all).
pub fn handle_stake_deposit(state: &mut WorldState, tx: &Transaction) -> Result<(), StateError> {
    // Backward-compatible deserialization: old payloads are 48 bytes (amount:16 + validator:32),
    // new payloads are 50 bytes (amount:16 + validator:32 + commission_bps:2).
    let (amount, validator, commission_bps) = if tx.payload.len() == 48 {
        // Legacy payload without commission_bps
        let legacy = borsh::from_slice::<(u128, [u8; 32])>(&tx.payload)
            .map_err(|e| StateError::PayloadDeserialize(e.to_string()))?;
        (legacy.0, legacy.1, 10000u16) // default: validator keeps all
    } else {
        let payload = StakeDepositPayload::try_from_slice(&tx.payload)
            .map_err(|e| StateError::PayloadDeserialize(e.to_string()))?;
        (payload.amount, payload.validator, payload.commission_bps)
    };

    if amount == 0 {
        return Err(StateError::ZeroAmount);
    }

    if commission_bps > 10000 {
        return Err(StateError::StakeError(format!(
            "commission_bps {} exceeds maximum 10000",
            commission_bps
        )));
    }

    // Check sender has enough balance
    let sender_bal = state.balances.get(&tx.from).copied().unwrap_or(0);
    if sender_bal < amount {
        return Err(StateError::InsufficientBalance {
            need: amount,
            have: sender_bal,
        });
    }

    // Determine the validator address: delegate or self-stake
    let validator_addr = if validator == [0u8; 32] {
        tx.from // self-stake
    } else {
        validator // delegated
    };

    // Compute new total stake under the validator address
    let current_stake = state.stakes.get(&validator_addr).copied().unwrap_or(0);
    let new_stake = current_stake
        .checked_add(amount)
        .ok_or_else(|| StateError::StakeError("stake overflow".into()))?;

    // First-time stakers must meet the minimum stake
    if current_stake == 0 && new_stake < MIN_STAKE {
        return Err(StateError::StakeError(format!(
            "initial stake {} below minimum {}",
            new_stake, MIN_STAKE
        )));
    }

    // Deduct from sender balance, record stake under validator address
    *state.balances.entry(tx.from).or_insert(0) -= amount;
    state.stakes.insert(validator_addr, new_stake);

    // Single-owner delegation: validator → owner (for reward routing).
    // Only the owner or the validator itself may add stake.
    let existing_owner = state.stake_delegations.get(&validator_addr).copied();
    if let Some(owner) = existing_owner {
        if tx.from != owner && tx.from != validator_addr {
            // Refund the balance deduction before returning error
            *state.balances.entry(tx.from).or_insert(0) += amount;
            state.stakes.insert(validator_addr, current_stake);
            return Err(StateError::StakeError(
                "validator already has a different delegation owner".into(),
            ));
        }
    }
    state.stake_delegations.entry(validator_addr).or_insert(tx.from);

    // Only update commission if sender is the validator or the delegation owner
    if tx.from == validator_addr || existing_owner.is_none() || existing_owner == Some(tx.from) {
        state.stake_commissions.insert(validator_addr, commission_bps);
    }

    // Track per-user delegation (Cosmos-style: delegator → validator → amount)
    let user_entry = state.user_delegations.entry(tx.from).or_default();
    let prev_user_stake = user_entry.get(&validator_addr).copied().unwrap_or(0);
    user_entry.insert(validator_addr, prev_user_stake + amount);

    Ok(())
}

/// ChangeDelegation: transfer delegation of a validator stake to a new owner.
///
/// Only the current delegator (or the validator itself for self-stake) can
/// change delegation. The stake amount is unaffected — only the reward
/// routing and commission rate are updated.
pub fn handle_change_delegation(state: &mut WorldState, tx: &Transaction) -> Result<(), StateError> {
    let payload = ChangeDelegationPayload::try_from_slice(&tx.payload)
        .map_err(|e| StateError::PayloadDeserialize(e.to_string()))?;

    if payload.commission_bps > 10000 {
        return Err(StateError::StakeError(format!(
            "commission_bps {} exceeds maximum 10000",
            payload.commission_bps
        )));
    }

    if payload.new_owner == payload.validator {
        return Err(StateError::StakeError(
            "new_owner cannot be the same as validator (use self-stake instead)".into(),
        ));
    }

    // Validator must have an existing stake
    let stake = state.stakes.get(&payload.validator).copied().unwrap_or(0);
    if stake == 0 {
        return Err(StateError::StakeError(
            "validator has no active stake".into(),
        ));
    }

    // Authorization: only the current delegator can change delegation.
    // For self-stake (delegator == validator), the validator itself can change.
    // Validators CANNOT unilaterally redirect external delegations.
    let current_delegator = state
        .stake_delegations
        .get(&payload.validator)
        .copied()
        .unwrap_or(payload.validator); // no delegation record = self-stake

    let is_self_stake = current_delegator == payload.validator;
    if tx.from != current_delegator && !(is_self_stake && tx.from == payload.validator) {
        return Err(StateError::StakeError(
            "not authorized: only the current delegator can change delegation".into(),
        ));
    }

    // Update delegation to new owner
    state
        .stake_delegations
        .insert(payload.validator, payload.new_owner);

    // Update commission rate
    state
        .stake_commissions
        .insert(payload.validator, payload.commission_bps);

    Ok(())
}

/// StakeWithdraw: begin unbonding stake (starts countdown to claim).
///
/// Supports delegated staking: the sender can unstake if they are either
/// the validator themselves (self-stake) or the delegated owner. The stake
/// is looked up under the appropriate validator address.
pub fn handle_stake_withdraw(state: &mut WorldState, tx: &Transaction) -> Result<(), StateError> {
    // Backward compat: old payloads are 16 bytes (amount only), new are 48 (amount + validator)
    let payload = if tx.payload.len() == 16 {
        StakeWithdrawPayload {
            amount: u128::from_le_bytes(tx.payload[..16].try_into().expect("payload is exactly 16 bytes — checked above")),
            validator: [0u8; 32],
        }
    } else {
        StakeWithdrawPayload::try_from_slice(&tx.payload)
            .map_err(|e| StateError::PayloadDeserialize(e.to_string()))?
    };

    if payload.amount == 0 {
        return Err(StateError::ZeroAmount);
    }

    // Determine the validator address whose stake to withdraw from.
    // If payload.validator is specified (non-zero), use it directly.
    // Otherwise fall back to tx.from (self-unstake) or find delegation.
    let validator_addr = if payload.validator != [0u8; 32] {
        // Explicit validator specified — verify sender is authorized
        let owner = state.stake_delegations.get(&payload.validator);
        if owner != Some(&tx.from) && payload.validator != tx.from {
            return Err(StateError::StakeError(
                "not authorized to unstake from this validator".into(),
            ));
        }
        payload.validator
    } else if state.stakes.get(&tx.from).copied().unwrap_or(0) > 0 {
        tx.from
    } else {
        let delegated_validator = state
            .stake_delegations
            .iter()
            .find(|(_, owner)| **owner == tx.from)
            .map(|(validator, _)| *validator);
        match delegated_validator {
            Some(v) => v,
            None => {
                return Err(StateError::StakeError(
                    "no stake found for this address (neither direct nor delegated)".into(),
                ));
            }
        }
    };

    let current_stake = state.stakes.get(&validator_addr).copied().unwrap_or(0);
    if payload.amount > current_stake {
        return Err(StateError::StakeError(format!(
            "unstake {} exceeds staked amount {}",
            payload.amount, current_stake
        )));
    }

    let remaining = current_stake - payload.amount;

    // If remaining stake is nonzero but below minimum, reject (must withdraw all)
    if remaining > 0 && remaining < MIN_STAKE {
        return Err(StateError::StakeError(format!(
            "remaining stake {} would be below minimum {}; withdraw all or leave at least {}",
            remaining, MIN_STAKE, MIN_STAKE
        )));
    }

    // Update or remove stake
    if remaining == 0 {
        state.stakes.remove(&validator_addr);
        state.stake_delegations.remove(&validator_addr);
        state.stake_commissions.remove(&validator_addr);
    } else {
        state.stakes.insert(validator_addr, remaining);
    }

    // Update per-user delegation tracking
    if let Some(user_entry) = state.user_delegations.get_mut(&tx.from) {
        let prev = user_entry.get(&validator_addr).copied().unwrap_or(0);
        let user_remaining = prev.saturating_sub(payload.amount);
        if user_remaining == 0 {
            user_entry.remove(&validator_addr);
            if user_entry.is_empty() {
                state.user_delegations.remove(&tx.from);
            }
        } else {
            user_entry.insert(validator_addr, user_remaining);
        }
    }

    // Cap unbonding entries per address to prevent spam
    const MAX_UNBONDING_PER_ADDRESS: usize = 100;
    let pending = state.unbonding_queue.iter().filter(|e| e.address == validator_addr).count();
    if pending >= MAX_UNBONDING_PER_ADDRESS {
        return Err(StateError::StakeError("too many pending unbonding entries".into()));
    }

    // Create unbonding entry — funds return to the sender (owner)
    let release_height = state.block_height + UNBONDING_PERIOD_BLOCKS;
    state.unbonding_queue.push(UnbondingEntry {
        address: tx.from,
        amount: payload.amount,
        release_height,
    });

    Ok(())
}

/// StakeClaim: claim all mature unbonding entries, crediting balance.
pub fn handle_stake_claim(state: &mut WorldState, tx: &Transaction) -> Result<(), StateError> {
    // StakeClaimPayload is a unit struct — we still deserialize to validate the payload
    let _payload = StakeClaimPayload::try_from_slice(&tx.payload)
        .map_err(|e| StateError::PayloadDeserialize(e.to_string()))?;

    let current_height = state.block_height;

    // Find all claimable entries for this sender
    let mut total_claimed: u128 = 0;
    let mut remaining_queue = Vec::new();

    for entry in std::mem::take(&mut state.unbonding_queue) {
        if entry.address == tx.from && entry.release_height <= current_height {
            total_claimed = total_claimed
                .checked_add(entry.amount)
                .ok_or_else(|| StateError::StakeError("claim overflow".into()))?;
        } else {
            remaining_queue.push(entry);
        }
    }

    if total_claimed == 0 {
        return Err(StateError::NoClaimableUnbonding);
    }

    state.unbonding_queue = remaining_queue;

    // Credit claimed amount back to balance
    *state.balances.entry(tx.from).or_insert(0) += total_claimed;

    Ok(())
}

/// MinerRegister: register a new miner on the network.
///
/// Validates tier, IP address length, name length, subnet sybil limits,
/// and creates the MinerInfo entry in state.
pub fn handle_miner_register(state: &mut WorldState, tx: &Transaction) -> Result<(), StateError> {
    let payload = MinerRegisterPayload::try_from_slice(&tx.payload)
        .map_err(|e| StateError::PayloadDeserialize(e.to_string()))?;

    // Must not already be registered
    if state.miners.contains_key(&tx.from) {
        return Err(StateError::MinerAlreadyRegistered);
    }

    // Only tier 1 (Online) is supported in Phase 1
    if payload.tier != 1 {
        return Err(StateError::InvalidMinerTier(payload.tier));
    }

    // Validate name length
    if payload.name.is_empty() || payload.name.len() > MAX_NAME_LEN {
        return Err(StateError::MinerNameTooLong {
            len: payload.name.len(),
            max: MAX_NAME_LEN,
        });
    }

    // Validate IP address length (4 for IPv4, 16 for IPv6)
    if payload.ip_addr.len() != 4 && payload.ip_addr.len() != 16 {
        return Err(StateError::InvalidIpLength(payload.ip_addr.len()));
    }

    // Extract /24 prefix (first 3 bytes) for sybil protection
    let ip_prefix = payload.ip_addr[..3].to_vec();

    // Count ALL registered miners in the same /24 subnet (not just active).
    // Using all registered miners prevents sybil bypass via active=false (V3).
    let same_subnet_count = state
        .miners
        .values()
        .filter(|m| m.ip_prefix == ip_prefix)
        .count();
    if same_subnet_count >= MAX_MINERS_PER_SUBNET {
        return Err(StateError::SubnetLimitReached {
            max: MAX_MINERS_PER_SUBNET,
        });
    }

    let v3_active = state.block_height >= CHECKIN_V3_HEIGHT;
    state.miners.insert(
        tx.from,
        MinerInfo {
            address: tx.from,
            tier: MinerTier::Online,
            name: payload.name,
            registered_at: state.block_height,
            last_heartbeat: if v3_active { 0 } else { state.block_height },
            ip_prefix,
            // V3: new miners start inactive, must checkin first
            active: !v3_active,
            reputation_bps: REPUTATION_NEWCOMER_BPS,
            pending_rewards: 0,
            pending_epoch: 0,
            epoch_attendance: 0,
            consecutive_misses: 0,
            // V3: 0 = never checked in
            last_checkin_epoch: 0,
        },
    );

    Ok(())
}

/// MinerHeartbeat: submit a periodic heartbeat to prove liveness.
///
/// Gas-free. Updates last_heartbeat, deduplicates by interval window,
/// and upgrades reputation based on miner age.
///
/// After HEARTBEAT_V2_HEIGHT, uses epoch-based check-in with shorter interval.
/// After CHECKIN_V3_HEIGHT, this tx type is rejected — miners use P2P checkin instead.
pub fn handle_miner_heartbeat(state: &mut WorldState, tx: &Transaction) -> Result<(), StateError> {
    // V3: reject MinerHeartbeat transactions entirely
    if state.block_height >= CHECKIN_V3_HEIGHT {
        return Err(StateError::HeartbeatDeprecated);
    }

    let _payload = MinerHeartbeatPayload::try_from_slice(&tx.payload)
        .map_err(|e| StateError::PayloadDeserialize(e.to_string()))?;

    // Must be a registered miner
    let miner = state
        .miners
        .get(&tx.from)
        .ok_or(StateError::MinerNotRegistered)?;

    if state.block_height >= HEARTBEAT_V2_HEIGHT {
        // --- V2 logic: epoch-number based check-in ---
        // Allow one heartbeat per epoch. No phase-locking — miners can check in
        // at any point within the epoch regardless of when they last checked in.
        let current_epoch = state.block_height / MINER_EPOCH_LENGTH;
        let last_epoch = miner.last_heartbeat / MINER_EPOCH_LENGTH;

        if last_epoch >= current_epoch {
            // Already checked in during this epoch
            return Err(StateError::HeartbeatTooEarly {
                next_allowed: (current_epoch + 1) * MINER_EPOCH_LENGTH,
                current: state.block_height,
            });
        }

        // Runtime dedup guard (epoch_checkins is borsh(skip), may be empty after restart,
        // but the last_epoch check above is the authoritative guard).
        if state.epoch_checkins.contains_key(&tx.from) {
            return Err(StateError::HeartbeatTooEarly {
                next_allowed: (current_epoch + 1) * MINER_EPOCH_LENGTH,
                current: state.block_height,
            });
        }

        // Update miner state
        let miner = state.miners.get_mut(&tx.from).expect("miner existence validated above");
        miner.last_heartbeat = state.block_height;

        // Record epoch check-in (runtime dedup cache, not used for settlement)
        state.epoch_checkins.insert(tx.from, true);

        // Also record in V1 tracker for backward compat during transition
        let v1_window = state.block_height / MINER_HEARTBEAT_INTERVAL_V1;
        state.miner_heartbeat_tracker.insert((tx.from, v1_window), true);
    } else {
        // --- V1 logic: legacy heartbeat ---
        let next_allowed = miner.last_heartbeat + MINER_HEARTBEAT_INTERVAL_V1;
        if state.block_height < next_allowed {
            return Err(StateError::HeartbeatTooEarly {
                next_allowed,
                current: state.block_height,
            });
        }

        let window = state.block_height / MINER_HEARTBEAT_INTERVAL_V1;
        if state.miner_heartbeat_tracker.contains_key(&(tx.from, window)) {
            return Err(StateError::HeartbeatTooEarly {
                next_allowed,
                current: state.block_height,
            });
        }

        // Compute reputation upgrade based on miner age
        let registered_at = miner.registered_at;
        let age = state.block_height.saturating_sub(registered_at);
        let new_reputation = if age >= BLOCKS_30_DAYS {
            REPUTATION_VETERAN_BPS
        } else if age >= BLOCKS_7_DAYS {
            REPUTATION_ESTABLISHED_BPS
        } else {
            REPUTATION_NEWCOMER_BPS
        };

        // Update miner state
        let miner = state.miners.get_mut(&tx.from).expect("miner existence validated above");
        miner.last_heartbeat = state.block_height;
        miner.active = true;
        miner.reputation_bps = new_reputation;

        // Record heartbeat in tracker
        state.miner_heartbeat_tracker.insert((tx.from, window), true);
    }

    Ok(())
}

/// ContractUpgradeAnnounce: announce intent to upgrade a contract (starts the timelock).
///
/// The caller must be the contract's `admin`. Stores a `PendingUpgrade` on the
/// contract instance. Re-announcing replaces any existing pending upgrade.
pub fn handle_contract_upgrade_announce(
    state: &mut WorldState,
    tx: &Transaction,
) -> Result<(), StateError> {
    let payload = ContractUpgradeAnnouncePayload::try_from_slice(&tx.payload)
        .map_err(|e| StateError::PayloadDeserialize(e.to_string()))?;

    // Contract must exist
    let instance = state
        .contracts
        .get(&payload.contract)
        .ok_or_else(|| StateError::ContractNotFound(hex::encode(payload.contract)))?;

    // Only the admin may announce an upgrade
    if tx.from != instance.admin {
        return Err(StateError::Unauthorized);
    }

    let announced_at = state.block_height;
    let ready_at = announced_at + crate::UPGRADE_DELAY_BLOCKS;

    let pending = claw_vm::PendingUpgrade {
        new_code_hash: payload.new_code_hash,
        announced_at,
        ready_at,
    };

    // Update in place (replace any existing pending upgrade)
    state
        .contracts
        .get_mut(&payload.contract)
        .expect("contract existence checked above")
        .pending_upgrade = Some(pending);

    Ok(())
}

/// ContractUpgradeExecute: execute a previously announced upgrade.
///
/// Requirements:
/// 1. Contract must exist.
/// 2. Caller must be the contract's `admin`.
/// 3. A `PendingUpgrade` must be present on the instance.
/// 4. `current_block_height >= pending.ready_at`.
/// 5. `blake3(new_code) == pending.new_code_hash`.
/// 6. New Wasm must be valid.
///
/// If `migrate_method` is `Some`, the migration is run against the existing
/// storage under the new code. Storage changes from the migration are applied.
pub fn handle_contract_upgrade_execute(
    state: &mut WorldState,
    tx: &Transaction,
    tx_index: u32,
) -> Result<(Vec<claw_types::block::BlockEvent>, TransactionReceipt), StateError> {
    let payload = ContractUpgradeExecutePayload::try_from_slice(&tx.payload)
        .map_err(|e| StateError::PayloadDeserialize(e.to_string()))?;

    // 1. Contract must exist
    let instance = state
        .contracts
        .get(&payload.contract)
        .ok_or_else(|| StateError::ContractNotFound(hex::encode(payload.contract)))?;

    // 2. Caller must be admin
    if tx.from != instance.admin {
        return Err(StateError::Unauthorized);
    }

    // 3. Pending upgrade must exist
    let pending = instance
        .pending_upgrade
        .clone()
        .ok_or(StateError::NoPendingUpgrade)?;

    // 4. Timelock: ready_at must have elapsed
    if state.block_height < pending.ready_at {
        return Err(StateError::UpgradeNotReady {
            current: state.block_height,
            ready_at: pending.ready_at,
        });
    }

    // 5. Verify code hash matches the announced commitment
    let submitted_hash = *blake3::hash(&payload.new_code).as_bytes();
    if submitted_hash != pending.new_code_hash {
        return Err(StateError::UpgradeCodeHashMismatch);
    }

    // 6. Validate code size and Wasm module
    if payload.new_code.len() > claw_vm::MAX_CONTRACT_CODE_SIZE {
        return Err(StateError::ContractCodeTooLarge {
            size: payload.new_code.len(),
            max: claw_vm::MAX_CONTRACT_CODE_SIZE,
        });
    }
    let engine = claw_vm::VmEngine::new();
    engine
        .validate(&payload.new_code)
        .map_err(|e| StateError::ContractExecutionFailed(e.to_string()))?;

    // Capture old code hash before modifying state
    let old_code_hash = state
        .contracts
        .get(&payload.contract)
        .expect("contract existence verified above")
        .code_hash;
    let upgrade_height = state.block_height;

    // --- Optional migration ---
    // Run before committing code swap so storage reads see the old state.
    let (migration_events, migration_fuel, migration_return, migration_receipt_events, migration_logs) = if let Some(ref migrate_method) = payload.migrate_method {
        // Snapshot existing storage for this contract
        let storage: std::collections::BTreeMap<Vec<u8>, Vec<u8>> = state
            .contract_storage
            .iter()
            .filter(|((addr, _), _)| addr == &payload.contract)
            .map(|((_, key), value)| (key.clone(), value.clone()))
            .collect();

        let ctx = claw_vm::ExecutionContext::new_top_level(
            tx.from,
            payload.contract,
            state.block_height,
            state.block_timestamp,
            0,
            claw_vm::DEFAULT_FUEL_LIMIT,
            false,
        );

        let result = execute_with_timeout(
            &engine,
            &payload.new_code,
            migrate_method,
            &payload.migrate_args,
            ctx,
            storage,
            state,
        )
        .map_err(|e| StateError::ContractExecutionFailed(e.to_string()))?;

        // Build receipt from migration execution result
        let receipt_events: Vec<ReceiptEvent> = result
            .events
            .iter()
            .map(|e| ReceiptEvent {
                topic: e.topic.clone(),
                data: e.data.clone(),
            })
            .collect();
        let receipt_fuel = result.fuel_consumed;
        let receipt_return = result.return_data.clone();
        let receipt_logs = result.logs.clone();

        // Collect migration events
        let events: Vec<claw_types::block::BlockEvent> = result
            .events
            .iter()
            .map(|e| claw_types::block::BlockEvent::ContractEvent {
                contract: payload.contract,
                tx_index,
                topic: e.topic.clone(),
                data: e.data.clone(),
            })
            .collect();

        // Apply migration storage changes
        for (key, value) in result.storage_changes {
            match value {
                Some(v) => {
                    state
                        .contract_storage
                        .insert((payload.contract, key), v);
                }
                None => {
                    state
                        .contract_storage
                        .remove(&(payload.contract, key));
                }
            }
        }

        (events, receipt_fuel, receipt_return, receipt_events, receipt_logs)
    } else {
        (Vec::new(), 0, Vec::new(), Vec::new(), Vec::new())
    };

    // --- Atomic commit: replace code and update instance ---

    // Replace bytecode
    state
        .contract_code
        .insert(payload.contract, payload.new_code.clone());

    // Update instance fields
    let instance = state
        .contracts
        .get_mut(&payload.contract)
        .expect("contract existence verified above");
    instance.previous_code_hash = Some(old_code_hash);
    instance.code_hash = submitted_hash;
    instance.upgrade_height = Some(upgrade_height);
    instance.pending_upgrade = None;

    let receipt = TransactionReceipt {
        tx_hash: tx.hash(),
        success: true,
        fuel_consumed: migration_fuel,
        fuel_limit: claw_vm::DEFAULT_FUEL_LIMIT,
        return_data: migration_return,
        error_message: None,
        events: migration_receipt_events,
        logs: migration_logs,
    };

    Ok((migration_events, receipt))
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use claw_vm::ChainState as ChainStateTrait;
    use crate::world::WorldState;

    /// Build a WorldState with preset balances — no transactions required.
    fn world_with_balances(entries: &[([u8; 32], u128)]) -> WorldState {
        let mut state = WorldState::default();
        for (addr, bal) in entries {
            state.balances.insert(*addr, *bal);
        }
        state
    }

    /// After the fix: ChainStateSnapshot captured via `capture_from_world` must
    /// return the correct balance for ANY address in the world state, not only
    /// the caller and contract addresses that are passed explicitly.
    ///
    /// This is the key regression: before the fix, a contract calling
    /// `host_token_balance` for a third-party address (e.g. a player in Arena Pool)
    /// would always receive 0.
    #[test]
    fn snapshot_returns_balance_for_arbitrary_address() {
        let addr_a = [1u8; 32]; // caller
        let addr_b = [2u8; 32]; // contract_address
        let addr_c = [3u8; 32]; // third-party — e.g. a player whose balance the contract queries

        let world = world_with_balances(&[
            (addr_a, 1_000),
            (addr_b, 2_000),
            (addr_c, 3_000),
        ]);

        // Only pass caller+contract (the old explicit list), but the fix ensures
        // the full balance map is cloned, so addr_c is still accessible.
        let snapshot = ChainStateSnapshot::capture_from_world(&world, &[addr_a, addr_b]);

        assert_eq!(
            snapshot.get_balance(&addr_c),
            3_000,
            "snapshot must return the real balance for any address, not just caller/contract"
        );

        // Sanity: the explicitly passed addresses still work.
        assert_eq!(snapshot.get_balance(&addr_a), 1_000);
        assert_eq!(snapshot.get_balance(&addr_b), 2_000);
    }

    /// An address with no balance in WorldState must return 0, not panic.
    #[test]
    fn snapshot_returns_zero_for_unknown_address() {
        let world = world_with_balances(&[]);
        let snapshot = ChainStateSnapshot::capture_from_world(&world, &[]);
        assert_eq!(snapshot.get_balance(&[42u8; 32]), 0);
    }

    /// Snapshot must correctly reflect multiple arbitrary addresses simultaneously.
    #[test]
    fn snapshot_handles_many_addresses() {
        let addrs: Vec<[u8; 32]> = (0u8..10).map(|i| [i; 32]).collect();
        let entries: Vec<([u8; 32], u128)> = addrs.iter().copied().zip(1_000u128..).collect();
        let world = world_with_balances(&entries);

        // Pass only the first two as the "explicit" list.
        let snapshot = ChainStateSnapshot::capture_from_world(&world, &addrs[..2]);

        // All 10 addresses must be queryable.
        for (i, addr) in addrs.iter().enumerate() {
            assert_eq!(
                snapshot.get_balance(addr),
                1_000 + i as u128,
                "address index {i} balance mismatch"
            );
        }
    }

    /// Zero-balance address (balance == 0) must still return 0, not be absent.
    #[test]
    fn snapshot_zero_balance_address_returns_zero() {
        let addr = [7u8; 32];
        let world = world_with_balances(&[(addr, 0)]);
        let snapshot = ChainStateSnapshot::capture_from_world(&world, &[]);
        assert_eq!(snapshot.get_balance(&addr), 0);
    }
}
