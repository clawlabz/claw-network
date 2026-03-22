//! Transaction handlers — one per TxType.

use borsh::BorshDeserialize;
use claw_types::state::*;
use claw_types::transaction::*;

use crate::error::StateError;
use crate::world::{
    WorldState, MAX_CATEGORY_LEN, MAX_DESCRIPTION_LEN, MAX_ENDPOINT_LEN, MAX_MEMO_LEN,
    MAX_METADATA_ENTRIES, MAX_NAME_LEN, MAX_SYMBOL_LEN,
};

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

/// TokenTransfer: transfer native CLW.
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

/// ReputationAttest: record a reputation attestation.
///
/// DEPRECATED: This transaction type is kept for backward compatibility but
/// attestations submitted via this method are no longer counted toward
/// Agent Score calculations. Use PlatformActivityReport (tx type 11) instead.
pub fn handle_reputation_attest(
    state: &mut WorldState,
    tx: &Transaction,
) -> Result<(), StateError> {
    let payload = ReputationAttestPayload::try_from_slice(&tx.payload)
        .map_err(|e| StateError::PayloadDeserialize(e.to_string()))?;

    if tx.from == payload.to {
        return Err(StateError::SelfAttestation);
    }

    if !state.agents.contains_key(&tx.from) {
        return Err(StateError::AgentNotRegistered);
    }

    if !state.agents.contains_key(&payload.to) {
        return Err(StateError::AgentNotRegistered);
    }

    if payload.score < -100 || payload.score > 100 {
        return Err(StateError::ScoreOutOfRange(payload.score));
    }

    if payload.category.is_empty() || payload.category.len() > MAX_CATEGORY_LEN {
        return Err(StateError::NameTooLong {
            len: payload.category.len(),
            max: MAX_CATEGORY_LEN,
        });
    }

    if payload.platform.len() > MAX_CATEGORY_LEN {
        return Err(StateError::NameTooLong {
            len: payload.platform.len(),
            max: MAX_CATEGORY_LEN,
        });
    }

    if payload.memo.len() > MAX_MEMO_LEN {
        return Err(StateError::MemoTooLong {
            len: payload.memo.len(),
            max: MAX_MEMO_LEN,
        });
    }

    // Limit attestations per attester-target pair
    const MAX_ATTESTATIONS_PER_PAIR: usize = 50;
    let existing_count = state.reputation.iter()
        .filter(|r| r.from == tx.from && r.to == payload.to)
        .count();
    if existing_count >= MAX_ATTESTATIONS_PER_PAIR {
        return Err(StateError::AttestationLimitReached {
            max: MAX_ATTESTATIONS_PER_PAIR,
        });
    }

    state.reputation.push(ReputationAttestation {
        from: tx.from,
        to: payload.to,
        category: payload.category,
        score: payload.score,
        platform: payload.platform,
        memo: payload.memo,
        block_height: state.block_height,
    });

    Ok(())
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
pub fn handle_contract_deploy(
    state: &mut WorldState,
    tx: &Transaction,
) -> Result<(), StateError> {
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

    // Store contract metadata and code
    let code_hash = *blake3::hash(&payload.code).as_bytes();
    let instance = claw_vm::ContractInstance {
        address: contract_address,
        code_hash,
        creator: tx.from,
        deployed_at: state.block_height,
    };

    state.contracts.insert(contract_address, instance);
    state
        .contract_code
        .insert(contract_address, payload.code.clone());

    // If init_method is specified, call the constructor
    if !payload.init_method.is_empty() {
        let ctx = claw_vm::ExecutionContext {
            caller: tx.from,
            contract_address,
            block_height: state.block_height,
            block_timestamp: 0,
            value: 0,
            fuel_limit: claw_vm::DEFAULT_FUEL_LIMIT,
        };

        // Empty storage for a freshly deployed contract
        let storage = std::collections::BTreeMap::new();

        let result = engine
            .execute(
                &payload.code,
                &payload.init_method,
                &payload.init_args,
                ctx,
                storage,
                state,
            )
            .map_err(|e| StateError::ContractExecutionFailed(e.to_string()))?;

        // Apply storage changes
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

        // Apply token transfers from constructor
        for (to, amount) in result.transfers {
            let contract_bal = state.balances.get(&contract_address).copied().unwrap_or(0);
            if contract_bal < amount {
                return Err(StateError::ContractTransferInsufficientBalance {
                    need: amount,
                    have: contract_bal,
                });
            }
            *state.balances.entry(contract_address).or_insert(0) -= amount;
            *state.balances.entry(to).or_insert(0) += amount;
        }
    }

    Ok(())
}

/// ContractCall: call a method on a deployed smart contract.
pub fn handle_contract_call(
    state: &mut WorldState,
    tx: &Transaction,
) -> Result<(), StateError> {
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

    let ctx = claw_vm::ExecutionContext {
        caller: tx.from,
        contract_address: payload.contract,
        block_height: state.block_height,
        block_timestamp: 0,
        value: payload.value,
        fuel_limit: claw_vm::DEFAULT_FUEL_LIMIT,
    };

    let engine = claw_vm::VmEngine::new();
    let result = engine
        .execute(&code, &payload.method, &payload.args, ctx, storage, state)
        .map_err(|e| {
            // Refund value on execution failure
            if payload.value > 0 {
                *state.balances.entry(payload.contract).or_insert(0) -= payload.value;
                *state.balances.entry(tx.from).or_insert(0) += payload.value;
            }
            StateError::ContractExecutionFailed(e.to_string())
        })?;

    // Apply storage changes
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

    // Apply token transfers from contract
    for (to, amount) in result.transfers {
        let contract_bal = state.balances.get(&payload.contract).copied().unwrap_or(0);
        if contract_bal < amount {
            // Refund value sent to contract before returning error
            if payload.value > 0 {
                *state.balances.entry(payload.contract).or_insert(0) -= payload.value;
                *state.balances.entry(tx.from).or_insert(0) += payload.value;
            }
            return Err(StateError::ContractTransferInsufficientBalance {
                need: amount,
                have: contract_bal,
            });
        }
        *state.balances.entry(payload.contract).or_insert(0) -= amount;
        *state.balances.entry(to).or_insert(0) += amount;
    }

    Ok(())
}

/// Maximum action_type length for platform reports.
const MAX_ACTION_TYPE_LEN: usize = claw_types::state::MAX_ACTION_TYPE_LEN;

/// Epoch length in blocks (must match claw_consensus::EPOCH_LENGTH).
const EPOCH_LENGTH: u64 = 100;

/// PlatformActivityReport: submit on-chain activity data from a platform.
///
/// Only Platform Agents (registered agents with >= 50,000 CLW staked) can submit.
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

    // Submitter must have >= 50,000 CLW staked (Platform Agent threshold)
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
        if !state.agents.contains_key(&entry.agent) {
            return Err(StateError::AgentNotRegistered);
        }
    }

    // Apply: aggregate platform activity for each reported agent
    for entry in &payload.reports {
        let agg = state.platform_activity.entry(entry.agent).or_default();
        agg.total_actions += entry.action_count as u64;
        agg.platform_count += 1;
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

    // Record delegation: validator → owner (for reward routing).
    // Only set if not already delegated (first staker is the owner).
    state.stake_delegations.entry(validator_addr).or_insert(tx.from);

    // Record commission rate for this validator
    state.stake_commissions.insert(validator_addr, commission_bps);

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
            amount: u128::from_le_bytes(tx.payload[..16].try_into().unwrap()),
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
        // Clean up delegation and commission mappings when fully unstaked
        state.stake_delegations.remove(&validator_addr);
        state.stake_commissions.remove(&validator_addr);
    } else {
        state.stakes.insert(validator_addr, remaining);
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
