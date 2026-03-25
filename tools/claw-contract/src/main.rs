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
    Build {
        /// Run wasm-opt -Oz to minimize the output binary
        #[arg(long)]
        optimize: bool,
    },

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

    /// Verify a deployed contract by rebuilding from source and comparing hashes
    Verify {
        /// Contract address (64-char hex, with or without 0x prefix)
        contract: String,

        /// Path to the contract source directory (must contain Cargo.toml)
        #[arg(long)]
        source: PathBuf,

        /// JSON-RPC endpoint URL
        #[arg(long, default_value = "https://testnet-rpc.clawlabz.xyz")]
        rpc: String,
    },

    /// Disassemble Wasm bytecode to WAT text format
    #[command(alias = "wat")]
    Disassemble {
        /// Contract address (64-char hex) OR path to a local .wasm file
        target: String,

        /// JSON-RPC endpoint URL (used when target is a contract address)
        #[arg(long, default_value = "https://testnet-rpc.clawlabz.xyz")]
        rpc: String,

        /// Write output to a file instead of stdout
        #[arg(long, short)]
        output: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::New { name } => cmd_new(&name),
        Commands::Build { optimize } => cmd_build(optimize),
        Commands::Deploy { wasm_file, method, args, rpc, key } => {
            cmd_deploy(&wasm_file, &method, &args, &rpc, &key).await
        }
        Commands::Call { contract, method, args, value, rpc, key } => {
            cmd_call(&contract, &method, &args, value, &rpc, &key).await
        }
        Commands::Devnet => cmd_devnet(),
        Commands::Verify { contract, source, rpc } => {
            cmd_verify(&contract, &source, &rpc).await
        }
        Commands::Disassemble { target, rpc, output } => {
            cmd_disassemble(&target, &rpc, output.as_deref()).await
        }
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

fn cmd_build(optimize: bool) -> Result<()> {
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

    // Report build size
    let size_before = std::fs::metadata(&wasm_path)
        .map(|m| m.len())
        .unwrap_or(0);
    if size_before > 0 {
        println!("Built: {wasm_path} ({}KB)", size_before / 1024);
    } else {
        println!("Built: {wasm_path}");
    }

    // Wasm optimization pass (opt-in via --optimize)
    if optimize {
        run_wasm_opt(&wasm_path, size_before)?;
    }

    Ok(())
}

/// Run `wasm-opt -Oz` on the compiled wasm and report size savings.
fn run_wasm_opt(wasm_path: &str, size_before: u64) -> Result<()> {
    let optimized_path = format!("{wasm_path}.opt");

    let wasm_opt_result = std::process::Command::new("wasm-opt")
        .args(["-Oz", wasm_path, "-o", &optimized_path])
        .status();

    match wasm_opt_result {
        Ok(s) if s.success() => {
            // Replace original with optimized
            std::fs::rename(&optimized_path, wasm_path)
                .context("replacing wasm with optimized version")?;

            let size_after = std::fs::metadata(wasm_path)
                .map(|m| m.len())
                .unwrap_or(0);

            println!("Optimized: {wasm_path} ({}KB)", size_after / 1024);

            if size_before > 0 && size_after > 0 {
                let saved = size_before.saturating_sub(size_after);
                let pct = (saved as f64 / size_before as f64) * 100.0;
                println!(
                    "  before: {}KB, after: {}KB, saved: {}KB ({:.1}%)",
                    size_before / 1024,
                    size_after / 1024,
                    saved / 1024,
                    pct,
                );
            }
        }
        Ok(_) => {
            // Clean up partial output
            let _ = std::fs::remove_file(&optimized_path);
            eprintln!("Warning: wasm-opt exited with error, skipping optimization");
        }
        Err(_) => {
            eprintln!(
                "Warning: wasm-opt not found. Install it to enable optimization:\n  \
                 brew install binaryen\n  \
                 or: cargo install wasm-opt"
            );
        }
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

async fn cmd_verify(contract_hex: &str, source_dir: &Path, rpc_url: &str) -> Result<()> {
    let address = contract_hex
        .strip_prefix("0x")
        .unwrap_or(contract_hex);

    // 1. Validate source directory
    let cargo_toml = source_dir.join("Cargo.toml");
    if !cargo_toml.exists() {
        bail!(
            "no Cargo.toml found in source directory: {}",
            source_dir.display()
        );
    }

    let cargo_toml_str =
        std::fs::read_to_string(&cargo_toml).context("reading source Cargo.toml")?;
    let pkg_name = extract_package_name(&cargo_toml_str)
        .context("could not determine package name from source Cargo.toml")?;

    // 2. Build the contract from source
    println!("Building {pkg_name} from source...");
    let status = std::process::Command::new("cargo")
        .args(["build", "--target", "wasm32-unknown-unknown", "--release"])
        .current_dir(source_dir)
        .status()
        .context("running cargo build in source directory")?;

    if !status.success() {
        bail!("cargo build failed for source directory: {}", source_dir.display());
    }

    // 3. Locate and hash the compiled .wasm
    let wasm_name = pkg_name.replace('-', "_");
    let wasm_path = source_dir.join(format!(
        "target/wasm32-unknown-unknown/release/{wasm_name}.wasm"
    ));

    if !wasm_path.exists() {
        bail!("expected wasm output not found: {}", wasm_path.display());
    }

    let wasm_bytes =
        std::fs::read(&wasm_path).context("reading compiled wasm")?;
    let local_hash = blake3::hash(&wasm_bytes);
    let local_hash_hex = hex::encode(local_hash.as_bytes());

    println!(
        "Local build: {} ({} bytes)",
        wasm_path.display(),
        wasm_bytes.len()
    );
    println!("Local hash:  {local_hash_hex}");

    // 4. Fetch on-chain code hash
    println!("Fetching on-chain contract info for {address}...");
    let info = rpc::fetch_contract_info(rpc_url, address)
        .await
        .context("fetching contract info from chain")?;

    let onchain_hash = &info.code_hash;
    println!("On-chain hash: {onchain_hash}");

    // 5. Compare
    println!();
    if local_hash_hex == *onchain_hash {
        println!("Verified \u{2713}  Source matches on-chain bytecode.");
        println!("  Contract: 0x{address}");
        println!("  Creator:  0x{}", info.creator);
        println!("  Deployed at block: {}", info.deployed_at);
    } else {
        println!("MISMATCH \u{2717}  Source does NOT match on-chain bytecode.");
        println!("  Local:    {local_hash_hex}");
        println!("  On-chain: {onchain_hash}");
        println!();
        println!("Possible causes:");
        println!("  - Different Rust/toolchain version");
        println!("  - Different source code");
        println!("  - wasm-opt was applied to the deployed binary but not locally (or vice versa)");
        bail!("verification failed: hash mismatch");
    }

    Ok(())
}

async fn cmd_disassemble(target: &str, rpc_url: &str, output: Option<&Path>) -> Result<()> {
    // Determine if target is a file path or a contract address
    let wasm_bytes = if Path::new(target).exists() {
        // Local file
        println!("Reading Wasm from file: {target}");
        std::fs::read(target)
            .with_context(|| format!("reading wasm file: {target}"))?
    } else {
        // Treat as contract address — fetch bytecode via RPC
        let address = target.strip_prefix("0x").unwrap_or(target);
        println!("Fetching bytecode for contract {address} from {rpc_url}...");
        rpc::fetch_contract_code(rpc_url, address)
            .await
            .context("fetching contract bytecode")?
    };

    println!("Wasm size: {} bytes", wasm_bytes.len());
    println!("Disassembling to WAT...");

    // Convert Wasm to WAT
    let wat = wasmprinter::print_bytes(&wasm_bytes)
        .context("failed to disassemble Wasm to WAT")?;

    match output {
        Some(path) => {
            std::fs::write(path, &wat)
                .with_context(|| format!("writing WAT to {}", path.display()))?;
            println!("WAT written to {} ({} bytes)", path.display(), wat.len());
        }
        None => {
            println!("--- WAT output ---");
            println!("{wat}");
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
