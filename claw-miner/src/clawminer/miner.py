"""Mining loop — registration, heartbeat, and graceful shutdown."""

import signal
import time
from typing import Any, Callable

from nacl.signing import SigningKey
from rich.console import Console

from clawminer.constants import (
    HEARTBEAT_INTERVAL_SECONDS,
    TX_TYPE_MINER_REGISTER,
    TX_TYPE_MINER_HEARTBEAT,
)
from clawminer.rpc import (
    get_miner_info,
    get_nonce,
    get_latest_block,
    send_transaction,
    RpcError,
)
from clawminer.tx import (
    build_miner_register_payload,
    build_miner_heartbeat_payload,
    build_transaction,
)
from clawminer.wallet import address_hex

console = Console()

# Sentinel for graceful shutdown
_shutdown_requested = False


def _handle_signal(signum: int, frame: Any) -> None:
    global _shutdown_requested
    _shutdown_requested = True
    console.print("\n[yellow]Shutdown requested, finishing current cycle...[/yellow]")


def check_registration(
    rpc_fn: Callable, endpoint: str, address: str
) -> bool:
    """Check if a miner is registered on-chain.

    Args:
        rpc_fn: Function to call for RPC (typically rpc.get_miner_info).
        endpoint: RPC endpoint URL.
        address: Hex-encoded miner address.

    Returns:
        True if registered, False otherwise.
    """
    result = rpc_fn(endpoint, address)
    return result is not None


def register_miner(
    endpoint: str,
    signing_key: SigningKey,
    tier: int,
    name: str,
    ip_addr: bytes,
) -> str:
    """Register as a miner on-chain.

    Args:
        endpoint: RPC endpoint URL.
        signing_key: Ed25519 signing key.
        tier: Miner tier (currently only tier 1 supported in Phase 1).
        name: Human-readable miner name.
        ip_addr: IP address bytes (4 for IPv4).

    Returns:
        Transaction hash.

    Raises:
        ValueError: If tier is not 1 (Phase 1 only supports Tier 1).
    """
    if tier != 1:
        raise ValueError("Phase 1 only supports Tier 1 (Online)")

    from_addr = bytes(signing_key.verify_key)
    address = address_hex(from_addr)
    nonce = get_nonce(endpoint, address) + 1

    payload = build_miner_register_payload(tier=tier, ip_addr=ip_addr, name=name)
    tx_bytes = build_transaction(
        tx_type=TX_TYPE_MINER_REGISTER,
        from_addr=from_addr,
        nonce=nonce,
        payload=payload,
        signing_key=signing_key,
    )

    return send_transaction(endpoint, tx_bytes.hex())


def send_heartbeat(
    endpoint: str,
    signing_key: SigningKey,
) -> str:
    """Send a miner heartbeat transaction.

    Args:
        endpoint: RPC endpoint URL.
        signing_key: Ed25519 signing key.

    Returns:
        Transaction hash.
    """
    from_addr = bytes(signing_key.verify_key)
    address = address_hex(from_addr)
    nonce = get_nonce(endpoint, address) + 1

    # Get latest block info for heartbeat
    block = get_latest_block(endpoint)
    if block is None:
        raise RuntimeError("Failed to get latest block")

    block_hash_hex = block.get("hash", "00" * 32)
    if block_hash_hex.startswith("0x"):
        block_hash_hex = block_hash_hex[2:]
    block_hash = bytes.fromhex(block_hash_hex)
    if len(block_hash) != 32:
        block_hash = block_hash.ljust(32, b"\x00")[:32]

    height = int(block.get("height", block.get("number", 0)))

    payload = build_miner_heartbeat_payload(block_hash=block_hash, height=height)
    tx_bytes = build_transaction(
        tx_type=TX_TYPE_MINER_HEARTBEAT,
        from_addr=from_addr,
        nonce=nonce,
        payload=payload,
        signing_key=signing_key,
    )

    return send_transaction(endpoint, tx_bytes.hex())


def start_mining(
    endpoint: str,
    signing_key: SigningKey,
    tier: int,
    name: str,
    ip_addr: bytes = b"\x00\x00\x00\x00",
) -> None:
    """Start the mining loop: register if needed, then heartbeat periodically.

    Args:
        endpoint: RPC endpoint URL.
        signing_key: Ed25519 signing key.
        tier: Miner tier (0-3).
        name: Human-readable miner name.
        ip_addr: IP address bytes.
    """
    global _shutdown_requested
    _shutdown_requested = False

    # Set up signal handlers for graceful shutdown
    signal.signal(signal.SIGINT, _handle_signal)
    signal.signal(signal.SIGTERM, _handle_signal)

    from_addr = bytes(signing_key.verify_key)
    address = address_hex(from_addr)

    console.print(f"[bold green]ClawMiner starting[/bold green]")
    console.print(f"  Address: {address}")
    console.print(f"  Tier:    {tier}")
    console.print(f"  Name:    {name}")
    console.print(f"  RPC:     {endpoint}")
    console.print()

    # Check registration
    if not check_registration(get_miner_info, endpoint, address):
        console.print("[yellow]Not registered. Registering...[/yellow]")
        try:
            tx_hash = register_miner(endpoint, signing_key, tier, name, ip_addr)
            console.print(f"[green]Registered! TX: {tx_hash}[/green]")
        except (RpcError, ConnectionError) as exc:
            console.print(f"[red]Registration failed: {exc}[/red]")
            return
    else:
        console.print("[green]Already registered.[/green]")

    # Heartbeat loop
    console.print(f"\n[bold]Starting heartbeat loop (interval: {HEARTBEAT_INTERVAL_SECONDS}s)[/bold]")

    while not _shutdown_requested:
        try:
            tx_hash = send_heartbeat(endpoint, signing_key)
            console.print(f"[green]Heartbeat sent: {tx_hash}[/green]")
        except (RpcError, ConnectionError, RuntimeError) as exc:
            console.print(f"[red]Heartbeat failed: {exc}[/red]")

        # Sleep in small increments to allow graceful shutdown
        for _ in range(HEARTBEAT_INTERVAL_SECONDS):
            if _shutdown_requested:
                break
            time.sleep(1)

    console.print("[bold yellow]Miner stopped.[/bold yellow]")
