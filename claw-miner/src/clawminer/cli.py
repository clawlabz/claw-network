"""CLI entry point for claw-miner — Click commands."""

import os
import socket
from pathlib import Path

import click
from nacl.signing import SigningKey
from rich.console import Console
from rich.table import Table

from clawminer.config import default_config, save_config, load_config
from clawminer.constants import (
    DEFAULT_CONFIG_FILENAME,
    DEFAULT_WALLET_FILENAME,
    TIER_NAMES,
)
from clawminer.rpc import get_balance, get_miner_info, get_block_number, RpcError
from clawminer.wallet import generate_keypair, save_wallet, load_wallet, address_hex

console = Console()


def _resolve_paths(config_dir: str | None = None) -> tuple[Path, Path, Path]:
    """Resolve config directory, config file, and wallet file paths."""
    base = Path(config_dir) if config_dir else Path.cwd()
    config_path = base / DEFAULT_CONFIG_FILENAME
    wallet_path = base / DEFAULT_WALLET_FILENAME
    return base, config_path, wallet_path


@click.group()
@click.version_option(package_name="clawminer")
def main():
    """ClawNetwork Agent Mining CLI — earn CLAW by contributing to the network."""
    pass


@main.command()
@click.option("--dir", "config_dir", default=None, help="Directory for config and wallet files.")
@click.option("--name", "miner_name", default=None, help="Miner display name.")
@click.option("--tier", type=int, default=None, help="Miner tier (0=Light, 1=Standard, 2=Full, 3=Archive).")
@click.option("--rpc", "rpc_endpoint", default=None, help="RPC endpoint URL.")
def init(config_dir, miner_name, tier, rpc_endpoint):
    """Initialize wallet and config files."""
    base, config_path, wallet_path = _resolve_paths(config_dir)
    base.mkdir(parents=True, exist_ok=True)

    # Generate wallet
    if wallet_path.exists():
        console.print(f"[yellow]Wallet already exists: {wallet_path}[/yellow]")
    else:
        private_key, public_key = generate_keypair()
        save_wallet(str(wallet_path), private_key)
        console.print(f"[green]Wallet created: {wallet_path}[/green]")
        console.print(f"  Address: {address_hex(public_key)}")

    # Generate config
    if config_path.exists():
        console.print(f"[yellow]Config already exists: {config_path}[/yellow]")
    else:
        cfg = default_config()
        cfg["wallet_path"] = str(wallet_path)
        if miner_name:
            cfg["miner_name"] = miner_name
        if tier is not None:
            cfg["tier"] = tier
        if rpc_endpoint:
            cfg["rpc_endpoint"] = rpc_endpoint
        save_config(str(config_path), cfg)
        console.print(f"[green]Config created: {config_path}[/green]")

    console.print("\n[bold]Ready! Run 'claw-miner start' to begin mining.[/bold]")


@main.command()
@click.option("--dir", "config_dir", default=None, help="Directory for config and wallet files.")
def start(config_dir):
    """Start mining — register if needed, then send periodic heartbeats."""
    from clawminer.miner import start_mining

    _, config_path, _ = _resolve_paths(config_dir)

    if not config_path.exists():
        console.print("[red]Config not found. Run 'claw-miner init' first.[/red]")
        raise SystemExit(1)

    cfg = load_config(str(config_path))
    wallet_path = cfg.get("wallet_path", DEFAULT_WALLET_FILENAME)

    if not Path(wallet_path).exists():
        console.print(f"[red]Wallet not found at {wallet_path}. Run 'claw-miner init' first.[/red]")
        raise SystemExit(1)

    private_key, _ = load_wallet(wallet_path)
    signing_key = SigningKey(private_key)

    # Detect IP address
    try:
        s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        s.connect(("8.8.8.8", 80))
        ip_str = s.getsockname()[0]
        s.close()
        ip_addr = socket.inet_aton(ip_str)
    except OSError:
        ip_addr = b"\x00\x00\x00\x00"

    start_mining(
        endpoint=cfg["rpc_endpoint"],
        signing_key=signing_key,
        tier=cfg.get("tier", 1),
        name=cfg.get("miner_name", "claw-miner"),
        ip_addr=ip_addr,
    )


@main.command()
def stop():
    """Stop mining (sends SIGTERM to running miner process)."""
    console.print("[yellow]To stop the miner, press Ctrl+C in the running terminal.[/yellow]")
    console.print("Or use: [bold]kill -TERM <pid>[/bold]")


@main.command()
@click.option("--dir", "config_dir", default=None, help="Directory for config and wallet files.")
def status(config_dir):
    """Show miner registration status and info."""
    _, config_path, _ = _resolve_paths(config_dir)

    if not config_path.exists():
        console.print("[red]Config not found. Run 'claw-miner init' first.[/red]")
        raise SystemExit(1)

    cfg = load_config(str(config_path))
    wallet_path = cfg.get("wallet_path", DEFAULT_WALLET_FILENAME)

    _, public_key = load_wallet(wallet_path)
    address = address_hex(public_key)
    endpoint = cfg["rpc_endpoint"]

    table = Table(title="Miner Status")
    table.add_column("Field", style="cyan")
    table.add_column("Value", style="white")

    table.add_row("Address", address)
    table.add_row("RPC", endpoint)

    try:
        info = get_miner_info(endpoint, address)
        if info:
            table.add_row("Registered", "[green]Yes[/green]")
            table.add_row("Tier", TIER_NAMES.get(info.get("tier", -1), "Unknown"))
            table.add_row("Name", str(info.get("name", "")))
            table.add_row("Active", str(info.get("active", False)))
            table.add_row("Reputation", str(info.get("reputation_score", 0)))
        else:
            table.add_row("Registered", "[red]No[/red]")
    except (RpcError, ConnectionError) as exc:
        table.add_row("Status", f"[red]Error: {exc}[/red]")

    try:
        height = get_block_number(endpoint)
        table.add_row("Block Height", str(height))
    except (RpcError, ConnectionError):
        pass

    console.print(table)


@main.command()
@click.option("--dir", "config_dir", default=None, help="Directory for config and wallet files.")
def balance(config_dir):
    """Show CLAW balance."""
    _, config_path, _ = _resolve_paths(config_dir)

    if not config_path.exists():
        console.print("[red]Config not found. Run 'claw-miner init' first.[/red]")
        raise SystemExit(1)

    cfg = load_config(str(config_path))
    wallet_path = cfg.get("wallet_path", DEFAULT_WALLET_FILENAME)

    _, public_key = load_wallet(wallet_path)
    address = address_hex(public_key)
    endpoint = cfg["rpc_endpoint"]

    try:
        bal = get_balance(endpoint, address)
        claw_amount = bal / 1_000_000_000  # 9 decimals
        console.print(f"Address: {address}")
        console.print(f"Balance: [bold green]{claw_amount:.9f} CLAW[/bold green] ({bal} base units)")
    except (RpcError, ConnectionError) as exc:
        console.print(f"[red]Failed to get balance: {exc}[/red]")
