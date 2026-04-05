# ClawNetwork Smart Contracts

## Available Contracts

- `reward-vault/` — CLAW reward distribution with daily caps and platform authorization
- `arena-pool/` — Game wallet with pre-deposit, lock/settle/refund, and emergency timeout

## Building

Each contract builds independently:

```bash
cd reward-vault && cargo build --target wasm32-unknown-unknown --release
cd arena-pool && cargo build --target wasm32-unknown-unknown --release
```

## Testing

```bash
cd reward-vault && cargo test
cd arena-pool && cargo test
```

Tests run natively (not inside the Wasm VM). Each contract ships with a mock host
environment in `tests/integration.rs` that stubs the SDK host functions so unit and
integration logic can be exercised without a running node.

## Deploying

Deployment scripts TBD. For now, deployment requires:

1. Build the Wasm bytecode:
   ```bash
   cd reward-vault && cargo build --target wasm32-unknown-unknown --release
   cd arena-pool && cargo build --target wasm32-unknown-unknown --release
   ```

2. Submit a `ContractDeploy` transaction (`TxType = 6`) with borsh-encoded `ContractDeployPayload`:

| Field         | Type       | Description                                      |
|---------------|------------|--------------------------------------------------|
| `code`        | `Vec<u8>`  | Wasm bytecode produced by the build step         |
| `init_method` | `String`   | Constructor entry-point name (empty = none)      |
| `init_args`   | `Vec<u8>`  | Borsh-encoded constructor arguments              |

3. Sign with an Ed25519 key that has sufficient CLAW balance to cover gas. Use the platform 
signer tool (`@claw/shared/clawchain/signer`) to build and sign the transaction before 
broadcasting via the `/tx/submit` RPC endpoint.

## Contract Addresses

| Contract     | Network | Status |
|--------------|---------|--------|
| reward-vault | mainnet | Not deployed |
| arena-pool   | mainnet | Not deployed |
