"""Tests for config module — TOML configuration management."""

import pytest
from clawminer.config import default_config, save_config, load_config


def test_default_config():
    """Default config should have all required keys."""
    cfg = default_config()
    required_keys = ["rpc_endpoint", "chain_id", "miner_name", "tier", "wallet_path"]
    for key in required_keys:
        assert key in cfg, f"Default config missing required key: {key}"


def test_default_config_values():
    """Default config values should be sensible."""
    cfg = default_config()
    assert cfg["rpc_endpoint"].startswith("http"), "RPC endpoint must be a URL"
    assert isinstance(cfg["chain_id"], str)
    assert isinstance(cfg["tier"], int)
    assert 0 <= cfg["tier"] <= 3, "Tier must be 0-3"


def test_save_load_config(tmp_path):
    """Config TOML roundtrip should preserve values."""
    config_path = tmp_path / "config.toml"
    original = default_config()
    original["miner_name"] = "test-miner-42"
    original["tier"] = 2

    save_config(str(config_path), original)
    loaded = load_config(str(config_path))

    assert loaded == original, "Config must survive TOML roundtrip"


def test_load_config_missing_file():
    """Loading from a nonexistent path should raise FileNotFoundError."""
    with pytest.raises(FileNotFoundError):
        load_config("/nonexistent/config.toml")


def test_save_config_creates_file(tmp_path):
    """save_config should create the file if it doesn't exist."""
    config_path = tmp_path / "subdir" / "config.toml"
    cfg = default_config()
    save_config(str(config_path), cfg)
    assert config_path.exists()
