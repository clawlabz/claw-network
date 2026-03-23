"""Tests for tx module — transaction construction and borsh encoding.

These tests verify that Python-constructed transactions match the exact
byte format expected by the Rust claw-node (borsh serialization).
"""

import struct
import pytest
from nacl.signing import SigningKey, VerifyKey

from clawminer.tx import (
    build_miner_register_payload,
    build_miner_heartbeat_payload,
    signable_bytes,
    build_transaction,
)
from clawminer.constants import TX_TYPE_MINER_REGISTER, TX_TYPE_MINER_HEARTBEAT


def test_miner_register_payload_format():
    """MinerRegister payload bytes should match Rust borsh format.

    Borsh layout: tier(u8) || ip_addr(u32 LE len + bytes) || name(u32 LE len + utf8)
    """
    payload = build_miner_register_payload(tier=1, ip_addr=b"\x7f\x00\x00\x01", name="my-miner")

    offset = 0
    # tier: u8
    assert payload[offset] == 1
    offset += 1

    # ip_addr: Vec<u8> = u32 LE length prefix + raw bytes
    ip_len = struct.unpack_from("<I", payload, offset)[0]
    offset += 4
    assert ip_len == 4
    assert payload[offset : offset + ip_len] == b"\x7f\x00\x00\x01"
    offset += ip_len

    # name: String (borsh) = u32 LE length prefix + utf8 bytes
    name_len = struct.unpack_from("<I", payload, offset)[0]
    offset += 4
    assert name_len == len("my-miner")
    assert payload[offset : offset + name_len] == b"my-miner"
    offset += name_len

    assert offset == len(payload), "No trailing bytes"


def test_miner_heartbeat_payload_format():
    """MinerHeartbeat payload bytes should match Rust borsh format.

    Borsh layout: latest_block_hash([u8;32]) || latest_height(u64 LE)
    """
    block_hash = bytes(range(32))
    payload = build_miner_heartbeat_payload(block_hash=block_hash, height=12345)

    assert len(payload) == 32 + 8
    assert payload[:32] == block_hash
    assert struct.unpack_from("<Q", payload, 32)[0] == 12345


def test_signable_bytes():
    """Signable bytes must match claw-node's format: tx_type(1) || from(32) || nonce(8 LE) || payload.

    Note: payload is included raw (no length prefix) in signable bytes,
    matching Rust Transaction::signable_bytes().
    """
    from_addr = bytes(range(32))
    payload = b"\xde\xad\xbe\xef"

    result = signable_bytes(TX_TYPE_MINER_REGISTER, from_addr, nonce=7, payload=payload)

    expected = (
        bytes([TX_TYPE_MINER_REGISTER])  # tx_type: 1 byte
        + from_addr  # from: 32 bytes
        + struct.pack("<Q", 7)  # nonce: u64 LE
        + payload  # payload: raw bytes (no length prefix)
    )
    assert result == expected


def test_sign_verify():
    """Ed25519 signature should be verifiable."""
    signing_key = SigningKey.generate()
    from_addr = bytes(signing_key.verify_key)
    payload = build_miner_register_payload(tier=0, ip_addr=b"\x0a\x00\x00\x01", name="test")

    tx_bytes = build_transaction(
        tx_type=TX_TYPE_MINER_REGISTER,
        from_addr=from_addr,
        nonce=1,
        payload=payload,
        signing_key=signing_key,
    )

    # Verify we can parse the borsh-encoded transaction and check signature
    offset = 0
    # tx_type: u8 (borsh enum discriminant)
    assert tx_bytes[offset] == TX_TYPE_MINER_REGISTER
    offset += 1

    # from: [u8;32]
    assert tx_bytes[offset : offset + 32] == from_addr
    offset += 32

    # nonce: u64 LE
    nonce_val = struct.unpack_from("<Q", tx_bytes, offset)[0]
    assert nonce_val == 1
    offset += 8

    # payload: Vec<u8> (u32 LE length + bytes)
    payload_len = struct.unpack_from("<I", tx_bytes, offset)[0]
    offset += 4
    tx_payload = tx_bytes[offset : offset + payload_len]
    assert tx_payload == payload
    offset += payload_len

    # signature: [u8;64]
    signature = tx_bytes[offset : offset + 64]
    assert len(signature) == 64
    offset += 64

    assert offset == len(tx_bytes), "No trailing bytes in transaction"

    # Verify the signature using PyNaCl
    msg = signable_bytes(TX_TYPE_MINER_REGISTER, from_addr, 1, payload)
    verify_key = VerifyKey(from_addr)
    verify_key.verify(msg, signature)  # Raises if invalid


def test_build_transaction_deterministic():
    """Same inputs should produce the same transaction bytes (except different signing keys)."""
    signing_key = SigningKey.generate()
    from_addr = bytes(signing_key.verify_key)
    payload = build_miner_heartbeat_payload(block_hash=b"\x00" * 32, height=100)

    tx1 = build_transaction(TX_TYPE_MINER_HEARTBEAT, from_addr, 5, payload, signing_key)
    tx2 = build_transaction(TX_TYPE_MINER_HEARTBEAT, from_addr, 5, payload, signing_key)

    # Everything except signature should be identical (Ed25519 signatures are deterministic
    # for the same key+message, so the full tx should be identical)
    assert tx1 == tx2
