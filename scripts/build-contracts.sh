#!/usr/bin/env bash
# Build all contracts to Wasm
set -euo pipefail

CONTRACTS_DIR="$(cd "$(dirname "$0")/../contracts" && pwd)"

for contract in reward-vault arena-pool; do
  echo "Building $contract..."
  cd "$CONTRACTS_DIR/$contract"
  cargo build --target wasm32-unknown-unknown --release
  WASM_PATH="target/wasm32-unknown-unknown/release/${contract//-/_}.wasm"
  if [ -f "$WASM_PATH" ]; then
    SIZE=$(wc -c < "$WASM_PATH" | tr -d ' ')
    echo "  ✓ $contract → $WASM_PATH ($SIZE bytes)"
  else
    echo "  ✗ Build failed for $contract"
    exit 1
  fi
done

echo "All contracts built successfully."
