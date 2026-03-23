"""Tests for wallet module — Ed25519 keypair generation, save/load, address encoding."""

import json
import pytest
from clawminer.wallet import generate_keypair, save_wallet, load_wallet, address_hex


def test_generate_keypair():
    """Generated keypair should be 32 bytes each."""
    private_key, public_key = generate_keypair()
    assert len(private_key) == 32, "Private key must be 32 bytes (Ed25519 seed)"
    assert len(public_key) == 32, "Public key must be 32 bytes"


def test_generate_keypair_unique():
    """Two generated keypairs should not be identical."""
    kp1 = generate_keypair()
    kp2 = generate_keypair()
    assert kp1[0] != kp2[0], "Private keys must differ"
    assert kp1[1] != kp2[1], "Public keys must differ"


def test_save_load_wallet(tmp_path):
    """Wallet save/load roundtrip should preserve keys."""
    wallet_path = tmp_path / "wallet.json"
    private_key, public_key = generate_keypair()

    save_wallet(str(wallet_path), private_key)
    loaded_priv, loaded_pub = load_wallet(str(wallet_path))

    assert loaded_priv == private_key, "Private key must survive roundtrip"
    assert loaded_pub == public_key, "Public key must survive roundtrip"


def test_save_wallet_file_format(tmp_path):
    """Saved wallet should be valid JSON with hex-encoded keys."""
    wallet_path = tmp_path / "wallet.json"
    private_key, public_key = generate_keypair()

    save_wallet(str(wallet_path), private_key)

    data = json.loads(wallet_path.read_text())
    assert "private_key" in data
    assert "public_key" in data
    assert len(data["private_key"]) == 64, "Hex-encoded 32 bytes = 64 chars"
    assert len(data["public_key"]) == 64


def test_load_wallet_missing_file():
    """Loading from a nonexistent path should raise FileNotFoundError."""
    with pytest.raises(FileNotFoundError):
        load_wallet("/nonexistent/wallet.json")


def test_address_hex():
    """Address should be 64-char lowercase hex string of public key."""
    _, public_key = generate_keypair()
    addr = address_hex(public_key)
    assert len(addr) == 64
    assert addr == public_key.hex()
    assert addr == addr.lower(), "Address must be lowercase hex"


def test_address_hex_deterministic():
    """Same public key should always produce the same address."""
    _, public_key = generate_keypair()
    assert address_hex(public_key) == address_hex(public_key)
