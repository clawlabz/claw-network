use thiserror::Error;

#[derive(Debug, Error)]
pub enum VmError {
    #[error("compilation failed: {0}")]
    CompilationFailed(String),
    #[error("instantiation failed: {0}")]
    InstantiationFailed(String),
    #[error("execution failed: {0}")]
    ExecutionFailed(String),
    #[error("out of fuel: used {used}, limit {limit}")]
    OutOfFuel { used: u64, limit: u64 },
    #[error("invalid wasm module: {0}")]
    InvalidModule(String),
    #[error("memory access error: {0}")]
    MemoryError(String),
    #[error("host function error: {0}")]
    HostError(String),
    #[error("method not found: {0}")]
    MethodNotFound(String),
    #[error("wasm code too large: {size} bytes (max {max})")]
    CodeTooLarge { size: usize, max: usize },
    #[error("contract aborted: {reason} (fuel consumed: {fuel_consumed})")]
    ContractAbort { reason: String, fuel_consumed: u64 },
    #[error("wasm memory limit exceeded: {pages} pages requested (max {max})")]
    MemoryLimitExceeded { pages: u32, max: u32 },
}
