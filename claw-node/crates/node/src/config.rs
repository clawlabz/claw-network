//! Node configuration and key management.
//!
//! Supports loading from `config.toml` in the data directory.
//! CLI arguments take precedence over config file values.
//!
//! Private key encryption: PBKDF2-HMAC-SHA256 + AES-256-GCM.
//! Set `CLAW_KEY_PASSWORD` env var to enable encryption.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{bail, Context, Result};
use claw_crypto::keys::generate_keypair;
use pbkdf2::pbkdf2_hmac;
use rand::Rng;
use serde::Deserialize;
use sha2::Sha256;
use std::fs;
use std::path::Path;

/// PBKDF2 iteration count.
const PBKDF2_ITERATIONS: u32 = 100_000;

/// Environment variable for key encryption password.
const PASSWORD_ENV: &str = "CLAW_KEY_PASSWORD";

/// Runtime node config loaded from key.json.
pub struct NodeConfig {
    pub address: [u8; 32],
    pub signing_key_bytes: [u8; 32],
}

/// Persistent config stored in `config.toml`.
#[derive(Debug, Deserialize, Default)]
pub struct FileConfig {
    #[serde(default)]
    pub node: NodeSection,
    #[serde(default)]
    pub network: NetworkSection,
    #[serde(default)]
    pub log: LogSection,
}

#[derive(Debug, Deserialize, Default)]
pub struct NodeSection {
    /// Data directory override (reserved for future use, ignored when loading)
    #[allow(dead_code)]
    pub data_dir: Option<String>,
    /// Network preset: "devnet", "testnet", or "mainnet"
    pub network: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct NetworkSection {
    /// RPC listen port
    pub rpc_port: Option<u16>,
    /// P2P listen port
    pub p2p_port: Option<u16>,
    /// Additional bootstrap peers (multiaddr format)
    #[serde(default)]
    pub bootstrap: Vec<String>,
    /// Force single-node mode
    pub single: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
pub struct LogSection {
    /// Log format: "text" or "json"
    pub format: Option<String>,
    /// Log filter directive (e.g. "claw=debug,info")
    pub filter: Option<String>,
}

/// Load config.toml from data directory, returning defaults if file doesn't exist.
pub fn load_file_config(data_dir: &Path) -> Result<FileConfig> {
    let config_path = data_dir.join("config.toml");
    if !config_path.exists() {
        return Ok(FileConfig::default());
    }

    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("read {}", config_path.display()))?;
    let cfg: FileConfig = toml::from_str(&content)
        .with_context(|| format!("parse {}", config_path.display()))?;
    Ok(cfg)
}

/// Write a default config.toml to the data directory (only if it doesn't exist).
pub fn write_default_config(data_dir: &Path, chain_id: &str) -> Result<()> {
    let config_path = data_dir.join("config.toml");
    if config_path.exists() {
        return Ok(());
    }

    let network_name = match chain_id {
        "claw-mainnet-1" => "mainnet",
        "claw-testnet-1" => "testnet",
        _ => "devnet",
    };

    let content = format!(
        r#"# ClawNetwork node configuration
# CLI arguments override these values.

[node]
network = "{network_name}"

[network]
rpc_port = 9710
p2p_port = 9711
# bootstrap = ["/ip4/1.2.3.4/tcp/9711"]
# single = false

[log]
format = "text"
# filter = "claw=info"
"#
    );

    fs::write(&config_path, content).context("write config.toml")?;
    tracing::info!("Wrote default config to {}", config_path.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Key encryption / decryption
// ---------------------------------------------------------------------------

/// Derive a 32-byte AES key from a password and salt using PBKDF2-HMAC-SHA256.
fn derive_aes_key(password: &str, salt: &[u8; 32]) -> [u8; 32] {
    let mut key = [0u8; 32];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, PBKDF2_ITERATIONS, &mut key);
    key
}

/// Encrypt a 32-byte secret key with AES-256-GCM.
///
/// Returns `(ciphertext_with_tag, salt, nonce)`.
fn encrypt_key(
    secret_key: &[u8; 32],
    password: &str,
) -> Result<(Vec<u8>, [u8; 32], [u8; 12])> {
    let mut rng = rand::thread_rng();

    let mut salt = [0u8; 32];
    rng.fill(&mut salt);

    let mut nonce_bytes = [0u8; 12];
    rng.fill(&mut nonce_bytes);

    let aes_key = derive_aes_key(password, &salt);
    let cipher = Aes256Gcm::new_from_slice(&aes_key)
        .map_err(|e| anyhow::anyhow!("create AES cipher: {e}"))?;

    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, secret_key.as_ref())
        .map_err(|e| anyhow::anyhow!("AES-GCM encrypt: {e}"))?;

    Ok((ciphertext, salt, nonce_bytes))
}

/// Decrypt a secret key encrypted with AES-256-GCM.
fn decrypt_key(
    ciphertext: &[u8],
    password: &str,
    salt: &[u8; 32],
    nonce: &[u8; 12],
) -> Result<[u8; 32]> {
    let aes_key = derive_aes_key(password, salt);
    let cipher = Aes256Gcm::new_from_slice(&aes_key)
        .map_err(|e| anyhow::anyhow!("create AES cipher: {e}"))?;

    let nonce = Nonce::from_slice(nonce);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| anyhow::anyhow!("decryption failed — wrong password or corrupted key file"))?;

    if plaintext.len() != 32 {
        bail!(
            "decrypted key has unexpected length {} (expected 32)",
            plaintext.len()
        );
    }

    let mut key = [0u8; 32];
    key.copy_from_slice(&plaintext);
    Ok(key)
}

/// Read the `CLAW_KEY_PASSWORD` env var, returning `None` if unset or empty.
fn read_password_env() -> Option<String> {
    std::env::var(PASSWORD_ENV).ok().filter(|p| !p.is_empty())
}

/// Build the encrypted key JSON representation.
fn build_encrypted_key_json(
    address: &[u8; 32],
    ciphertext: &[u8],
    salt: &[u8; 32],
    nonce: &[u8; 12],
    chain_id: &str,
) -> serde_json::Value {
    serde_json::json!({
        "address": hex::encode(address),
        "encrypted": true,
        "salt": hex::encode(salt),
        "nonce": hex::encode(nonce),
        "ciphertext": hex::encode(ciphertext),
        "chain_id": chain_id,
    })
}

/// Build the plaintext key JSON representation.
fn build_plaintext_key_json(
    address: &[u8; 32],
    secret_key: &[u8; 32],
    chain_id: &str,
) -> serde_json::Value {
    serde_json::json!({
        "address": hex::encode(address),
        "secret_key": hex::encode(secret_key),
        "chain_id": chain_id,
    })
}

/// Write key JSON to file with restricted permissions.
fn write_key_file(key_path: &Path, json: &serde_json::Value) -> Result<()> {
    fs::write(key_path, serde_json::to_string_pretty(json)?).context("write key file")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(key_path, fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialize a new node: create data dir, generate keypair, write config.
pub fn init_node(data_dir: &Path, chain_id: &str) -> Result<()> {
    fs::create_dir_all(data_dir).context("create data dir")?;

    let key_path = data_dir.join("key.json");
    if key_path.exists() {
        tracing::info!("Key already exists at {}", key_path.display());
        let cfg = load_config(data_dir)?;
        println!("Address: {}", hex::encode(cfg.address));
        // Still write default config if missing
        write_default_config(data_dir, chain_id)?;
        return Ok(());
    }

    let (sk, vk) = generate_keypair();
    let addr = vk.to_bytes();
    let sk_bytes = sk.to_bytes();

    let password = read_password_env();

    let key_data = match password {
        Some(ref pw) => {
            let (ciphertext, salt, nonce) = encrypt_key(&sk_bytes, pw)?;
            tracing::info!("Private key encrypted with PBKDF2 + AES-256-GCM");
            build_encrypted_key_json(&addr, &ciphertext, &salt, &nonce, chain_id)
        }
        None => {
            tracing::warn!(
                "Private key stored without encryption. Set CLAW_KEY_PASSWORD to enable encryption."
            );
            build_plaintext_key_json(&addr, &sk_bytes, chain_id)
        }
    };

    write_key_file(&key_path, &key_data)?;

    // Write default config.toml
    write_default_config(data_dir, chain_id)?;

    println!("Node initialized at {}", data_dir.display());
    println!("Address: {}", hex::encode(addr));

    Ok(())
}

/// Load node config from data dir.
///
/// Handles both plaintext and encrypted key.json formats.
pub fn load_config(data_dir: &Path) -> Result<NodeConfig> {
    let key_path = data_dir.join("key.json");
    let content = fs::read_to_string(&key_path).context("read key file")?;
    let json: serde_json::Value = serde_json::from_str(&content)?;

    let addr_hex = json["address"].as_str().context("missing address")?;
    let addr_bytes = hex::decode(addr_hex)?;
    let mut address = [0u8; 32];
    if addr_bytes.len() != 32 {
        bail!("address has unexpected length {} (expected 32)", addr_bytes.len());
    }
    address.copy_from_slice(&addr_bytes);

    let encrypted = json["encrypted"].as_bool().unwrap_or(false);

    let signing_key_bytes = if encrypted {
        let password = read_password_env()
            .context("key is encrypted but CLAW_KEY_PASSWORD env var is not set")?;

        let salt_hex = json["salt"].as_str().context("missing salt in encrypted key")?;
        let nonce_hex = json["nonce"].as_str().context("missing nonce in encrypted key")?;
        let ct_hex = json["ciphertext"]
            .as_str()
            .context("missing ciphertext in encrypted key")?;

        let salt_vec = hex::decode(salt_hex).context("decode salt")?;
        let nonce_vec = hex::decode(nonce_hex).context("decode nonce")?;
        let ciphertext = hex::decode(ct_hex).context("decode ciphertext")?;

        if salt_vec.len() != 32 {
            bail!("salt has unexpected length {} (expected 32)", salt_vec.len());
        }
        if nonce_vec.len() != 12 {
            bail!("nonce has unexpected length {} (expected 12)", nonce_vec.len());
        }

        let mut salt = [0u8; 32];
        salt.copy_from_slice(&salt_vec);
        let mut nonce = [0u8; 12];
        nonce.copy_from_slice(&nonce_vec);

        decrypt_key(&ciphertext, &password, &salt, &nonce)?
    } else {
        tracing::warn!(
            "Private key stored without encryption. Set CLAW_KEY_PASSWORD to enable encryption."
        );
        let sk_hex = json["secret_key"].as_str().context("missing secret_key")?;
        let sk_bytes = hex::decode(sk_hex)?;
        if sk_bytes.len() != 32 {
            bail!(
                "secret_key has unexpected length {} (expected 32)",
                sk_bytes.len()
            );
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&sk_bytes);
        key
    };

    Ok(NodeConfig {
        address,
        signing_key_bytes,
    })
}

/// Import a private key (32-byte Ed25519 seed) and save to key.json.
///
/// If `CLAW_KEY_PASSWORD` is set, the key is encrypted.
/// Returns the derived address.
pub fn import_key(data_dir: &Path, secret_key: &[u8; 32], chain_id: &str) -> Result<[u8; 32]> {
    fs::create_dir_all(data_dir).context("create data dir")?;

    let key_path = data_dir.join("key.json");
    if key_path.exists() {
        eprintln!("WARNING: Existing key will be overwritten at {}", key_path.display());
    }

    // Derive public key (address) from the secret key
    let signing_key = claw_crypto::ed25519_dalek::SigningKey::from_bytes(secret_key);
    let verifying_key = signing_key.verifying_key();
    let address = verifying_key.to_bytes();

    let password = read_password_env();

    let key_data = match password {
        Some(ref pw) => {
            let (ciphertext, salt, nonce) = encrypt_key(secret_key, pw)?;
            tracing::info!("Private key encrypted with PBKDF2 + AES-256-GCM");
            build_encrypted_key_json(&address, &ciphertext, &salt, &nonce, chain_id)
        }
        None => {
            tracing::warn!(
                "Private key stored without encryption. Set CLAW_KEY_PASSWORD to enable encryption."
            );
            build_plaintext_key_json(&address, secret_key, chain_id)
        }
    };

    write_key_file(&key_path, &key_data)?;
    write_default_config(data_dir, chain_id)?;

    Ok(address)
}

/// Export the private key from key.json.
///
/// Returns `(secret_key_bytes, address)`.
pub fn export_key(data_dir: &Path) -> Result<([u8; 32], [u8; 32])> {
    let cfg = load_config(data_dir)?;
    Ok((cfg.signing_key_bytes, cfg.address))
}

/// Encrypt an existing plaintext key.json in-place.
///
/// Reads the current key, encrypts it with the password from `CLAW_KEY_PASSWORD`,
/// and writes the encrypted format back.
pub fn encrypt_existing_key(data_dir: &Path) -> Result<()> {
    let key_path = data_dir.join("key.json");
    let content = fs::read_to_string(&key_path).context("read key file")?;
    let json: serde_json::Value = serde_json::from_str(&content)?;

    if json["encrypted"].as_bool().unwrap_or(false) {
        bail!("key.json is already encrypted");
    }

    let password =
        read_password_env().context("CLAW_KEY_PASSWORD env var must be set to encrypt the key")?;

    let addr_hex = json["address"].as_str().context("missing address")?;
    let sk_hex = json["secret_key"].as_str().context("missing secret_key")?;
    let chain_id = json["chain_id"]
        .as_str()
        .unwrap_or("claw-devnet");

    let addr_bytes = hex::decode(addr_hex)?;
    let sk_bytes = hex::decode(sk_hex)?;

    if addr_bytes.len() != 32 {
        bail!("address has unexpected length {} (expected 32)", addr_bytes.len());
    }
    if sk_bytes.len() != 32 {
        bail!("secret_key has unexpected length {} (expected 32)", sk_bytes.len());
    }

    let mut address = [0u8; 32];
    address.copy_from_slice(&addr_bytes);
    let mut secret_key = [0u8; 32];
    secret_key.copy_from_slice(&sk_bytes);

    let (ciphertext, salt, nonce) = encrypt_key(&secret_key, &password)?;

    let encrypted_json =
        build_encrypted_key_json(&address, &ciphertext, &salt, &nonce, chain_id);

    write_key_file(&key_path, &encrypted_json)?;

    println!("Key encrypted successfully.");
    println!("Address: {}", hex::encode(address));

    Ok(())
}
