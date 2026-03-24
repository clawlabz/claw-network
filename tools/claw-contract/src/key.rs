use anyhow::{bail, Context, Result};
use ed25519_dalek::SigningKey;
use std::path::Path;

/// Load a signing key from either a raw hex string (64 hex chars, optionally
/// prefixed with "0x") or from a file that contains such a hex string.
///
/// Detection heuristic:
///   - If the input looks like an existing file path → read the file.
///   - Otherwise treat the input as an inline hex string.
pub fn load_signing_key(input: &str) -> Result<SigningKey> {
    // Check if input is a file path that exists
    let hex_str = if Path::new(input).exists() {
        let contents = std::fs::read_to_string(input)
            .with_context(|| format!("reading key file: {input}"))?;
        contents.trim().to_string()
    } else {
        // Check if it looks like a file path but doesn't exist
        if input.contains('/') || input.contains('\\') {
            bail!("key file not found: {input}");
        }
        input.to_string()
    };

    parse_hex_key(&hex_str)
}

fn parse_hex_key(hex_str: &str) -> Result<SigningKey> {
    let stripped = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    if stripped.len() != 64 {
        bail!(
            "signing key must be 32 bytes (64 hex chars), got {} chars",
            stripped.len()
        );
    }
    let bytes = hex::decode(stripped).context("invalid hex in signing key")?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("expected exactly 32 bytes"))?;
    Ok(SigningKey::from_bytes(&arr))
}
