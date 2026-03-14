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
