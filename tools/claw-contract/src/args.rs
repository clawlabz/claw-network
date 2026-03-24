use anyhow::{bail, Result};

/// Parse a hex string (with or without 0x prefix) into bytes.
/// An empty string or "0x" alone returns an empty Vec.
pub fn parse_hex_or_empty(input: &str) -> Result<Vec<u8>> {
    let stripped = input.strip_prefix("0x").unwrap_or(input);
    if stripped.is_empty() {
        return Ok(Vec::new());
    }
    if stripped.len() % 2 != 0 {
        bail!("hex string has odd length: {}", stripped.len());
    }
    hex::decode(stripped).map_err(|e| anyhow::anyhow!("invalid hex: {e}"))
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn uppercase_hex_is_accepted() {
        let result = parse_hex_or_empty("0xDEADBEEF").unwrap();
        assert_eq!(result, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }
}
