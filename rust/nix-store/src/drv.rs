use anyhow::Result;
use std::ptr::NonNull;

use nix_c_raw as raw;
use nix_util::context::Context;
use nix_util::string_return::{callback_get_result_string, callback_get_result_string_data};
use nix_util::{check_call, result_string_init};

pub struct Derivation {
    raw: NonNull<raw::derivation>,
    /* An error context to reuse. This way we don't have to allocate them for each store operation. */
    context: Context,
}
impl Derivation {
    /// This is a low level function that you shouldn't have to call unless you are developing the Nix bindings.
    ///
    /// Construct a new `Derivation` by first cloning the C derivation.
    ///
    /// # Safety
    ///
    /// This does not take ownership of the C store path, so it should be a borrowed pointer, or you should free it.
    pub unsafe fn new_raw_clone(raw: NonNull<raw::derivation>) -> Self {
        Self::new_raw(
            NonNull::new(raw::derivation_clone(raw.as_ptr()))
                .or_else(|| panic!("nix_derivation_clone returned a null pointer"))
                .unwrap(),
        )
    }

    /// This is a low level function that you shouldn't have to call unless you are developing the Nix bindings.
    ///
    /// Takes ownership of a C `nix_derivation`. It will be freed when the `Derivation` is dropped.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the provided `NonNull<raw::derivation>` is valid and that the ownership
    /// semantics are correctly followed. The `raw` pointer must not be used after being passed to this function.
    pub unsafe fn new_raw(raw: NonNull<raw::derivation>) -> Self {
        Derivation {
            raw,
            context: Context::new(),
        }
    }

    /// This is a low level function that you shouldn't have to call unless you are developing the Nix bindings.
    ///
    /// Get a pointer to the underlying Nix C API derivation.
    ///
    /// # Safety
    ///
    /// This function is unsafe because it returns a raw pointer. The caller must ensure that the pointer is not used beyond the lifetime of this `Derivation`.
    pub unsafe fn as_ptr(&self) -> *mut nix_c_raw::derivation {
        self.raw.as_ptr()
    }

    #[doc(alias = "nix_derivation_to_json")]
    pub fn to_json_string(&mut self) -> Result<String> {
        let mut r = result_string_init!();
        unsafe {
            check_call!(raw::derivation_to_json(
                &mut self.context,
                self.raw.as_ptr(),
                Some(callback_get_result_string),
                callback_get_result_string_data(&mut r)
            ))
        }?;
        r
    }
}
impl Drop for Derivation {
    fn drop(&mut self) {
        unsafe {
            raw::derivation_free(self.as_ptr());
        }
    }
}
