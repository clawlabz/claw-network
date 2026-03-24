# ClawNetwork Smart Contracts

## Available Contracts

- `reward-vault/` — CLAW reward distribution with daily caps and platform authorization
- `arena-pool/` — Game wallet with pre-deposit, lock/settle/refund, and emergency timeout

## Building

```bash
cargo build --target wasm32-unknown-unknown --release
```

Or use the provided script to build all contracts at once:

```bash
./scripts/build-contracts.sh
```

## Testing

```bash
cargo test
```

Tests run natively (not inside the Wasm VM). Each contract ships with a mock host
environment in `tests/integration.rs` that stubs the SDK host functions so unit and
integration logic can be exercised without a running node.

## Deploying

```bash
./scripts/deploy-contract.sh <contract-name> <rpc-url> <deployer-private-key>
```

Example:

```bash
./scripts/deploy-contract.sh reward-vault https://rpc.clawlabz.xyz <hex-private-key>
```

### Deployment internals

Deployment submits a `ContractDeploy` transaction (`TxType = 6`) whose payload is a
borsh-encoded `ContractDeployPayload`:

| Field         | Type       | Description                                      |
|---------------|------------|--------------------------------------------------|
| `code`        | `Vec<u8>`  | Wasm bytecode produced by the build step         |
| `init_method` | `String`   | Constructor entry-point name (empty = none)      |
| `init_args`   | `Vec<u8>`  | Borsh-encoded constructor arguments              |

The transaction must be signed with an Ed25519 key that has sufficient CLAW balance to
cover gas. Use the platform signer tool (`@claw/shared/clawchain/signer`) to build and
sign the transaction before broadcasting via the `/tx/submit` RPC endpoint.

## Contract addresses (mainnet)

| Contract     | Address |
|--------------|---------|
| reward-vault | TBD — deploy and record here |
| arena-pool   | TBD — deploy and record here |
