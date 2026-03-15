use crate::constants::MAX_CONTRACT_CODE_SIZE;
use crate::error::VmError;

/// Validate Wasm bytecode before deployment.
pub fn validate_contract_code(code: &[u8]) -> Result<(), VmError> {
    if code.is_empty() {
        return Err(VmError::InvalidModule("empty wasm bytecode".to_string()));
    }
    if code.len() > MAX_CONTRACT_CODE_SIZE {
        return Err(VmError::CodeTooLarge {
            size: code.len(),
            max: MAX_CONTRACT_CODE_SIZE,
        });
    }
    // Wasm magic number check
    if code.len() < 4 || &code[0..4] != b"\0asm" {
        return Err(VmError::InvalidModule(
            "invalid wasm magic number".to_string(),
        ));
    }
    Ok(())
}
