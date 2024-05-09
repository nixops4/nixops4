/// Callback for nix_store_get_uri and other functions that return a string.
///
/// This function is used by the other nix_* crates, and you should never need to call it yourself.
///
/// Some functions in the nix library "return" strings without giving you ownership over them, by letting you pass a callback function that gets to look at that string. This callback simply turns that string pointer into an owned rust String.
pub unsafe extern "C" fn callback_get_vec_u8(
    start: *const ::std::os::raw::c_char,
    n: std::os::raw::c_uint,
    user_data: *mut std::os::raw::c_void,
) {
    let ret = user_data as *mut Vec<u8>;
    let slice = std::slice::from_raw_parts(start as *const u8, n as usize);
    if !(*ret).is_empty() {
        panic!("callback_get_vec_u8: slice must be empty. Were we called twice?");
    }
    (*ret).extend_from_slice(slice);
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix_c_raw as raw;

    /// Typecheck the function signature against the generated bindings in nix_c_raw.
    static _CALLBACK_GET_VEC_U8: raw::get_string_callback = Some(callback_get_vec_u8);

    #[test]
    fn test_callback_get_vec_u8_empty() {
        let mut ret: Vec<u8> = Vec::new();
        let start: *const std::os::raw::c_char = std::ptr::null();
        let n: std::os::raw::c_uint = 0;
        let user_data: *mut std::os::raw::c_void =
            &mut ret as *mut Vec<u8> as *mut std::os::raw::c_void;

        unsafe {
            callback_get_vec_u8(start, n, user_data);
        }

        assert_eq!(ret, vec![]);
    }

    #[test]
    fn test_callback_get_vec_u8() {
        let mut ret: Vec<u8> = Vec::new();
        let start: *const std::os::raw::c_char = b"helloGARBAGE".as_ptr() as *const i8;
        let n: std::os::raw::c_uint = 5;
        let user_data: *mut std::os::raw::c_void =
            &mut ret as *mut Vec<u8> as *mut std::os::raw::c_void;

        unsafe {
            callback_get_vec_u8(start, n, user_data);
        }

        assert_eq!(ret, b"hello".to_vec());
    }
}
