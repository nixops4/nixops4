use anyhow::Result;
use nix_c_raw as raw;

use crate::{
    context,
    string_return::{callback_get_vec_u8, callback_get_vec_u8_data},
};

pub fn set(key: &str, value: &str) -> Result<()> {
    let ctx = context::Context::new();
    let key = std::ffi::CString::new(key)?;
    let value = std::ffi::CString::new(value)?;
    unsafe {
        raw::nix_setting_set(ctx.ptr(), key.as_ptr(), value.as_ptr());
    };
    ctx.check_err()
}

pub fn get(key: &str) -> Result<String> {
    let ctx = context::Context::new();
    let key = std::ffi::CString::new(key)?;
    let mut raw_buffer: Vec<u8> = Vec::new();
    unsafe {
        raw::nix_setting_get(
            ctx.ptr(),
            key.as_ptr(),
            callback_get_vec_u8 as *mut std::ffi::c_void,
            callback_get_vec_u8_data(&mut raw_buffer),
        )
    };
    ctx.check_err()?;
    String::from_utf8(raw_buffer).map_err(|e| e.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctor::ctor;

    #[ctor]
    fn setup() {
        let ctx = context::Context::new();
        unsafe {
            nix_c_raw::nix_libstore_init(ctx.ptr());
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
