use anyhow::Result;
use nix_c_raw as raw;
use nix_util::context::{self, Context};

pub struct FlakeSettings {
    pub(crate) ptr: *mut raw::flake_settings,
}
impl Drop for FlakeSettings {
    fn drop(&mut self) {
        unsafe {
            raw::flake_settings_free(self.ptr);
        }
    }
}
impl FlakeSettings {
    pub fn new() -> Result<Self> {
        let mut ctx = Context::new();
        let s = unsafe { context::check_call!(raw::flake_settings_new(&mut ctx)) }?;
        Ok(FlakeSettings { ptr: s })
    }
    pub fn init_globally(&mut self) -> Result<()> {
        let mut ctx = Context::new();
        unsafe { context::check_call!(raw::flake_init_global(&mut ctx, self.ptr)) }?;
        Ok(())
    }
}
