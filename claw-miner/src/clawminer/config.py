"""TOML configuration management for claw-miner."""

from pathlib import Path

import tomli
import tomli_w

from clawminer.constants import (
    DEFAULT_RPC_ENDPOINT,
    DEFAULT_CHAIN_ID,
    DEFAULT_WALLET_FILENAME,
    TIER_STANDARD,
)


def default_config() -> dict:
    """Return the default miner configuration.

    Returns:
        Dict with all required configuration keys.
    """
    return {
        "rpc_endpoint": DEFAULT_RPC_ENDPOINT,
        "chain_id": DEFAULT_CHAIN_ID,
        "miner_name": "claw-miner",
        "tier": TIER_STANDARD,
        "wallet_path": DEFAULT_WALLET_FILENAME,
    }


def save_config(path: str, config: dict) -> None:
    """Save configuration to a TOML file.

    Args:
        path: File path to save to.
        config: Configuration dictionary.
    """
    file_path = Path(path)
    file_path.parent.mkdir(parents=True, exist_ok=True)
    file_path.write_bytes(tomli_w.dumps(config).encode())


def load_config(path: str) -> dict:
    """Load configuration from a TOML file.

    Args:
        path: File path to load from.

    Returns:
        Configuration dictionary.

    Raises:
        FileNotFoundError: If the config file doesn't exist.
    """
    file_path = Path(path)
    if not file_path.exists():
        raise FileNotFoundError(f"Config file not found: {path}")

    with open(file_path, "rb") as f:
        return tomli.load(f)
