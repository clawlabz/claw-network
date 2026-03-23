"""JSON-RPC 2.0 client for ClawNetwork node communication."""

from typing import Any

import httpx


class RpcError(Exception):
    """Raised when the RPC server returns a JSON-RPC error."""

    def __init__(self, code: int, message: str):
        self.code = code
        self.message = message
        super().__init__(f"RPC error {code}: {message}")


def rpc_call(endpoint: str, method: str, params: list | None = None) -> Any:
    """Make a JSON-RPC 2.0 call.

    Args:
        endpoint: RPC server URL.
        method: RPC method name.
        params: Optional list of parameters.

    Returns:
        The "result" field from the JSON-RPC response.

    Raises:
        RpcError: If the server returns a JSON-RPC error.
        ConnectionError: If the server is unreachable.
    """
    body = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params or [],
    }

    try:
        response = httpx.post(endpoint, json=body, timeout=30.0)
        response.raise_for_status()
    except httpx.ConnectError as exc:
        raise ConnectionError(f"Cannot connect to {endpoint}: {exc}") from exc
    except httpx.HTTPError as exc:
        raise ConnectionError(f"HTTP error from {endpoint}: {exc}") from exc

    data = response.json()

    if "error" in data:
        err = data["error"]
        raise RpcError(err.get("code", -1), err.get("message", "Unknown error"))

    return data.get("result")


# --- Convenience methods ---


def get_balance(endpoint: str, address: str) -> int:
    """Get CLAW balance for an address.

    Returns:
        Balance in base units (1 CLAW = 10^9 base units).
    """
    result = rpc_call(endpoint, "claw_getBalance", [address])
    if isinstance(result, str) and result.startswith("0x"):
        return int(result, 16)
    return int(result) if result is not None else 0


def get_nonce(endpoint: str, address: str) -> int:
    """Get current nonce for an address."""
    result = rpc_call(endpoint, "claw_getNonce", [address])
    return int(result) if result is not None else 0


def get_miner_info(endpoint: str, address: str) -> dict | None:
    """Get miner registration info. Returns None if not registered."""
    return rpc_call(endpoint, "claw_getMinerInfo", [address])


def send_transaction(endpoint: str, tx_hex: str) -> str:
    """Submit a signed transaction.

    Args:
        tx_hex: Hex-encoded borsh-serialized transaction.

    Returns:
        Transaction hash.
    """
    return rpc_call(endpoint, "claw_sendTransaction", [tx_hex])


def get_block_number(endpoint: str) -> int:
    """Get the current block height."""
    result = rpc_call(endpoint, "claw_blockNumber")
    return int(result) if result is not None else 0


def get_latest_block(endpoint: str) -> dict | None:
    """Get the latest block."""
    return rpc_call(endpoint, "claw_getBlock", ["latest"])


def faucet(endpoint: str, address: str) -> str:
    """Request testnet tokens from faucet."""
    return rpc_call(endpoint, "claw_faucet", [address])
