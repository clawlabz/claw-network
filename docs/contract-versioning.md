# ClawNetwork Contract Versioning Guide

## Why Versioning Matters

ClawNetwork contracts are immutable once deployed — code cannot be changed after the Wasm binary is submitted to the chain. This differs from EVM-based chains where proxy patterns allow routing calls through an upgradeable delegate:

- No proxy pattern available (no cross-contract calls, no `delegatecall` equivalent)
- Contract address is derived from deployer address + nonce, so it changes on each deployment
- Any bug fix or feature addition requires deploying a brand-new contract at a new address
- All funds held by the old contract must be explicitly migrated to the new one

The versioning strategy documented here exists to manage that lifecycle safely.

---

## Built-in Version Support

Both current contracts store a `version` field that is set to `1` at initialization time. This field serves as a human-readable marker and as a guard against re-initialization.

**Reward Vault** (`contracts/reward-vault/src/types.rs`):

```rust
pub const KEY_VERSION: &[u8] = b"version";
pub const KEY_PAUSED:  &[u8] = b"paused";
pub const KEY_OWNER:   &[u8] = b"owner";
```

The `init` entry point writes `version = 1` to the `"version"` storage key and rejects any subsequent `init` call if that key already exists, preventing accidental re-initialization.

**Arena Pool** (`contracts/arena-pool/src/logic.rs`):

```rust
pub struct ContractState {
    pub version: u32,   // set to 1 in apply_init
    pub paused:  bool,
    // ...
}
```

The full contract state is serialized as a single Borsh blob under `"__state__"`. The `version` field travels with the state snapshot on every write.

Both contracts expose `pause()` and `unpause()` (owner-only). When paused:
- Reward Vault: `claim_reward` is blocked; `withdraw` (owner) and `fund` still work.
- Arena Pool: `deposit` and `lock_entries` are blocked; `withdraw` and emergency refund paths still work.

---

## Registry Pattern (Recommended)

Because the contract address changes on every deployment, any backend that hard-codes an address will break during an upgrade. The recommended approach is to treat the contract address as configuration that can be updated independently of the code.

```
┌──────────────────────────────────────────────────────┐
│                   Registry (config)                   │
│                                                        │
│  "reward_vault"  →  0xABCD...  (v1 active address)   │
│  "arena_pool"    →  0x1234...  (v1 active address)   │
└──────────────────────────────────────────────────────┘
         │                              │
         ▼                              ▼
┌─────────────────┐          ┌──────────────────┐
│  Arena Backend  │          │  Market Backend  │
│  reads address  │          │  reads address   │
│  at startup     │          │  at startup      │
└─────────────────┘          └──────────────────┘
```

In practice, the registry is currently implemented as environment variables:

```bash
ARENA_REWARD_VAULT_ADDRESS=<hex-encoded contract address>
ARENA_POOL_CONTRACT_ADDRESS=<hex-encoded contract address>
```

These are read at backend startup. Changing the active contract requires updating these values and redeploying (or triggering a rolling restart via a feature flag).

A future on-chain Registry contract could serve the same role by storing a `name → address` mapping on-chain and allowing the owner to atomically update it.

---

## Upgrade Workflow

### Step 1: Deploy New Contract Version

Build the new Wasm binary and deploy it as a fresh contract. The new contract receives a new address (deployer address + incremented nonce).

```bash
# Build
cargo build --release --target wasm32-unknown-unknown -p reward-vault

# Deploy (uses deployer account's next nonce internally)
claw-cli contract deploy \
  --wasm target/wasm32-unknown-unknown/release/reward_vault.wasm \
  --signer <owner-key>

# Note the new contract address printed in the output
NEW_CONTRACT=0x...
```

Initialize the new contract with the same parameters as v1, plus any new parameters introduced in v2:

```bash
claw-cli contract call \
  --contract $NEW_CONTRACT \
  --method init \
  --args '{"owner":"<owner-hex>","platforms":[...],"daily_cap":...,"min_games":...}' \
  --signer <owner-key>
```

### Step 2: Pause Old Contract

Call `pause()` on the old contract. Only the `owner` address can do this.

```bash
claw-cli contract call \
  --contract $OLD_CONTRACT \
  --method pause \
  --signer <owner-key>
```

While paused:
- **Reward Vault**: all `claim_reward` calls revert with `"paused"`. `withdraw` (owner treasury pull) and `fund` (replenish) still work.
- **Arena Pool**: `deposit` and `lock_entries` revert with `"contract is paused"`. `withdraw`, `refund_game`, and `refund_game_emergency` still work so users can always recover their funds.

### Step 3: Migrate Funds

Pull the treasury/fee balance from the old contract into the owner wallet, then fund the new contract.

**Reward Vault — withdraw and re-fund:**

```bash
# Pull all remaining CLAW from old vault to owner wallet
claw-cli contract call \
  --contract $OLD_CONTRACT \
  --method withdraw \
  --args '{"amount": <current-balance>}' \
  --signer <owner-key>

# Fund the new vault
claw-cli contract call \
  --contract $NEW_CONTRACT \
  --method fund \
  --args '{}' \
  --value <amount> \
  --signer <owner-key>
```

**Arena Pool — claim fees and wait for user withdrawals:**

```bash
# Claim accumulated platform fees
claw-cli contract call \
  --contract $OLD_CONTRACT \
  --method claim_fees \
  --signer <owner-key>
```

User balances cannot be force-transferred; users must withdraw themselves (see Step 5).

### Step 4: Update Backend Configuration

Update the environment variable(s) to point at the new contract address:

```bash
# In .env.local / Vercel environment variables
ARENA_REWARD_VAULT_ADDRESS=<new-contract-hex>
ARENA_POOL_CONTRACT_ADDRESS=<new-contract-hex>
```

Redeploy the backend. All new transactions will be routed to the new contract. The old contract remains on-chain in a paused state so any in-flight transactions can still settle.

For zero-downtime switches, use a feature flag approach:
1. Deploy with both old and new addresses in config.
2. Toggle the feature flag to switch traffic to the new contract.
3. Monitor for errors before removing the old address from config.

### Step 5: User Migration (Arena Pool only)

The Reward Vault holds platform-owned funds — only the owner needs to migrate (Step 3 covers this). The Arena Pool, however, holds individual user balances. Users whose balance is held in the old pool contract need to:

1. Withdraw from the old contract via the UI or CLI.
2. Deposit into the new contract.

The UI should detect this situation and display a migration banner:

```
Condition: user has balance > 0 on old pool contract address
           AND current active contract is the new address

Banner: "You have X CLAW in an older game wallet contract.
         [Withdraw from old contract] to move your funds."
```

The backend can detect this by querying both the old and new contract addresses during the balance check phase, comparing `state.available(user_address)` on each.

---

## Emergency Procedures

If a security vulnerability is discovered in a deployed contract:

1. **Pause immediately.** Call `pause()` on the affected contract as the first action. This halts new deposits and claims within seconds of the transaction landing on-chain.

2. **Do not panic about locked user funds.** Even while paused, `withdraw` (Arena Pool) and `refund_game_emergency` remain callable. All user funds are recoverable.

3. **Deploy the patched version.** Follow the full upgrade workflow above (Steps 1-4). Do not attempt to patch the existing contract — it is immutable.

4. **Communicate to users.** If user action is required for fund migration (Arena Pool case), post instructions in community channels with the old and new contract addresses.

5. **Monitor the old contract for 30 days.** Keep the old contract address known internally. Any user who did not see the migration notice can still withdraw up to 30 days after the pause.

---

## Version Compatibility Matrix

| Change Type | Example | Fund Migration Required? | Action |
|---|---|---|---|
| New method added | Add `get_version()` view | No | Deploy new, update env var, point backend |
| Storage layout changed | Add new field to `ContractState` | Yes | Full fund migration; old state is unreadable by new binary |
| Parameter change (fee %) | Change `fee_bps` default | No | Deploy new, init with updated params |
| Bug fix (logic error) | Fix overflow in `settle_game` | Yes | Pause old, deploy new, migrate |
| Bug fix (view only) | Fix wrong return value | No | Deploy new, point backend |
| Security vulnerability | Unauthorized withdrawal path | Yes | Emergency pause, deploy patched, migrate immediately |

**Key rule**: if the Borsh-serialized `ContractState` struct changes shape in any way (fields added, removed, reordered, or type-changed), the old state blob cannot be deserialized by the new binary. Treat any such change as requiring full fund migration.

---

## Best Practices

**Before deploying a new version:**
- Test the complete upgrade workflow on testnet first, including fund migration.
- Verify the new contract's `init` parameters match the intended production values.
- Confirm the new Wasm binary passes all unit and integration tests (`cargo test`).

**During migration:**
- Keep the old contract running in paused state for at least 30 days before considering it abandoned.
- Monitor `withdraw` events on the old contract address to track user migration progress.
- Do not remove the old contract address from internal records until all user balances are confirmed zero.

**After migration:**
- Update `ARENA_REWARD_VAULT_ADDRESS` / `ARENA_POOL_CONTRACT_ADDRESS` in all environments (staging, production).
- Document the version change in the changelog below.
- Alert any third-party integrators who may have the old address hard-coded.

---

## Contract Address Changelog

| Contract | Version | Address | Deployed | Notes |
|---|---|---|---|---|
| Reward Vault | v1 | TBD (testnet) | TBD | Initial deployment |
| Arena Pool | v1 | TBD (testnet) | TBD | Initial deployment |

Update this table each time a new version is deployed to any network. Include the exact contract address, deployment date, and a brief description of what changed from the previous version.
