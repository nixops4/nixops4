use anyhow::Result;
use nix_c_raw as raw;
use nix_util::string_return::{callback_get_vec_u8, callback_get_vec_u8_data};

pub struct StorePath {
    raw: *mut raw::StorePath,
}
impl StorePath {
    /**
     * This is a low level function that you shouldn't have to call unless you are developing the Nix bindings.
     *
     * Construct a new `StorePath` by first cloning the C store path.
     * This does not take ownership of the C store path, so it should be a borrowed value, or you should free it.
     */
    pub fn new_raw_clone(raw: *const raw::StorePath) -> Self {
        Self::new_raw(unsafe { raw::store_path_clone(raw as *mut raw::StorePath) })
    }
    /**
     * This is a low level function that you shouldn't have to call unless you are developing the Nix bindings.
     *
     * Takes ownership of a C `nix_store_path`. It will be freed when the `StorePath` is dropped.
     */
    pub fn new_raw(raw: *mut raw::StorePath) -> Self {
        StorePath { raw }
    }
    pub fn name(&self) -> Result<String> {
        unsafe {
            let mut vec = Vec::new();
            raw::store_path_name(
                self.raw,
                Some(callback_get_vec_u8),
                callback_get_vec_u8_data(&mut vec),
            );
            String::from_utf8(vec).map_err(|e| e.into())
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
