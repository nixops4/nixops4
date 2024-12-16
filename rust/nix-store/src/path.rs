use std::ptr::NonNull;

use anyhow::Result;
use nix_c_raw as raw;
use nix_util::{
    result_string_init,
    string_return::{callback_get_result_string, callback_get_result_string_data},
};

pub struct StorePath {
    raw: NonNull<raw::StorePath>,
}
impl StorePath {
    /// This is a low level function that you shouldn't have to call unless you are developing the Nix bindings.
    ///
    /// Construct a new `StorePath` by first cloning the C store path.
    /// This does not take ownership of the C store path, so it should be a borrowed value, or you should free it.
    pub fn new_raw_clone(raw: NonNull<raw::StorePath>) -> Self {
        Self::new_raw(
            NonNull::new(unsafe { raw::store_path_clone(raw.as_ptr()) })
                .or_else(|| panic!("nix_store_path_clone returned a null pointer"))
                .unwrap(),
        )
    }
    /// This is a low level function that you shouldn't have to call unless you are developing the Nix bindings.
    ///
    /// Takes ownership of a C `nix_store_path`. It will be freed when the `StorePath` is dropped.
    pub fn new_raw(raw: NonNull<raw::StorePath>) -> Self {
        StorePath { raw }
    }
    pub fn name(&self) -> Result<String> {
        unsafe {
            let mut r = result_string_init!();
            raw::store_path_name(
                self.as_ptr(),
                Some(callback_get_result_string),
                callback_get_result_string_data(&mut r),
            );
            r
        }
    }

    pub fn as_ptr(&self) -> *mut nix_c_raw::StorePath {
        self.raw.as_ptr()
    }
}
impl Drop for StorePath {
    fn drop(&mut self) {
        unsafe {
            raw::store_path_free(self.as_ptr());
        }
    }
}
