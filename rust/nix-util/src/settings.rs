use anyhow::Result;
use nix_c_raw as raw;
use std::sync::Mutex;

use crate::{
    check_call, context, result_string_init,
    string_return::{callback_get_result_string, callback_get_result_string_data},
};

// Global mutex to protect concurrent access to Nix settings
// See the documentation on `set()` for important thread safety information.
static SETTINGS_MUTEX: Mutex<()> = Mutex::new(());

/// Set a Nix setting.
///
/// # Thread Safety
///
/// This function uses a mutex to serialize access through the Rust API.
/// However, the underlying Nix settings system uses global mutable state
/// without internal synchronization.
///
/// The mutex provides protection between Rust callers but cannot prevent:
/// - C++ Nix code from modifying settings concurrently
/// - Other Nix operations from reading settings during modification
///
/// For multi-threaded applications, ensure that no other Nix operations
/// are running while changing settings. Settings are best modified during
/// single-threaded initialization.
pub fn set(key: &str, value: &str) -> Result<()> {
    // Lock the mutex to ensure thread-safe access to global settings
    let guard = SETTINGS_MUTEX.lock().unwrap();

    let mut ctx = context::Context::new();
    let key = std::ffi::CString::new(key)?;
    let value = std::ffi::CString::new(value)?;
    unsafe {
        check_call!(raw::setting_set(&mut ctx, key.as_ptr(), value.as_ptr()))?;
    }
    drop(guard);
    Ok(())
}

/// Get a Nix setting.
///
/// # Thread Safety
///
/// See the documentation on [`set()`] for important thread safety information.
pub fn get(key: &str) -> Result<String> {
    // Lock the mutex to ensure thread-safe access to global settings
    let guard = SETTINGS_MUTEX.lock().unwrap();

    let mut ctx = context::Context::new();
    let key = std::ffi::CString::new(key)?;
    let mut r: Result<String> = result_string_init!();
    unsafe {
        check_call!(raw::setting_get(
            &mut ctx,
            key.as_ptr(),
            Some(callback_get_result_string),
            callback_get_result_string_data(&mut r)
        ))?;
    }
    drop(guard);
    r
}

#[cfg(test)]
mod tests {
    use crate::check_call;

    use super::*;

    #[ctor::ctor]
    fn setup() {
        let mut ctx = context::Context::new();
        unsafe {
            check_call!(raw::libstore_init(&mut ctx)).unwrap();
        }
    }

    #[test]
    fn set_get() {
        // Something that shouldn't matter if it's a different value temporarily
        let key = "user-agent-suffix";

        // Save the old value, in case it's important. Probably not.
        // If this doesn't work, pick a different setting to test with
        let old_value = get(key).unwrap();

        let new_value = "just a string that we're storing into some option for testing purposes";

        let res_e = (|| {
            set(key, new_value)?;
            get(key)
        })();

        // Restore immediately; try not to affect other tests (if relevant).
        set(key, old_value.as_str()).unwrap();

        let res = res_e.unwrap();

        assert_eq!(res, new_value);
    }
}
