use std::ptr::NonNull;

use anyhow::{Context as _, Result};
use nix_c_raw as raw;
use nix_expr::eval_state::EvalState;
use nix_fetchers::FetchersSettings;
use nix_util::{
    context::{self, Context},
    result_string_init,
    string_return::{callback_get_result_string, callback_get_result_string_data},
};

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

pub struct FlakeReferenceParseFlags {
    pub(crate) ptr: NonNull<raw::flake_reference_parse_flags>,
}
impl Drop for FlakeReferenceParseFlags {
    fn drop(&mut self) {
        unsafe {
            raw::flake_reference_parse_flags_free(self.ptr.as_ptr());
        }
    }
}
impl FlakeReferenceParseFlags {
    pub fn new(settings: &FlakeSettings) -> Result<Self> {
        let mut ctx = Context::new();
        let ptr = unsafe {
            context::check_call!(raw::flake_reference_parse_flags_new(&mut ctx, settings.ptr))
        }?;
        let ptr = NonNull::new(ptr)
            .context("flake_reference_parse_flags_new unexpectedly returned null")?;
        Ok(FlakeReferenceParseFlags { ptr })
    }
    pub fn set_base_directory(&mut self, base_directory: &str) -> Result<()> {
        let mut ctx = Context::new();
        unsafe {
            context::check_call!(raw::flake_reference_parse_flags_set_base_directory(
                &mut ctx,
                self.ptr.as_ptr(),
                base_directory.as_ptr() as *const i8,
                base_directory.len()
            ))
        }?;
        Ok(())
    }
}

pub struct FlakeReference {
    pub(crate) ptr: NonNull<raw::flake_reference>,
}
impl Drop for FlakeReference {
    fn drop(&mut self) {
        unsafe {
            raw::flake_reference_free(self.ptr.as_ptr());
        }
    }
}
impl FlakeReference {
    pub fn parse_with_fragment(
        fetch_settings: &FetchersSettings,
        flake_settings: &FlakeSettings,
        flags: &FlakeReferenceParseFlags,
        reference: &str,
    ) -> Result<(FlakeReference, String)> {
        let mut ctx = Context::new();
        let mut r = result_string_init!();
        let mut ptr: *mut raw::flake_reference = std::ptr::null_mut();
        unsafe {
            context::check_call!(raw::flake_reference_and_fragment_from_string(
                &mut ctx,
                fetch_settings.raw_ptr(),
                flake_settings.ptr,
                flags.ptr.as_ptr(),
                reference.as_ptr() as *const i8,
                reference.len(),
                // pointer to ptr
                &mut ptr,
                Some(callback_get_result_string),
                callback_get_result_string_data(&mut r)
            ))
        }?;
        let ptr = NonNull::new(ptr)
            .context("flake_reference_and_fragment_from_string unexpectedly returned null")?;
        Ok((FlakeReference { ptr: ptr }, r?))
    }
}

pub struct FlakeLockFlags {
    pub(crate) ptr: *mut raw::flake_lock_flags,
}
impl Drop for FlakeLockFlags {
    fn drop(&mut self) {
        unsafe {
            raw::flake_lock_flags_free(self.ptr);
        }
    }
}
impl FlakeLockFlags {
    pub fn new(settings: &FlakeSettings) -> Result<Self> {
        let mut ctx = Context::new();
        let s = unsafe { context::check_call!(raw::flake_lock_flags_new(&mut ctx, settings.ptr)) }?;
        Ok(FlakeLockFlags { ptr: s })
    }
}

pub struct LockedFlake {
    pub(crate) ptr: NonNull<raw::locked_flake>,
}
impl Drop for LockedFlake {
    fn drop(&mut self) {
        unsafe {
            raw::locked_flake_free(self.ptr.as_ptr());
        }
    }
}
impl LockedFlake {
    pub fn lock(
        fetch_settings: &FetchersSettings,
        flake_settings: &FlakeSettings,
        eval_state: &EvalState,
        flags: &FlakeLockFlags,
        flake_ref: &FlakeReference,
    ) -> Result<LockedFlake> {
        let mut ctx = Context::new();
        let ptr = unsafe {
            context::check_call!(raw::flake_lock(
                &mut ctx,
                fetch_settings.raw_ptr(),
                flake_settings.ptr,
                eval_state.raw_ptr(),
                flags.ptr,
                flake_ref.ptr.as_ptr()
            ))
        }?;
        let ptr = NonNull::new(ptr).context("flake_lock unexpectedly returned null")?;
        Ok(LockedFlake { ptr })
    }

    pub fn outputs(
        &self,
        flake_settings: &FlakeSettings,
        eval_state: &mut EvalState,
    ) -> Result<nix_expr::value::Value> {
        let mut ctx = Context::new();
        unsafe {
            let r = context::check_call!(raw::locked_flake_get_output_attrs(
                &mut ctx,
                flake_settings.ptr,
                eval_state.raw_ptr(),
                self.ptr.as_ptr()
            ))?;
            Ok(nix_expr::value::__private::raw_value_new(r))
        }
    }
}

#[cfg(test)]
mod tests {
    use nix_expr::eval_state::{gc_register_my_thread, EvalStateBuilder};
    use nix_store::store::Store;

    use super::*;

    fn init() {
        nix_util::settings::set("experimental-features", "flakes").unwrap();
    }

    #[test]
    fn flake_settings_getflake_exists() {
        init();
        let gc_registration = gc_register_my_thread();
        let store = Store::open(None, []).unwrap();
        let mut eval_state = EvalStateBuilder::new(store)
            .unwrap()
            .flakes(&FlakeSettings::new().unwrap())
            .unwrap()
            .build()
            .unwrap();

        let v = eval_state
            .eval_from_string("builtins?getFlake", "<test>")
            .unwrap();

        let b = eval_state.require_bool(&v).unwrap();

        assert_eq!(b, true);

        drop(gc_registration);
    }

    #[test]
    fn flake_lock_load_flake() {
        init();
        let gc_registration = gc_register_my_thread();
        let store = Store::open(None, []).unwrap();
        let flake_settings = FlakeSettings::new().unwrap();
        let mut eval_state = EvalStateBuilder::new(store)
            .unwrap()
            .flakes(&flake_settings)
            .unwrap()
            .build()
            .unwrap();

        let tmp_dir = tempfile::tempdir().unwrap();

        // Create flake.nix
        let flake_nix = tmp_dir.path().join("flake.nix");
        std::fs::write(
            &flake_nix,
            r#"
{
    outputs = { ... }: {
        hello = "potato";
    };
}
        "#,
        )
        .unwrap();

        let flake_lock_flags = FlakeLockFlags::new(&flake_settings).unwrap();

        let (flake_ref, fragment) = FlakeReference::parse_with_fragment(
            &FetchersSettings::new().unwrap(),
            &flake_settings,
            &FlakeReferenceParseFlags::new(&flake_settings).unwrap(),
            &format!("path:{}#subthing", tmp_dir.path().display()),
        )
        .unwrap();

        assert_eq!(fragment, "subthing");

        let locked_flake = LockedFlake::lock(
            &FetchersSettings::new().unwrap(),
            &flake_settings,
            &eval_state,
            &flake_lock_flags,
            &flake_ref,
        )
        .unwrap();

        let outputs = locked_flake
            .outputs(&flake_settings, &mut eval_state)
            .unwrap();

        let hello = eval_state.require_attrs_select(&outputs, &"hello").unwrap();
        let hello = eval_state.require_string(&hello).unwrap();

        assert_eq!(hello, "potato");

        drop(tmp_dir);
        drop(gc_registration);
    }
}
