#!/usr/bin/env bash
# Deploy a contract to ClawNetwork
# Usage: ./scripts/deploy-contract.sh <contract-name> <rpc-url> <private-key-hex>
set -euo pipefail

CONTRACT_NAME="${1:?Usage: deploy-contract.sh <name> <rpc-url> <private-key>}"
RPC_URL="${2:?Missing RPC URL}"
PRIVATE_KEY="${3:?Missing deployer private key (hex)}"

WASM_PATH="contracts/$CONTRACT_NAME/target/wasm32-unknown-unknown/release/${CONTRACT_NAME//-/_}.wasm"

if [ ! -f "$WASM_PATH" ]; then
  echo "Wasm not found at $WASM_PATH. Run build-contracts.sh first."
  exit 1
fi

echo "Deploying $CONTRACT_NAME to $RPC_URL..."
echo "Wasm size: $(wc -c < "$WASM_PATH" | tr -d ' ') bytes"

# Read wasm binary and hex-encode
WASM_HEX=$(xxd -p "$WASM_PATH" | tr -d '\n')

# Submit deploy transaction via RPC
# This is a placeholder — actual deployment requires constructing and signing
# a ContractDeploy transaction with the platform signer tool
echo ""
echo "To deploy, use the platform signer tool:"
echo "  import { PlatformSigner } from '@claw/shared/clawchain/signer'"
echo "  const signer = new PlatformSigner({ privateKeyHex: '...', rpcClient })"
echo "  // Build ContractDeploy tx with wasm bytes"
echo ""
echo "Contract Wasm hex (first 64 chars): ${WASM_HEX:0:64}..."
echo "Full hex length: ${#WASM_HEX} chars (${#WASM_HEX}/2 bytes)"
