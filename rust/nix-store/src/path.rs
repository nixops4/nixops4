use anyhow::Result;
use nix_c_raw as raw;
use nix_util::{
    result_string_init,
    string_return::{callback_get_result_string, callback_get_result_string_data},
};

pub struct StorePath {
    raw: *mut raw::StorePath,
}
impl StorePath {
    /// This is a low level function that you shouldn't have to call unless you are developing the Nix bindings.
    ///
    /// Construct a new `StorePath` by first cloning the C store path.
    /// This does not take ownership of the C store path, so it should be a borrowed value, or you should free it.
    pub fn new_raw_clone(raw: *const raw::StorePath) -> Self {
        Self::new_raw(unsafe { raw::store_path_clone(raw as *mut raw::StorePath) })
    }
    /// This is a low level function that you shouldn't have to call unless you are developing the Nix bindings.
    ///
    /// Takes ownership of a C `nix_store_path`. It will be freed when the `StorePath` is dropped.
    pub fn new_raw(raw: *mut raw::StorePath) -> Self {
        StorePath { raw }
    }
    pub fn name(&self) -> Result<String> {
        unsafe {
            let mut r = result_string_init!();
            raw::store_path_name(
                self.raw,
                Some(callback_get_result_string),
                callback_get_result_string_data(&mut r),
            );
            r
        }
    }
}
impl Drop for StorePath {
    fn drop(&mut self) {
        unsafe {
            raw::store_path_free(self.raw);
        }
    }
}
