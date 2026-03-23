"""Transaction construction and borsh encoding for ClawNetwork.

Borsh format must exactly match claw-node's Rust serialization:
- Transaction: tx_type(u8) || from([u8;32]) || nonce(u64 LE) || payload(Vec<u8>) || signature([u8;64])
- Vec<u8>: u32 LE length prefix + raw bytes
- String: u32 LE length prefix + UTF-8 bytes
- Signable bytes: tx_type(1) || from(32) || nonce(8 LE) || payload(raw, no length prefix)
"""

import struct

from nacl.signing import SigningKey


def build_miner_register_payload(tier: int, ip_addr: bytes, name: str) -> bytes:
    """Build borsh-encoded MinerRegisterPayload.

    Borsh layout: tier(u8) || ip_addr(u32 LE len + bytes) || name(u32 LE len + utf8)

    Args:
        tier: Miner tier (0-3).
        ip_addr: IP address bytes (4 for IPv4, 16 for IPv6).
        name: Human-readable miner name.

    Returns:
        Borsh-encoded payload bytes.
    """
    buf = bytearray()
    # tier: u8
    buf.append(tier & 0xFF)
    # ip_addr: Vec<u8>
    buf.extend(struct.pack("<I", len(ip_addr)))
    buf.extend(ip_addr)
    # name: String (borsh = u32 LE len + utf8 bytes)
    name_bytes = name.encode("utf-8")
    buf.extend(struct.pack("<I", len(name_bytes)))
    buf.extend(name_bytes)
    return bytes(buf)


def build_miner_heartbeat_payload(block_hash: bytes, height: int) -> bytes:
    """Build borsh-encoded MinerHeartbeatPayload.

    Borsh layout: latest_block_hash([u8;32]) || latest_height(u64 LE)

    Args:
        block_hash: 32-byte hash of the latest synced block.
        height: Height of the latest synced block.

    Returns:
        Borsh-encoded payload bytes.
    """
    if len(block_hash) != 32:
        raise ValueError(f"block_hash must be 32 bytes, got {len(block_hash)}")

    buf = bytearray()
    # latest_block_hash: [u8;32] (fixed, no length prefix)
    buf.extend(block_hash)
    # latest_height: u64 LE
    buf.extend(struct.pack("<Q", height))
    return bytes(buf)


def signable_bytes(tx_type: int, from_addr: bytes, nonce: int, payload: bytes) -> bytes:
    """Construct the message bytes that get signed.

    Must match Rust Transaction::signable_bytes():
        tx_type(1 byte) || from(32 bytes) || nonce(u64 LE, 8 bytes) || payload(raw bytes)

    Note: payload is included WITHOUT a length prefix in the signable message.

    Args:
        tx_type: Transaction type discriminant.
        from_addr: 32-byte sender public key.
        nonce: Transaction nonce.
        payload: Raw payload bytes (already borsh-encoded).

    Returns:
        Bytes to be signed.
    """
    buf = bytearray()
    buf.append(tx_type & 0xFF)
    buf.extend(from_addr)
    buf.extend(struct.pack("<Q", nonce))
    buf.extend(payload)
    return bytes(buf)


def build_transaction(
    tx_type: int,
    from_addr: bytes,
    nonce: int,
    payload: bytes,
    signing_key: SigningKey,
) -> bytes:
    """Build a complete borsh-encoded signed transaction.

    Borsh layout:
        tx_type(u8) || from([u8;32]) || nonce(u64 LE) || payload(Vec<u8>) || signature([u8;64])

    Where Vec<u8> = u32 LE length prefix + raw bytes.

    Args:
        tx_type: Transaction type discriminant.
        from_addr: 32-byte sender public key.
        nonce: Transaction nonce.
        payload: Borsh-encoded payload bytes.
        signing_key: PyNaCl SigningKey for Ed25519 signing.

    Returns:
        Complete borsh-encoded transaction bytes.
    """
    # Sign the message
    msg = signable_bytes(tx_type, from_addr, nonce, payload)
    signed = signing_key.sign(msg)
    signature = signed.signature  # 64 bytes

    # Build the full borsh-encoded transaction
    buf = bytearray()
    # tx_type: u8
    buf.append(tx_type & 0xFF)
    # from: [u8;32]
    buf.extend(from_addr)
    # nonce: u64 LE
    buf.extend(struct.pack("<Q", nonce))
    # payload: Vec<u8> (u32 LE length prefix + bytes)
    buf.extend(struct.pack("<I", len(payload)))
    buf.extend(payload)
    # signature: [u8;64] (fixed size, no length prefix)
    buf.extend(signature)

    return bytes(buf)
