"""Tests for miner module — mining loop logic."""

import pytest
from unittest.mock import MagicMock, patch
from clawminer.miner import check_registration


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
