use anyhow::Result;

/// Callback for nix_store_get_uri and other functions that return a string.
///
/// This function is used by the other nix_* crates, and you should never need to call it yourself.
///
/// Some functions in the nix library "return" strings without giving you ownership over them, by letting you pass a callback function that gets to look at that string. This callback simply turns that string pointer into an owned rust String.
///
/// # Safety
///
/// _Manual memory management_
///
/// Only for passing to the nix C API. Do not call this function directly.
pub unsafe extern "C" fn callback_get_result_string(
    start: *const ::std::os::raw::c_char,
    n: std::os::raw::c_uint,
    user_data: *mut std::os::raw::c_void,
) {
    let ret = user_data as *mut Result<String>;

    if start.is_null() {
        if n != 0 {
            panic!("callback_get_result_string: start is null but n is not zero");
        }
        *ret = Ok(String::new());
        return;
    }

    let slice = std::slice::from_raw_parts(start as *const u8, n as usize);

    if (*ret).is_ok() {
        panic!(
            "callback_get_result_string: Result must be initialized to Err. Did Nix call us twice?"
        );
    }

    *ret = String::from_utf8(slice.to_vec())
        .map_err(|e| anyhow::format_err!("Nix string is not valid UTF-8: {}", e));
}

pub fn callback_get_result_string_data(vec: &mut Result<String>) -> *mut std::os::raw::c_void {
    vec as *mut Result<String> as *mut std::os::raw::c_void
}

#[macro_export]
macro_rules! result_string_init {
    () => {
        Err(anyhow::anyhow!("String was not set by Nix C API"))
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix_c_raw as raw;

    /// Typecheck the function signature against the generated bindings in nix_c_raw.
    static _CALLBACK_GET_RESULT_STRING: raw::get_string_callback = Some(callback_get_result_string);

    #[test]
    fn test_callback_get_result_string_empty() {
        let mut ret: Result<String> = result_string_init!();
        let start: *const std::os::raw::c_char = std::ptr::null();
        let n: std::os::raw::c_uint = 0;
        let user_data: *mut std::os::raw::c_void = callback_get_result_string_data(&mut ret);

        unsafe {
            callback_get_result_string(start, n, user_data);
        }

        let s = ret.unwrap();
        assert_eq!(s, "");
    }

    #[test]
    fn test_callback_result_string() {
        let mut ret: Result<String> = result_string_init!();
        let start: *const std::os::raw::c_char = b"helloGARBAGE".as_ptr() as *const i8;
        let n: std::os::raw::c_uint = 5;
        let user_data: *mut std::os::raw::c_void = callback_get_result_string_data(&mut ret);
        unsafe {
            callback_get_result_string(start, n, user_data);
        }

        let s = ret.unwrap();
        assert_eq!(s, "hello");
    }
}
