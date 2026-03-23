"""Ed25519 wallet — keypair generation, save/load, address encoding."""

import json
from pathlib import Path

from nacl.signing import SigningKey


def generate_keypair() -> tuple[bytes, bytes]:
    """Generate a new Ed25519 keypair.

    Returns:
        (private_key, public_key) — both 32 bytes.
        private_key is the seed; public_key is the verify key.
    """
    signing_key = SigningKey.generate()
    private_key = bytes(signing_key)  # 32-byte seed
    public_key = bytes(signing_key.verify_key)  # 32-byte public key
    return private_key, public_key


def save_wallet(path: str, private_key: bytes, password: str | None = None) -> None:
    """Save wallet to a JSON file.

    Args:
        path: File path to save to.
        private_key: 32-byte Ed25519 seed.
        password: Optional encryption password (reserved for future use).
    """
    signing_key = SigningKey(private_key)
    public_key = bytes(signing_key.verify_key)

    data = {
        "private_key": private_key.hex(),
        "public_key": public_key.hex(),
    }

    file_path = Path(path)
    file_path.parent.mkdir(parents=True, exist_ok=True)
    file_path.write_text(json.dumps(data, indent=2) + "\n")


def load_wallet(path: str, password: str | None = None) -> tuple[bytes, bytes]:
    """Load wallet from a JSON file.

    Args:
        path: File path to load from.
        password: Optional decryption password (reserved for future use).

    Returns:
        (private_key, public_key) — both 32 bytes.

    Raises:
        FileNotFoundError: If the wallet file doesn't exist.
    """
    file_path = Path(path)
    if not file_path.exists():
        raise FileNotFoundError(f"Wallet file not found: {path}")

    data = json.loads(file_path.read_text())
    private_key = bytes.fromhex(data["private_key"])

    # Derive public key from private key to ensure consistency
    signing_key = SigningKey(private_key)
    public_key = bytes(signing_key.verify_key)

    return private_key, public_key


def address_hex(public_key: bytes) -> str:
    """Convert a 32-byte public key to a 64-char lowercase hex address.

    Args:
        public_key: 32-byte Ed25519 public key.

    Returns:
        64-character lowercase hex string.
    """
    return public_key.hex()
