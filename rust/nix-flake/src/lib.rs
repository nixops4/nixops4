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
    fn add_to_eval_state_builder(
        &self,
        builder: &mut nix_expr::eval_state::EvalStateBuilder,
    ) -> Result<()> {
        let mut ctx = Context::new();
        unsafe {
            context::check_call!(raw::flake_settings_add_to_eval_state_builder(
                &mut ctx,
                self.ptr,
                builder.raw_ptr()
            ))
        }?;
        Ok(())
    }
}

pub trait EvalStateBuilderExt {
    fn flakes(self, settings: &FlakeSettings) -> Result<nix_expr::eval_state::EvalStateBuilder>;
}
impl EvalStateBuilderExt for nix_expr::eval_state::EvalStateBuilder {
    fn flakes(
        mut self,
        settings: &FlakeSettings,
    ) -> Result<nix_expr::eval_state::EvalStateBuilder> {
        settings.add_to_eval_state_builder(&mut self)?;
        Ok(self)
    }
}
