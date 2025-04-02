use anyhow::{Context as _, Result};
use nix_c_raw as raw;
use nix_util::context::{self, Context};
use std::ptr::NonNull;

pub struct FetchersSettings {
    pub(crate) ptr: NonNull<raw::fetchers_settings>,
}
impl Drop for FetchersSettings {
    fn drop(&mut self) {
        unsafe {
            raw::fetchers_settings_free(self.ptr.as_ptr());
        }
    }
}
impl FetchersSettings {
    pub fn new() -> Result<Self> {
        let mut ctx = Context::new();
        let ptr = unsafe { context::check_call!(raw::fetchers_settings_new(&mut ctx))? };
        Ok(FetchersSettings {
            ptr: NonNull::new(ptr).context("fetchers_settings_new unexpectedly returned null")?,
        })
    }

    pub fn raw_ptr(&self) -> *mut raw::fetchers_settings {
        self.ptr.as_ptr()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetchers_settings_new() {
        let _ = FetchersSettings::new().unwrap();
    }
}
