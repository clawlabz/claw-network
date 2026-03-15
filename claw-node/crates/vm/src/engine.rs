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
        let host_env = HostEnv {
            context,
            storage: Arc::new(Mutex::new(storage)),
            storage_changes: Arc::new(Mutex::new(Vec::new())),
            logs: Arc::new(Mutex::new(Vec::new())),
            transfers: Arc::new(Mutex::new(Vec::new())),
            return_data: Arc::new(Mutex::new(Vec::new())),
            balances: Arc::new(balances_map),
            agent_scores: Arc::new(scores_map),
            registered_agents: Arc::new(registered_set),
            memory: None,
            fuel_remaining: Arc::new(Mutex::new(fuel_limit)),
            fuel_consumed: Arc::new(Mutex::new(0)),
            fuel_limit,
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
                "token_transfer" => Function::new_typed_with_env(&mut store, &func_env, host::host_token_transfer),
                "log_msg" => Function::new_typed_with_env(&mut store, &func_env, host::host_log),
                "return_data" => Function::new_typed_with_env(&mut store, &func_env, host::host_return_data),
                "abort" => Function::new_typed_with_env(&mut store, &func_env, host::host_abort),
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

        let call_args = match args_ptr {
            Some(ptr) => vec![Value::I32(ptr as i32), Value::I32(args.len() as i32)],
            None => vec![],
        };

        let _result = func.call(&mut store, &call_args).map_err(|e| {
            let msg = e.to_string();
            if msg.contains("fuel") {
                VmError::OutOfFuel {
                    used: fuel_limit,
                    limit: fuel_limit,
                }
            } else {
                VmError::ExecutionFailed(msg)
            }
        })?;

        // 9. Collect results
        let env_ref = func_env.as_ref(&store);
        let return_data = env_ref.return_data.lock().unwrap().clone();
        let storage_changes = env_ref.storage_changes.lock().unwrap().clone();
        let logs = env_ref.logs.lock().unwrap().clone();
        let transfers = env_ref.transfers.lock().unwrap().clone();
        let fuel_consumed = *env_ref.fuel_consumed.lock().unwrap();

        Ok(ExecutionResult {
            return_data,
            fuel_consumed,
            storage_changes,
            logs,
            transfers,
        })
    }
}
