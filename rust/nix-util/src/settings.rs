use anyhow::Result;
use nix_c_raw as raw;

use crate::{
    context, result_string_init,
    string_return::{callback_get_result_string, callback_get_result_string_data},
};

pub fn set(key: &str, value: &str) -> Result<()> {
    let ctx = context::Context::new();
    let key = std::ffi::CString::new(key)?;
    let value = std::ffi::CString::new(value)?;
    unsafe {
        raw::setting_set(ctx.ptr(), key.as_ptr(), value.as_ptr());
    };
    ctx.check_err()
}

pub fn get(key: &str) -> Result<String> {
    let ctx = context::Context::new();
    let key = std::ffi::CString::new(key)?;
    let mut r: Result<String> = result_string_init!();
    unsafe {
        raw::setting_get(
            ctx.ptr(),
            key.as_ptr(),
            Some(callback_get_result_string),
            callback_get_result_string_data(&mut r),
        )
    };
    ctx.check_err()?;
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[ctor::ctor]
    fn setup() {
        let ctx = context::Context::new();
        unsafe {
            nix_c_raw::libstore_init(ctx.ptr());
        };
        ctx.check_err().unwrap();
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
