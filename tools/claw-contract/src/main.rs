mod args;
mod key;
mod new;
mod rpc;
mod tx;

#[cfg(test)]
mod tests;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "claw-contract",
    about = "Developer CLI for ClawNetwork smart contracts",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new contract project from template
    New {
        /// Contract project name
        name: String,
    },

    /// Build the contract Wasm (run inside a contract project)
    Build,

    /// Deploy a compiled Wasm contract to the chain
    Deploy {
        /// Path to the compiled .wasm file
        wasm_file: PathBuf,

        /// Constructor method name (empty string = no constructor)
        #[arg(long, default_value = "")]
        method: String,

        /// Constructor arguments (hex encoded, e.g. 0x or 0x0102ab)
        #[arg(long, default_value = "0x")]
        args: String,

        /// JSON-RPC endpoint URL
        #[arg(long, default_value = "https://testnet-rpc.clawlabz.xyz")]
        rpc: String,

        /// Signing key (64-char hex or path to a file containing the hex)
        #[arg(long)]
        key: String,
    },

    /// Call a method on a deployed contract
    Call {
        /// Contract address (64-char hex)
        contract: String,

        /// Method name to invoke
        method: String,

        /// Method arguments (hex encoded)
        #[arg(long, default_value = "0x")]
        args: String,

        /// Native CLAW value to send (in base units, 9 decimals)
        #[arg(long, default_value = "0")]
        value: u128,

        /// JSON-RPC endpoint URL
        #[arg(long, default_value = "https://testnet-rpc.clawlabz.xyz")]
        rpc: String,

        /// Signing key (64-char hex or path to a file containing the hex)
        #[arg(long)]
        key: String,
    },

    /// Start a single-node local devnet for development
    Devnet,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::New { name } => cmd_new(&name),
        Commands::Build => cmd_build(),
        Commands::Deploy { wasm_file, method, args, rpc, key } => {
            cmd_deploy(&wasm_file, &method, &args, &rpc, &key).await
        }
        Commands::Call { contract, method, args, value, rpc, key } => {
            cmd_call(&contract, &method, &args, value, &rpc, &key).await
        }
        Commands::Devnet => cmd_devnet(),
    }
}

// ---- Subcommand implementations ----

fn cmd_new(name: &str) -> Result<()> {
    let cwd = std::env::current_dir().context("getting current directory")?;
    new::create_contract_project(&cwd, name)?;
    println!("Created contract project: {name}/");
    println!();
    println!("Next steps:");
    println!("  cd {name}");
    println!("  cargo build --target wasm32-unknown-unknown --release");
    Ok(())
}

fn cmd_build() -> Result<()> {
    // Determine project name from Cargo.toml in current directory
    let cargo_toml_path = Path::new("Cargo.toml");
    if !cargo_toml_path.exists() {
        bail!("no Cargo.toml found in current directory — run this inside a contract project");
    }

    let cargo_toml_str =
        std::fs::read_to_string(cargo_toml_path).context("reading Cargo.toml")?;
    let pkg_name = extract_package_name(&cargo_toml_str)
        .context("could not determine package name from Cargo.toml")?;

    println!("Building {pkg_name} for wasm32-unknown-unknown...");

    // Run cargo build
    let status = std::process::Command::new("cargo")
        .args(["build", "--target", "wasm32-unknown-unknown", "--release"])
        .status()
        .context("running cargo build")?;

    if !status.success() {
        bail!("cargo build failed");
    }

    // Determine wasm file path
    let wasm_name = pkg_name.replace('-', "_");
    let wasm_path = format!(
        "target/wasm32-unknown-unknown/release/{wasm_name}.wasm"
    );

    // Report size
    if let Ok(meta) = std::fs::metadata(&wasm_path) {
        let kb = meta.len() / 1024;
        println!("Built: {wasm_path} ({kb}KB)");
    } else {
        println!("Built: {wasm_path}");
    }

    // Try wasm-opt (optional)
    let wasm_opt = std::process::Command::new("wasm-opt")
        .args(["-Oz", "--output", &wasm_path, &wasm_path])
        .status();

    match wasm_opt {
        Ok(s) if s.success() => {
            if let Ok(meta) = std::fs::metadata(&wasm_path) {
                let kb = meta.len() / 1024;
                println!("Optimized with wasm-opt: {wasm_path} ({kb}KB)");
            }
        }
        Ok(_) => eprintln!("Warning: wasm-opt failed, skipping optimization"),
        Err(_) => eprintln!("Warning: wasm-opt not found, skipping optimization"),
    }

    Ok(())
}

async fn cmd_deploy(
    wasm_file: &Path,
    method: &str,
    args_hex: &str,
    rpc_url: &str,
    key_input: &str,
) -> Result<()> {
    // 1. Load Wasm
    let code = std::fs::read(wasm_file)
        .with_context(|| format!("reading wasm file: {}", wasm_file.display()))?;
    println!(
        "Loaded {} ({} bytes)",
        wasm_file.display(),
        code.len()
    );

    // 2. Parse args
    let init_args = args::parse_hex_or_empty(args_hex)
        .context("parsing --args")?;

    // 3. Load signing key
    let signing_key = key::load_signing_key(key_input)
        .context("loading signing key")?;

    // 4. Fetch current nonce
    let pubkey = ed25519_dalek::VerifyingKey::from(&signing_key);
    let from_hex = hex::encode(pubkey.to_bytes());
    let nonce = rpc::fetch_nonce(rpc_url, &from_hex).await
        .context("fetching nonce")?;
    let deploy_nonce = nonce + 1;

    // 5. Encode payload
    let payload = tx::encode_contract_deploy_payload(&code, method, &init_args);

    // 6. Build & sign transaction
    let signed = tx::build_and_sign_transaction(
        tx::TxType::ContractDeploy,
        &signing_key,
        deploy_nonce,
        payload,
    );

    // 7. Submit
    println!("Submitting ContractDeploy transaction (nonce {deploy_nonce})...");
    let tx_hash = rpc::submit_transaction(rpc_url, &signed).await
        .context("submitting transaction")?;
    println!("Transaction submitted: {tx_hash}");

    // 8. Poll for confirmation
    println!("Waiting for confirmation...");
    rpc::poll_confirmation(rpc_url, &tx_hash, 30).await
        .context("waiting for confirmation")?;

    // 9. Compute and display contract address
    let contract_addr = tx::derive_contract_address(&signed.from, deploy_nonce);
    println!("Contract deployed successfully!");
    println!("Contract address: 0x{}", hex::encode(contract_addr));

    Ok(())
}

async fn cmd_call(
    contract_hex: &str,
    method: &str,
    args_hex: &str,
    value: u128,
    rpc_url: &str,
    key_input: &str,
) -> Result<()> {
    // 1. Parse contract address
    let contract_bytes = hex::decode(contract_hex.strip_prefix("0x").unwrap_or(contract_hex))
        .context("parsing contract address")?;
    if contract_bytes.len() != 32 {
        bail!("contract address must be 32 bytes (64 hex chars)");
    }
    let contract: [u8; 32] = contract_bytes.try_into().unwrap();

    // 2. Parse args
    let call_args = args::parse_hex_or_empty(args_hex)
        .context("parsing --args")?;

    // 3. Load signing key
    let signing_key = key::load_signing_key(key_input)
        .context("loading signing key")?;

    // 4. Fetch current nonce
    let pubkey = ed25519_dalek::VerifyingKey::from(&signing_key);
    let from_hex = hex::encode(pubkey.to_bytes());
    let nonce = rpc::fetch_nonce(rpc_url, &from_hex).await
        .context("fetching nonce")?;
    let call_nonce = nonce + 1;

    // 5. Encode payload
    let payload = tx::encode_contract_call_payload(&contract, method, &call_args, value);

    // 6. Build & sign
    let signed = tx::build_and_sign_transaction(
        tx::TxType::ContractCall,
        &signing_key,
        call_nonce,
        payload,
    );

    // 7. Submit
    println!("Calling {method} on 0x{} (nonce {call_nonce})...", hex::encode(contract));
    let tx_hash = rpc::submit_transaction(rpc_url, &signed).await
        .context("submitting transaction")?;
    println!("Transaction submitted: {tx_hash}");

    // 8. Poll for confirmation
    println!("Waiting for confirmation...");
    rpc::poll_confirmation(rpc_url, &tx_hash, 30).await
        .context("waiting for confirmation")?;
    println!("Call confirmed.");

    Ok(())
}

fn cmd_devnet() -> Result<()> {
    // Try to locate claw-node binary
    let binary = find_claw_node_binary();

    match binary {
        Some(bin) => {
            println!("Starting devnet with {}", bin.display());
            println!("Devnet running at http://localhost:9730");
            println!("Press Ctrl+C to stop.");

            let status = std::process::Command::new(&bin)
                .args(["--network", "devnet", "--single"])
                .status()
                .with_context(|| format!("running {}", bin.display()))?;

            if !status.success() {
                bail!("claw-node exited with non-zero status");
            }
        }
        None => {
            bail!(
                "claw-node binary not found. Build it first:\n\
                 cd claw-node && cargo build --release\n\
                 Or add it to your PATH."
            );
        }
    }

    Ok(())
}

// ---- Helpers ----

fn extract_package_name(cargo_toml: &str) -> Option<String> {
    for line in cargo_toml.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("name") {
            let rest = rest.trim();
            if let Some(rest) = rest.strip_prefix('=') {
                let name = rest.trim().trim_matches('"').to_string();
                if !name.is_empty() {
                    return Some(name);
                }
            }
        }
    }
    None
}

fn find_claw_node_binary() -> Option<PathBuf> {
    // Look in common locations relative to cwd
    let candidates = [
        PathBuf::from("../../claw-node/target/release/claw-node"),
        PathBuf::from("claw-node/target/release/claw-node"),
        PathBuf::from("claw-node"),
    ];
    for path in &candidates {
        if path.exists() {
            return Some(path.clone());
        }
    }
    // Check PATH
    which_claw_node()
}

fn which_claw_node() -> Option<PathBuf> {
    std::env::var("PATH").ok().and_then(|path_env| {
        for dir in path_env.split(':') {
            let candidate = Path::new(dir).join("claw-node");
            if candidate.exists() {
                return Some(candidate);
            }
        }
        None
    })
}
