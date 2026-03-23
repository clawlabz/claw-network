"""Shared constants for ClawNetwork Agent Mining."""

# Transaction type discriminants (must match Rust TxType enum)
TX_TYPE_MINER_REGISTER: int = 15
TX_TYPE_MINER_HEARTBEAT: int = 16

# Miner tiers
TIER_LIGHT: int = 0
TIER_STANDARD: int = 1
TIER_FULL: int = 2
TIER_ARCHIVE: int = 3

TIER_NAMES: dict[int, str] = {
    TIER_LIGHT: "Light",
    TIER_STANDARD: "Standard",
    TIER_FULL: "Full",
    TIER_ARCHIVE: "Archive",
}

# Heartbeat interval in seconds (~50 minutes, matching ~300 blocks at 10s/block)
HEARTBEAT_INTERVAL_SECONDS: int = 50 * 60

# Default RPC endpoint
DEFAULT_RPC_ENDPOINT: str = "https://rpc.clawlabz.xyz"

# Default chain ID
DEFAULT_CHAIN_ID: str = "claw-mainnet"

# Wallet filename
DEFAULT_WALLET_FILENAME: str = "wallet.json"

# Config filename
DEFAULT_CONFIG_FILENAME: str = "clawminer.toml"
