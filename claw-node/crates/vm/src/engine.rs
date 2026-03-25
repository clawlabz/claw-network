use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use wasmer::{imports, Function, FunctionEnv, Instance, Module, Store, Value};
use wasmer::Singlepass;

use crate::error::VmError;
use crate::host::{self, HostEnv};
use crate::types::{ChainState, ExecutionContext, ExecutionResult};
use crate::constants::MAX_CONTRACT_CODE_SIZE;

pub struct VmEngine;

impl VmEngine {
    pub fn new() -> Self {
        Self
    }

    /// Derive contract address from deployer + nonce.
    pub fn derive_contract_address(deployer: &[u8; 32], nonce: u64) -> [u8; 32] {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"claw_contract_v1:");
        buf.extend_from_slice(deployer);
        buf.extend_from_slice(&nonce.to_le_bytes());
        *blake3::hash(&buf).as_bytes()
    }

    /// Validate Wasm bytecode by attempting compilation.
    pub fn validate(&self, code: &[u8]) -> Result<(), VmError> {
        if code.len() > MAX_CONTRACT_CODE_SIZE {
            return Err(VmError::CodeTooLarge {
                size: code.len(),
                max: MAX_CONTRACT_CODE_SIZE,
            });
        }
        let compiler = Singlepass::new();
        let store = Store::new(compiler);
        Module::new(&store, code).map_err(|e| VmError::InvalidModule(e.to_string()))?;
        Ok(())
    }

    /// Execute a contract method.
    pub fn execute(
        &self,
        code: &[u8],
        method: &str,
        args: &[u8],
        context: ExecutionContext,
        storage: BTreeMap<Vec<u8>, Vec<u8>>,
        chain_state: &dyn ChainState,
    ) -> Result<ExecutionResult, VmError> {
        // 1. Compile
        let compiler = Singlepass::new();
        let mut store = Store::new(compiler);
        let module =
            Module::new(&store, code).map_err(|e| VmError::CompilationFailed(e.to_string()))?;

        // 2. Build chain state snapshots
        // Collect addresses from storage keys, context, etc.
        let mut all_addresses: Vec<[u8; 32]> = Vec::new();
        all_addresses.push(context.caller);
        all_addresses.push(context.contract_address);

        let mut balances_map = BTreeMap::new();
        let mut scores_map = BTreeMap::new();
        let mut registered_set = BTreeSet::new();

        for addr in &all_addresses {
            balances_map.insert(*addr, chain_state.get_balance(addr));
            scores_map.insert(*addr, chain_state.get_agent_score(addr));
            if chain_state.get_agent_registered(addr) {
                registered_set.insert(*addr);
            }
        }

        // 3. Create host environment
        let fuel_limit = context.fuel_limit;
        let call_depth = context.call_depth;
        let executing_contracts = context.executing_contracts.clone();

        // Build contract code map from chain_state for cross-contract calls.
        // For top-level calls this is populated by the handler; for nested calls
        // the parent's code map is forwarded.
        let contract_code_map: Arc<BTreeMap<[u8; 32], Vec<u8>>> = Arc::new(BTreeMap::new());

        // Wrap chain_state as Arc<dyn ChainState> for HostEnv
        // We create a thin wrapper that delegates to the reference.
        // Since HostEnv needs an owned Arc, we build a snapshot-based one.
        let chain_state_arc: Option<Arc<dyn ChainState>> = None;
        // Note: chain_state_arc and contract_code are set externally via
        // execute_with_cross_call_support() or by the caller after HostEnv creation.

        let host_env = HostEnv {
            context,
            storage: Arc::new(Mutex::new(storage)),
            storage_changes: Arc::new(Mutex::new(Vec::new())),
            logs: Arc::new(Mutex::new(Vec::new())),
            transfers: Arc::new(Mutex::new(Vec::new())),
            return_data: Arc::new(Mutex::new(Vec::new())),
            events: Arc::new(Mutex::new(Vec::new())),
            balances: Arc::new(balances_map),
            agent_scores: Arc::new(scores_map),
            registered_agents: Arc::new(registered_set),
            memory: None,
            fuel_remaining: Arc::new(Mutex::new(fuel_limit)),
            fuel_consumed: Arc::new(Mutex::new(0)),
            fuel_limit,
            chain_state: chain_state_arc,
            contract_code: contract_code_map,
            last_cross_call_return: Arc::new(Mutex::new(Vec::new())),
            call_depth,
            executing_contracts,
        };

        let func_env = FunctionEnv::new(&mut store, host_env);

        // 4. Build imports
        let import_object = imports! {
            "env" => {
                "storage_read" => Function::new_typed_with_env(&mut store, &func_env, host::host_storage_read),
                "storage_write" => Function::new_typed_with_env(&mut store, &func_env, host::host_storage_write),
                "storage_has" => Function::new_typed_with_env(&mut store, &func_env, host::host_storage_has),
                "storage_delete" => Function::new_typed_with_env(&mut store, &func_env, host::host_storage_delete),
                "caller" => Function::new_typed_with_env(&mut store, &func_env, host::host_caller),
                "block_height" => Function::new_typed_with_env(&mut store, &func_env, host::host_block_height),
                "block_timestamp" => Function::new_typed_with_env(&mut store, &func_env, host::host_block_timestamp),
                "contract_address" => Function::new_typed_with_env(&mut store, &func_env, host::host_contract_address),
                "value_lo" => Function::new_typed_with_env(&mut store, &func_env, host::host_value_lo),
                "value_hi" => Function::new_typed_with_env(&mut store, &func_env, host::host_value_hi),
                "agent_get_score" => Function::new_typed_with_env(&mut store, &func_env, host::host_agent_get_score),
                "agent_is_registered" => Function::new_typed_with_env(&mut store, &func_env, host::host_agent_is_registered),
                "token_balance" => Function::new_typed_with_env(&mut store, &func_env, host::host_token_balance),
                "token_balance_hi" => Function::new_typed_with_env(&mut store, &func_env, host::host_token_balance_hi),
                "token_transfer" => Function::new_typed_with_env(&mut store, &func_env, host::host_token_transfer),
                "log_msg" => Function::new_typed_with_env(&mut store, &func_env, host::host_log),
                "return_data" => Function::new_typed_with_env(&mut store, &func_env, host::host_return_data),
                "abort" => Function::new_typed_with_env(&mut store, &func_env, host::host_abort),
                "emit_event" => Function::new_typed_with_env(&mut store, &func_env, host::host_emit_event),
                "call_contract" => Function::new_typed_with_env(&mut store, &func_env, host::host_call_contract),
                "cross_call_return_data" => Function::new_typed_with_env(&mut store, &func_env, host::host_cross_call_return_data),
            }
        };

        // 5. Instantiate
        let instance = Instance::new(&mut store, &module, &import_object)
            .map_err(|e| VmError::InstantiationFailed(e.to_string()))?;

        // 6. Set memory reference in host env
        let memory = instance
            .exports
            .get_memory("memory")
            .map_err(|e| VmError::MemoryError(e.to_string()))?;
        func_env.as_mut(&mut store).memory = Some(memory.clone());

        // 7. If args are provided, write them to guest memory via alloc
        let args_ptr = if !args.is_empty() {
            if let Ok(alloc_fn) = instance.exports.get_function("alloc") {
                let result = alloc_fn
                    .call(&mut store, &[Value::I32(args.len() as i32)])
                    .map_err(|e| VmError::ExecutionFailed(format!("alloc failed: {e}")))?;
                let ptr = result[0].unwrap_i32() as u32;
                let mem_view = memory.view(&store);
                mem_view
                    .write(ptr as u64, args)
                    .map_err(|e| VmError::MemoryError(e.to_string()))?;
                Some(ptr)
            } else {
                None
            }
        } else {
            None
        };

        // 8. Call the method
        let func = instance
            .exports
            .get_function(method)
            .map_err(|_| VmError::MethodNotFound(method.to_string()))?;

        // Determine call args based on the function's actual signature.
        // SDK contracts export methods as (i32, i32) -> () for (args_ptr, args_len).
        // Simple/test contracts may export methods with no params: () -> ().
        let func_type = func.ty(&store);
        let func_params = func_type.params();
        let call_args = if func_params.len() == 2 {
            // SDK-style: always pass (ptr, len), even when args is empty → (0, 0)
            match args_ptr {
                Some(ptr) => vec![Value::I32(ptr as i32), Value::I32(args.len() as i32)],
                None => vec![Value::I32(0), Value::I32(0)],
            }
        } else {
            // No-param style (test/simple contracts)
            vec![]
        };

        let _result = func.call(&mut store, &call_args).map_err(|e| {
            let msg = e.to_string();
            let fuel_consumed = func_env
                .as_ref(&store)
                .fuel_consumed
                .lock()
                .map(|f| *f)
                .unwrap_or(0);

            if msg.contains("out of fuel") || msg.contains("fuel exhausted") {
                VmError::OutOfFuel {
                    used: fuel_consumed,
                    limit: fuel_limit,
                }
            } else if msg.contains("contract abort:") {
                let reason = msg
                    .split("contract abort:")
                    .nth(1)
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|| msg.clone());
                VmError::ContractAbort {
                    reason,
                    fuel_consumed,
                }
            } else {
                VmError::ExecutionFailed(format!("{msg} (fuel consumed: {fuel_consumed})"))
            }
        })?;

        // 9. Collect results
        let env_ref = func_env.as_ref(&store);
        let return_data = env_ref.return_data.lock().unwrap().clone();
        let storage_changes = env_ref.storage_changes.lock().unwrap().clone();
        let logs = env_ref.logs.lock().unwrap().clone();
        let transfers = env_ref.transfers.lock().unwrap().clone();
        let events = env_ref.events.lock().unwrap().clone();
        let fuel_consumed = *env_ref.fuel_consumed.lock().unwrap();

        Ok(ExecutionResult {
            return_data,
            fuel_consumed,
            storage_changes,
            logs,
            transfers,
            events,
        })
    }

    /// Execute a contract with cross-contract call support.
    ///
    /// Like `execute()` but injects `chain_state` and `contract_code` into the
    /// HostEnv so `host_call_contract` can spawn nested executions.
    pub fn execute_with_cross_calls(
        &self,
        code: &[u8],
        method: &str,
        args: &[u8],
        context: ExecutionContext,
        storage: BTreeMap<Vec<u8>, Vec<u8>>,
        chain_state: Arc<dyn ChainState>,
        contract_code: Arc<BTreeMap<[u8; 32], Vec<u8>>>,
    ) -> Result<ExecutionResult, VmError> {
        // 1. Compile
        let compiler = Singlepass::new();
        let mut store = Store::new(compiler);
        let module =
            Module::new(&store, code).map_err(|e| VmError::CompilationFailed(e.to_string()))?;

        // 2. Build chain state snapshots for balance/agent queries
        let mut all_addresses: Vec<[u8; 32]> = Vec::new();
        all_addresses.push(context.caller);
        all_addresses.push(context.contract_address);

        let mut balances_map = BTreeMap::new();
        let mut scores_map = BTreeMap::new();
        let mut registered_set = BTreeSet::new();

        for addr in &all_addresses {
            balances_map.insert(*addr, chain_state.get_balance(addr));
            scores_map.insert(*addr, chain_state.get_agent_score(addr));
            if chain_state.get_agent_registered(addr) {
                registered_set.insert(*addr);
            }
        }

        // 3. Create host environment with cross-call support
        let fuel_limit = context.fuel_limit;
        let call_depth = context.call_depth;
        let executing_contracts = context.executing_contracts.clone();

        let host_env = HostEnv {
            context,
            storage: Arc::new(Mutex::new(storage)),
            storage_changes: Arc::new(Mutex::new(Vec::new())),
            logs: Arc::new(Mutex::new(Vec::new())),
            transfers: Arc::new(Mutex::new(Vec::new())),
            return_data: Arc::new(Mutex::new(Vec::new())),
            events: Arc::new(Mutex::new(Vec::new())),
            balances: Arc::new(balances_map),
            agent_scores: Arc::new(scores_map),
            registered_agents: Arc::new(registered_set),
            memory: None,
            fuel_remaining: Arc::new(Mutex::new(fuel_limit)),
            fuel_consumed: Arc::new(Mutex::new(0)),
            fuel_limit,
            chain_state: Some(chain_state),
            contract_code,
            last_cross_call_return: Arc::new(Mutex::new(Vec::new())),
            call_depth,
            executing_contracts,
        };

        let func_env = FunctionEnv::new(&mut store, host_env);

        // 4. Build imports (same as execute)
        let import_object = imports! {
            "env" => {
                "storage_read" => Function::new_typed_with_env(&mut store, &func_env, host::host_storage_read),
                "storage_write" => Function::new_typed_with_env(&mut store, &func_env, host::host_storage_write),
                "storage_has" => Function::new_typed_with_env(&mut store, &func_env, host::host_storage_has),
                "storage_delete" => Function::new_typed_with_env(&mut store, &func_env, host::host_storage_delete),
                "caller" => Function::new_typed_with_env(&mut store, &func_env, host::host_caller),
                "block_height" => Function::new_typed_with_env(&mut store, &func_env, host::host_block_height),
                "block_timestamp" => Function::new_typed_with_env(&mut store, &func_env, host::host_block_timestamp),
                "contract_address" => Function::new_typed_with_env(&mut store, &func_env, host::host_contract_address),
                "value_lo" => Function::new_typed_with_env(&mut store, &func_env, host::host_value_lo),
                "value_hi" => Function::new_typed_with_env(&mut store, &func_env, host::host_value_hi),
                "agent_get_score" => Function::new_typed_with_env(&mut store, &func_env, host::host_agent_get_score),
                "agent_is_registered" => Function::new_typed_with_env(&mut store, &func_env, host::host_agent_is_registered),
                "token_balance" => Function::new_typed_with_env(&mut store, &func_env, host::host_token_balance),
                "token_balance_hi" => Function::new_typed_with_env(&mut store, &func_env, host::host_token_balance_hi),
                "token_transfer" => Function::new_typed_with_env(&mut store, &func_env, host::host_token_transfer),
                "log_msg" => Function::new_typed_with_env(&mut store, &func_env, host::host_log),
                "return_data" => Function::new_typed_with_env(&mut store, &func_env, host::host_return_data),
                "abort" => Function::new_typed_with_env(&mut store, &func_env, host::host_abort),
                "emit_event" => Function::new_typed_with_env(&mut store, &func_env, host::host_emit_event),
                "call_contract" => Function::new_typed_with_env(&mut store, &func_env, host::host_call_contract),
                "cross_call_return_data" => Function::new_typed_with_env(&mut store, &func_env, host::host_cross_call_return_data),
            }
        };

        // 5-9: Same instantiation + call + result collection as execute()
        let instance = Instance::new(&mut store, &module, &import_object)
            .map_err(|e| VmError::InstantiationFailed(e.to_string()))?;

        let memory = instance
            .exports
            .get_memory("memory")
            .map_err(|e| VmError::MemoryError(e.to_string()))?;
        func_env.as_mut(&mut store).memory = Some(memory.clone());

        let args_ptr = if !args.is_empty() {
            if let Ok(alloc_fn) = instance.exports.get_function("alloc") {
                let result = alloc_fn
                    .call(&mut store, &[Value::I32(args.len() as i32)])
                    .map_err(|e| VmError::ExecutionFailed(format!("alloc failed: {e}")))?;
                let ptr = result[0].unwrap_i32() as u32;
                let mem_view = memory.view(&store);
                mem_view
                    .write(ptr as u64, args)
                    .map_err(|e| VmError::MemoryError(e.to_string()))?;
                Some(ptr)
            } else {
                None
            }
        } else {
            None
        };

        let func = instance
            .exports
            .get_function(method)
            .map_err(|_| VmError::MethodNotFound(method.to_string()))?;

        let func_type = func.ty(&store);
        let func_params = func_type.params();
        let call_args = if func_params.len() == 2 {
            match args_ptr {
                Some(ptr) => vec![Value::I32(ptr as i32), Value::I32(args.len() as i32)],
                None => vec![Value::I32(0), Value::I32(0)],
            }
        } else {
            vec![]
        };

        let _result = func.call(&mut store, &call_args).map_err(|e| {
            let msg = e.to_string();
            let fuel_consumed = func_env
                .as_ref(&store)
                .fuel_consumed
                .lock()
                .map(|f| *f)
                .unwrap_or(0);

            if msg.contains("out of fuel") || msg.contains("fuel exhausted") {
                VmError::OutOfFuel {
                    used: fuel_consumed,
                    limit: fuel_limit,
                }
            } else if msg.contains("contract abort:") {
                let reason = msg
                    .split("contract abort:")
                    .nth(1)
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|| msg.clone());
                VmError::ContractAbort {
                    reason,
                    fuel_consumed,
                }
            } else {
                VmError::ExecutionFailed(format!("{msg} (fuel consumed: {fuel_consumed})"))
            }
        })?;

        let env_ref = func_env.as_ref(&store);
        let return_data = env_ref.return_data.lock().unwrap().clone();
        let storage_changes = env_ref.storage_changes.lock().unwrap().clone();
        let logs = env_ref.logs.lock().unwrap().clone();
        let transfers = env_ref.transfers.lock().unwrap().clone();
        let events = env_ref.events.lock().unwrap().clone();
        let fuel_consumed = *env_ref.fuel_consumed.lock().unwrap();

        Ok(ExecutionResult {
            return_data,
            fuel_consumed,
            storage_changes,
            logs,
            transfers,
            events,
        })
    }
}
