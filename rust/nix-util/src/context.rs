use anyhow::{bail, Result};
use nix_c_raw as raw;
use std::ptr::null_mut;
use std::ptr::NonNull;

pub struct Context {
    inner: NonNull<raw::c_context>,
}

impl Context {
    pub fn new() -> Self {
        let ctx = unsafe { raw::c_context_create() };
        if ctx.is_null() {
            // We've failed to allocate a (relatively small) Context struct.
            // We're almost certainly going to crash anyways.
            panic!("nix_c_context_create returned a null pointer");
        }
        let ctx = Context {
            inner: NonNull::new(ctx).unwrap(),
        };
        ctx
    }

    /// This is currently private because of the switch to `check_one_call`, which is more ergonomic.
    /// The pattern its callers use make it hard to forget to use this function.
    ///
    /// If we have a good use case for `check_err`, we can expose it again.
    fn ptr(&self) -> *mut raw::c_context {
        self.inner.as_ptr()
    }

    /// Check the error code and return an error if it's not `NIX_OK`.
    ///
    /// This is currently private because of the switch to `check_one_call`, which is more ergonomic.
    /// The pattern its callers use make it hard to forget to use this function.
    ///
    /// If we have a good use case for `check_err`, we can expose it again.
    fn check_err(&self) -> Result<()> {
        let err = unsafe { raw::err_code(self.inner.as_ptr()) };
        if err != raw::NIX_OK.try_into().unwrap() {
            // msgp is a borrowed pointer (pointing into the context), so we don't need to free it
            let msgp = unsafe { raw::err_msg(null_mut(), self.inner.as_ptr(), null_mut()) };
            // Turn the i8 pointer into a Rust string by copying
            let msg: &str = unsafe { core::ffi::CStr::from_ptr(msgp).to_str()? };
            bail!("{}", msg);
        }
        Ok(())
    }

    fn clear(&self) {
        unsafe {
            raw::set_err_msg(
                self.inner.as_ptr(),
                raw::NIX_OK.try_into().unwrap(),
                b"\0".as_ptr() as *const i8,
            );
        }
    }

    pub fn check_err_and_clear(&self) -> Result<()> {
        let r = self.check_err();
        if r.is_err() {
            self.clear();
        }
        r
    }

    /// Run the function, and check the error, then reset the error.
    /// Make at most one call to a Nix function in `f`.
    /// Do not use if the context isn't fresh or cleared (e.g. with `check_err_and_clear`).
    pub fn check_one_call<T, F: FnOnce(*mut raw::c_context) -> T>(&self, f: F) -> Result<T> {
        let t = f(self.ptr());
        self.check_err_and_clear()?;
        Ok(t)
    }

    pub fn check_one_call_or_key_none<T, F: FnOnce(*mut raw::c_context) -> T>(
        &self,
        f: F,
    ) -> Result<Option<T>> {
        let t = f(self.ptr());
        if unsafe { raw::err_code(self.inner.as_ptr()) == raw::NIX_ERR_KEY } {
            self.clear();
            return Ok(None);
        }
        self.check_err_and_clear()?;
        Ok(Some(t))
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        unsafe {
            raw::c_context_free(self.inner.as_ptr());
        }
    }
}

#[macro_export]
macro_rules! check_call {
    ($f:path[$ctx:expr $(, $arg:expr)*]) => {
        {
            let ret = $f($ctx.ptr() $(, $arg)*);
            match $ctx.check_err() {
                Ok(_) => Ok(ret),
                Err(e) => {
                    $ctx.clear();
                    Err(e)
                }
            }
        }
    }
}

pub use check_call;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_new_and_drop() {
        // don't crash
        let _c = Context::new();
    }
}
