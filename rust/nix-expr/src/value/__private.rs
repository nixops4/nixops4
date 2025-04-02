//! Functions that are relevant for other bindings modules, but normally not end users.
use super::Value;
use nix_c_raw as raw;

/// See [Value::new].
pub unsafe fn raw_value_new(ptr: *mut raw::Value) -> Value {
    Value::new(ptr)
}

/// See [Value::new_borrowed].
pub unsafe fn raw_value_new_borrowed(ptr: *mut raw::Value) -> Value {
    Value::new_borrowed(ptr)
}
