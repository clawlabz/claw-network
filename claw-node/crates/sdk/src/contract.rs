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
