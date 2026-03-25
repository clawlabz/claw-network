//! Contract helper macros for entry-point boilerplate.
//!
//! Provides [`setup_alloc!`], [`entry!`], and [`require!`].

/// Generate the `alloc` function required by the ClawNetwork VM to pass
/// arguments into the WASM module.
///
/// Call this exactly once in your contract's `lib.rs`:
///
/// ```ignore
/// claw_sdk::setup_alloc!();
/// ```
#[macro_export]
macro_rules! setup_alloc {
    () => {
        #[no_mangle]
        pub extern "C" fn alloc(size: i32) -> *mut u8 {
            let layout = core::alloc::Layout::from_size_align(size as usize, 1).unwrap();
            unsafe { std::alloc::alloc(layout) }
        }
    };
}

/// Deserialize borsh-encoded arguments, execute a closure, and set return data.
///
/// # Usage
///
/// ```ignore
/// #[no_mangle]
/// pub extern "C" fn my_method(args_ptr: i32, args_len: i32) {
///     claw_sdk::entry!(args_ptr, args_len, |args: MyArgs| {
///         // handle args ...
///         b"ok".to_vec()
///     });
/// }
/// ```
#[macro_export]
macro_rules! entry {
    ($ptr:expr, $len:expr, |$args:ident : $ty:ty| $body:block) => {{
        let slice = unsafe { core::slice::from_raw_parts($ptr as *const u8, $len as usize) };
        let $args: $ty = borsh::from_slice(slice).expect("failed to deserialize args");
        let result: Vec<u8> = $body;
        if !result.is_empty() {
            $crate::env::set_return_data(&result);
        }
    }};
}

/// Require a condition or abort execution with an error message.
///
/// ```ignore
/// claw_sdk::require!(amount > 0, "amount must be positive");
/// ```
#[macro_export]
macro_rules! require {
    ($cond:expr, $msg:expr) => {
        if !($cond) {
            $crate::env::panic_msg($msg);
        }
    };
}

/// Define a contract method that receives borsh-encoded arguments.
/// Generates the `#[no_mangle] extern "C" fn` wrapper with automatic
/// deserialization and return-data handling.
///
/// # Usage
///
/// ```ignore
/// claw_sdk::contract_method!(transfer, TransferArgs, |args| {
///     // process args ...
///     borsh::to_vec(&receipt).unwrap()
/// });
/// ```
#[macro_export]
macro_rules! contract_method {
    ($name:ident, $args_type:ty, $body:expr) => {
        #[no_mangle]
        pub extern "C" fn $name(args_ptr: i32, args_len: i32) {
            $crate::entry!(args_ptr, args_len, |args: $args_type| { $body(args) });
        }
    };
}

/// Define a contract method with no arguments.
///
/// The body must return a `Vec<u8>`. If non-empty it is set as return data.
///
/// ```ignore
/// claw_sdk::contract_method_no_args!(get_count, || {
///     let count = claw_sdk::storage::get(b"count").unwrap_or_default();
///     count
/// });
/// ```
#[macro_export]
macro_rules! contract_method_no_args {
    ($name:ident, $body:expr) => {
        #[no_mangle]
        pub extern "C" fn $name(_args_ptr: i32, _args_len: i32) {
            let result: Vec<u8> = $body();
            if !result.is_empty() {
                $crate::env::set_return_data(&result);
            }
        }
    };
}

/// Define a view method (same as [`contract_method!`] but communicates intent).
///
/// View methods are read-only by convention; the macro is identical to
/// `contract_method!` but makes the contract source more self-documenting.
#[macro_export]
macro_rules! view_method {
    ($name:ident, $args_type:ty, $body:expr) => {
        $crate::contract_method!($name, $args_type, $body);
    };
}

/// Define a view method with no arguments.
///
/// Same as [`contract_method_no_args!`] but signals read-only intent.
#[macro_export]
macro_rules! view_method_no_args {
    ($name:ident, $body:expr) => {
        $crate::contract_method_no_args!($name, $body);
    };
}

/// Emit a structured contract event.
///
/// Accepts a topic string literal (or expression) and an optional data argument.
/// Data may be any `&[u8]` expression; if omitted, an empty payload is used.
///
/// # Examples
///
/// ```ignore
/// // Topic only (empty data):
/// claw_sdk::emit!("transfer");
///
/// // Topic + raw bytes:
/// claw_sdk::emit!("transfer", &amount.to_le_bytes());
///
/// // Topic + borsh-encoded payload:
/// let payload = borsh::to_vec(&my_struct).unwrap();
/// claw_sdk::emit!("my_event", &payload);
/// ```
#[macro_export]
macro_rules! emit {
    ($topic:expr) => {
        $crate::env::emit_event_raw($topic, &[])
    };
    ($topic:expr, $data:expr) => {
        $crate::env::emit_event_raw($topic, $data)
    };
}
