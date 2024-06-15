use anyhow::Result;
use nix_c_raw as raw;

use crate::{
    check_call, context, result_string_init,
    string_return::{callback_get_result_string, callback_get_result_string_data},
};

pub fn set(key: &str, value: &str) -> Result<()> {
    let mut ctx = context::Context::new();
    let key = std::ffi::CString::new(key)?;
    let value = std::ffi::CString::new(value)?;
    unsafe {
        check_call!(raw::setting_set[&mut ctx, key.as_ptr(), value.as_ptr()])?;
    }
    Ok(())
}

pub fn get(key: &str) -> Result<String> {
    let mut ctx = context::Context::new();
    let key = std::ffi::CString::new(key)?;
    let mut r: Result<String> = result_string_init!();
    unsafe {
        check_call!(raw::setting_get[&mut ctx, key.as_ptr(), Some(callback_get_result_string), callback_get_result_string_data(&mut r)])?;
    }
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
            check_call!(raw::libstore_init[&mut ctx]).unwrap();
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
