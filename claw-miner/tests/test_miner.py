"""Tests for miner module — mining loop logic."""

import pytest
from unittest.mock import MagicMock, patch, call
from clawminer.miner import (
    check_registration,
    register_miner,
    send_heartbeat,
)


def test_check_registration_registered():
    """check_registration should return True when miner is registered."""
    mock_rpc = MagicMock()
    mock_rpc.return_value = {"tier": 1, "name": "test-miner", "active": True}

    result = check_registration(mock_rpc, "http://localhost:9070", "aa" * 32)
    assert result is True


def test_check_registration_not_registered():
    """check_registration should return False when miner is not found."""
    mock_rpc = MagicMock()
    mock_rpc.return_value = None

    result = check_registration(mock_rpc, "http://localhost:9070", "aa" * 32)
    assert result is False


def test_get_latest_block_calls_correct_rpc():
    """get_latest_block should call claw_getBlockByNumber, not claw_getBlock."""
    from clawminer.rpc import get_latest_block

    with patch("clawminer.rpc.rpc_call") as mock_rpc:
        # Mock responses
        mock_rpc.side_effect = [
            42,  # claw_blockNumber returns 42
            {"height": 42, "hash": "0x" + "aa" * 32},  # claw_getBlockByNumber returns block
        ]

        result = get_latest_block("http://localhost:9070")

        # Verify we called both methods in correct order
        assert mock_rpc.call_count == 2
        calls = mock_rpc.call_args_list
        assert calls[0] == call("http://localhost:9070", "claw_blockNumber")
        assert calls[1] == call("http://localhost:9070", "claw_getBlockByNumber", [42])

        # Verify we got the block back
        assert result["height"] == 42


def test_register_rejects_invalid_tier():
    """register_miner should raise ValueError for tier != 1."""
    from nacl.signing import SigningKey

    signing_key = SigningKey.generate()

    with pytest.raises(ValueError, match="Phase 1 only supports Tier 1"):
        register_miner(
            endpoint="http://localhost:9070",
            signing_key=signing_key,
            tier=0,  # Invalid tier
            name="test-miner",
            ip_addr=b"\x00\x00\x00\x00",
        )


def test_register_accepts_tier_one():
    """register_miner should accept tier 1 and proceed with registration."""
    from nacl.signing import SigningKey

    signing_key = SigningKey.generate()

    with patch("clawminer.miner.get_nonce") as mock_nonce, \
         patch("clawminer.miner.send_transaction") as mock_send, \
         patch("clawminer.miner.build_transaction") as mock_build:

        mock_nonce.return_value = 100
        mock_send.return_value = "0x" + "ff" * 32
        mock_build.return_value = b"signed_tx"

        result = register_miner(
            endpoint="http://localhost:9070",
            signing_key=signing_key,
            tier=1,  # Valid tier
            name="test-miner",
            ip_addr=b"\x00\x00\x00\x00",
        )

        # Should call get_nonce
        mock_nonce.assert_called_once()
        # Should send transaction
        mock_send.assert_called_once()
        # Should return tx hash
        assert result == "0x" + "ff" * 32


def test_status_reads_reputation_bps(tmp_path):
    """Miner info parsing should read 'reputation_bps' field."""
    from clawminer.cli import status
    from click.testing import CliRunner

    # Create a dummy config file so config_path.exists() passes
    config_file = tmp_path / "clawminer.toml"
    config_file.write_text("")

    runner = CliRunner()

    with patch("clawminer.cli.load_config") as mock_load_cfg, \
         patch("clawminer.cli.load_wallet") as mock_load_wallet, \
         patch("clawminer.cli.get_miner_info") as mock_get_info, \
         patch("clawminer.cli.get_block_number") as mock_block_height:

        # Mock config and wallet
        mock_load_cfg.return_value = {
            "rpc_endpoint": "http://localhost:9070",
            "wallet_path": "/tmp/wallet.json"
        }
        mock_load_wallet.return_value = (
            b"private_key",
            bytes.fromhex("aa" * 32),  # public key
        )

        # Mock miner info with reputation_bps field
        mock_get_info.return_value = {
            "tier": 1,
            "name": "test-miner",
            "active": True,
            "reputation_bps": 5000,  # Should read this field
        }
        mock_block_height.return_value = 100

        # Run status command with tmp_path so config file is found
        result = runner.invoke(status, ["--dir", str(tmp_path)])

        # Verify command executed successfully
        assert result.exit_code == 0, f"Exit code: {result.exit_code}, Output: {result.output}"

        # Verify reputation_bps was read and displayed in output
        mock_get_info.assert_called_once()
        assert "5000" in result.output, f"Expected '5000' in output but got: {result.output}"
