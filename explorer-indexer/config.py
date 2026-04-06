"""Configuration — all values from environment variables."""

import os

RPC_URL = os.environ.get("RPC_URL", "http://127.0.0.1:9710")
NETWORK = os.environ.get("NETWORK", "mainnet")

DB_HOST = os.environ.get("DB_HOST", "127.0.0.1")
DB_PORT = int(os.environ.get("DB_PORT", "5432"))
DB_NAME = os.environ.get("DB_NAME", "litebase_claw")
DB_USER = os.environ.get("DB_USER", "claw")
DB_PASS = os.environ.get("DB_PASS", "")

# Indexer tuning
BATCH_SIZE = int(os.environ.get("BATCH_SIZE", "50"))
POLL_INTERVAL = int(os.environ.get("POLL_INTERVAL", "3"))
ERROR_RETRY_INTERVAL = int(os.environ.get("ERROR_RETRY_INTERVAL", "10"))
