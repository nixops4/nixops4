use crate::value::{Int, Value, ValueType};
use anyhow::Context as _;
use anyhow::{bail, Result};
use lazy_static::lazy_static;
use nix_c_raw as raw;
use nix_store::path::StorePath;
use nix_store::store::Store;
use nix_util::context::Context;
use nix_util::string_return::{callback_get_result_string, callback_get_result_string_data};
use nix_util::{check_call, check_call_opt_key, result_string_init};
use std::ffi::{c_char, CString};
use std::os::raw::c_uint;
use std::ptr::{null, null_mut, NonNull};

lazy_static! {
    static ref INIT: Result<()> = {
        unsafe {
            raw::GC_allow_register_threads();
            check_call!(raw::libexpr_init(&mut Context::new()))?;
            Ok(())
        }
    };
}
pub fn init() -> Result<()> {
    let x = INIT.as_ref();
    match x {
        Ok(_) => Ok(()),
        Err(e) => {
            // Couldn't just clone the error, so we have to print it here.
            Err(anyhow::format_err!("nix_libstore_init error: {}", e))
        }
    }
}

pub struct RealisedString {
    pub s: String,
    pub paths: Vec<StorePath>,
}

pub struct EvalState {
    eval_state: NonNull<raw::EvalState>,
    store: Store,
    context: Context,
}
impl EvalState {
    pub fn new<'a>(store: Store, lookup_path: impl IntoIterator<Item = &'a str>) -> Result<Self> {
        let mut context = Context::new();

        // this intermediate value must be here and must not be moved
        // because it owns the data the `*const c_char` pointers point to.
        let lookup_path: Vec<CString> = lookup_path
            .into_iter()
            .map(|path| {
                CString::new(path).with_context(|| {
                    format!("EvalState::new: lookup_path `{path}` contains null byte")
                })
            })
            .collect::<Result<_>>()?;

        // this intermediate value owns the data the `*mut *const c_char` pointer points to.
        let mut lookup_path: Vec<*const c_char> = lookup_path
            .iter()
            .map(|s| s.as_ptr())
            .chain(std::iter::once(null())) // signal the end of the array
            .collect();

        init()?;

        let eval_state = unsafe {
            check_call!(raw::state_create(
                &mut context,
                lookup_path.as_mut_ptr(),
                store.raw_ptr()
            ))
        }?;
        Ok(EvalState {
            eval_state: NonNull::new(eval_state).unwrap_or_else(|| {
                panic!("nix_state_create returned a null pointer without an error")
            }),
            store,
            context,
        })
    }
    pub fn raw_ptr(&self) -> *mut raw::EvalState {
        self.eval_state.as_ptr()
    }
    pub fn store(&self) -> &Store {
        &self.store
    }
    /// Parses and evaluates a Nix expression `expr`.
    ///
    /// Expressions can contain relative paths such as `./.` that are resolved relative to the given `path`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use nix_expr::eval_state::EvalState;
    /// use nix_store::store::Store;
    /// use nix_expr::value::Value;
    ///
    /// # fn main() -> anyhow::Result<()> {
    /// # let mut es = EvalState::new(Store::open("auto", [])?, [])?;
    /// let v: Value = es.eval_from_string("42", ".")?;
    /// assert_eq!(es.require_int(&v)?, 42);
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias = "nix_expr_eval_from_string")]
    pub fn eval_from_string(&mut self, expr: &str, path: &str) -> Result<Value> {
        let expr_ptr =
            CString::new(expr).with_context(|| "eval_from_string: expr contains null byte")?;
        let path_ptr =
            CString::new(path).with_context(|| "eval_from_string: path contains null byte")?;
        unsafe {
            let value = self.new_value_uninitialized()?;
            check_call!(raw::expr_eval_from_string(
                &mut self.context,
                self.eval_state.as_ptr(),
                expr_ptr.as_ptr(),
                path_ptr.as_ptr(),
                value.raw_ptr()
            ))?;
            Ok(value)
        }
    }
    /// Try turn any Value into a Value that isn't a Thunk.
    pub fn force(&mut self, v: &Value) -> Result<()> {
        unsafe {
            check_call!(raw::value_force(
                &mut self.context,
                self.eval_state.as_ptr(),
                v.raw_ptr()
            ))
        }?;
        Ok(())
    }
    pub fn value_type_unforced(&mut self, value: &Value) -> Option<ValueType> {
        let r = unsafe { check_call!(raw::get_type(&mut self.context, value.raw_ptr())) };
        // .unwrap(): no reason for this to fail, as it does not evaluate
        ValueType::from_raw(r.unwrap())
    }
    pub fn value_type(&mut self, value: &Value) -> Result<ValueType> {
        match self.value_type_unforced(value) {
            Some(a) => Ok(a),
            None => {
                self.force(value)?;
                match self.value_type_unforced(value) {
                    Some(a) => Ok(a),
                    None => {
                        panic!("Nix value must not be thunk after being forced.")
                    }
                }
            }
        }
    }
    pub fn require_int(&mut self, v: &Value) -> Result<Int> {
        let t = self.value_type(v)?;
        if t != ValueType::Int {
            bail!("expected an int, but got a {:?}", t);
        }
        unsafe { check_call!(raw::get_int(&mut self.context, v.raw_ptr())) }
    }
    /// Evaluate, and require that the value is an attrset.
    /// Returns a list of the keys in the attrset.
    pub fn require_attrs_names(&mut self, v: &Value) -> Result<Vec<String>> {
        let t = self.value_type(v)?;
        if t != ValueType::AttrSet {
            bail!("expected an attrset, but got a {:?}", t);
        }
        let n = unsafe { check_call!(raw::get_attrs_size(&mut self.context, v.raw_ptr())) }?;
        let mut attrs = Vec::with_capacity(n as usize);
        for i in 0..n {
            let cstr_ptr: *const i8 = unsafe {
                check_call!(raw::get_attr_name_byidx(
                    &mut self.context,
                    v.raw_ptr(),
                    self.eval_state.as_ptr(),
                    i as c_uint
                ))
            }?;
            let cstr = unsafe { std::ffi::CStr::from_ptr(cstr_ptr) };
            let s = cstr
                .to_str()
                .map_err(|e| anyhow::format_err!("Nix attrset key is not valid UTF-8: {}", e))?;
            attrs.insert(i as usize, s.to_owned());
        }
        Ok(attrs)
    }

    /// Evaluate, require that the value is an attrset, and select an attribute by name.
    pub fn require_attrs_select(&mut self, v: &Value, attr_name: &str) -> Result<Value> {
        let t = self.value_type(v)?;
        if t != ValueType::AttrSet {
            bail!("expected an attrset, but got a {:?}", t);
        }
        let attr_name = CString::new(attr_name)
            .with_context(|| "require_attrs_select: attrName contains null byte")?;
        let v2 = unsafe {
            check_call!(raw::get_attr_byname(
                &mut self.context,
                v.raw_ptr(),
                self.eval_state.as_ptr(),
                attr_name.as_ptr()
            ))
        }?;
        Ok(Value::new(v2))
    }

    /// Evaluate, require that the value is an attrset, and select an attribute by name.
    ///
    /// Return `Err(...)` if `v` is not an attrset, or if some other error occurred.
    ///
    /// Return `Ok(None)` if the attribute is not present.
    ///
    /// Return `Ok(Some(value))` if the attribute is present.
    pub fn require_attrs_select_opt(
        &mut self,
        v: &Value,
        attr_name: &str,
    ) -> Result<Option<Value>> {
        let t = self.value_type(v)?;
        if t != ValueType::AttrSet {
            bail!("expected an attrset, but got a {:?}", t);
        }
        let attr_name = CString::new(attr_name)
            .with_context(|| "require_attrs_select_opt: attrName contains null byte")?;
        let v2 = unsafe {
            check_call_opt_key!(raw::get_attr_byname(
                &mut self.context,
                v.raw_ptr(),
                self.eval_state.as_ptr(),
                attr_name.as_ptr()
            ))
        }?;
        Ok(v2.map(Value::new))
    }

    /// Create a new value containing the passed string.
    /// Returns a string value without any string context.
    pub fn new_value_str(&mut self, s: &str) -> Result<Value> {
        let s = CString::new(s).with_context(|| "new_value_str: contains null byte")?;
        let v = unsafe {
            let value = self.new_value_uninitialized()?;
            check_call!(raw::init_string(
                &mut self.context,
                value.raw_ptr(),
                s.as_ptr()
            ))?;
            value
        };
        Ok(v)
    }

    pub fn new_value_int(&mut self, i: Int) -> Result<Value> {
        let v = unsafe {
            let value = self.new_value_uninitialized()?;
            check_call!(raw::init_int(&mut self.context, value.raw_ptr(), i))?;
            value
        };
        Ok(v)
    }

    /// Not exposed, because the caller must always explicitly handle the context or not accept one at all.
    fn get_string(&mut self, value: &Value) -> Result<String> {
        let mut r = result_string_init!();
        unsafe {
            check_call!(raw::get_string(
                &mut self.context,
                value.raw_ptr(),
                Some(callback_get_result_string),
                callback_get_result_string_data(&mut r)
            ))?;
        };
        r
    }
    /// NOTE: this will be replaced by two methods, one that also returns the context, and one that checks that the context is empty
    pub fn require_string(&mut self, value: &Value) -> Result<String> {
        let t = self.value_type(value)?;
        if t != ValueType::String {
            bail!("expected a string, but got a {:?}", t);
        }
        self.get_string(value)
    }
    pub fn realise_string(
        &mut self,
        value: &Value,
        is_import_from_derivation: bool,
    ) -> Result<RealisedString> {
        let t = self.value_type(value)?;
        if t != ValueType::String {
            bail!("expected a string, but got a {:?}", t);
        }

        let rs = unsafe {
            check_call!(raw::string_realise(
                &mut self.context,
                self.eval_state.as_ptr(),
                value.raw_ptr(),
                is_import_from_derivation
            ))
        }?;

        let s = unsafe {
            let start = raw::realised_string_get_buffer_start(rs) as *const u8;
            let size = raw::realised_string_get_buffer_size(rs);
            let slice = std::slice::from_raw_parts(start, size);
            String::from_utf8(slice.to_vec())
                .map_err(|e| anyhow::format_err!("Nix string is not valid UTF-8: {}", e))?
        };

        let paths = unsafe {
            let n = raw::realised_string_get_store_path_count(rs);
            let mut paths = Vec::with_capacity(n as usize);
            for i in 0..n {
                let path = raw::realised_string_get_store_path(rs, i);
                paths.push(StorePath::new_raw_clone(path));
            }
            paths
        };

        // We've converted the nix_realised_string to a native struct containing copies, so we can free it now.
        unsafe {
            raw::realised_string_free(rs);
        }

        Ok(RealisedString { s, paths })
    }

    /// Eagerly apply a function to an argument.
    ///
    /// For a lazy version, see [`new_value_apply`][`EvalState::new_value_apply`].
    pub fn call(&mut self, f: Value, a: Value) -> Result<Value> {
        let value = self.new_value_uninitialized()?;
        unsafe {
            check_call!(raw::value_call(
                &mut self.context,
                self.eval_state.as_ptr(),
                f.raw_ptr(),
                a.raw_ptr(),
                value.raw_ptr()
            ))
        }?;
        Ok(value)
    }

    /// Apply a function to an argument, but don't evaluate the result just yet.
    ///
    /// For an eager version, see [`call`][`EvalState::call`].
    pub fn new_value_apply(&mut self, f: &Value, a: &Value) -> Result<Value> {
        let value = self.new_value_uninitialized()?;
        unsafe {
            check_call!(raw::init_apply(
                &mut self.context,
                value.raw_ptr(),
                f.raw_ptr(),
                a.raw_ptr()
            ))
        }?;
        Ok(value)
    }

    fn new_value_uninitialized(&mut self) -> Result<Value> {
        let value = unsafe {
            check_call!(raw::alloc_value(
                &mut self.context,
                self.eval_state.as_ptr()
            ))
        }?;
        Ok(Value::new(value))
    }
}

pub fn gc_now() {
    unsafe {
        raw::gc_now();
    }
}

/// Run a function while making sure that the current thread is registered with the GC.
pub fn gc_registering_current_thread<F, R>(f: F) -> Result<R>
where
    F: FnOnce() -> R,
{
    init()?;
    if unsafe { raw::GC_thread_is_registered() } != 0 {
        return Ok(f());
    } else {
        gc_register_my_thread()?;
        let r = f();
        unsafe {
            raw::GC_unregister_my_thread();
        }
        return Ok(r);
    }
}

pub fn gc_register_my_thread() -> Result<()> {
    unsafe {
        let already_done = raw::GC_thread_is_registered();
        if already_done != 0 {
            return Ok(());
        }
        let mut sb: raw::GC_stack_base = raw::GC_stack_base {
            mem_base: null_mut(),
        };
        let r = raw::GC_get_stack_base(&mut sb);
        if r as u32 != raw::GC_SUCCESS {
            Err(anyhow::format_err!("GC_get_stack_base failed: {}", r))?;
        }
        raw::GC_register_my_thread(&sb);
        Ok(())
    }
}

impl Drop for EvalState {
    fn drop(&mut self) {
        unsafe {
            raw::state_free(self.raw_ptr());
        }
    }
}

/// Initialize the Nix library for testing. This includes some modifications to the Nix settings, that must not be used in production.
/// Use at your own peril, in rust test suites.
pub fn test_init() {
    init().unwrap();

    // During development, we encountered a problem where the build hook
    // would cause the test suite to reinvokes itself, causing an infinite loop.
    // While _NIX_TEST_NO_SANDBOX=1 should prevent this, we may also set the
    // build hook to "" to prevent this.
    // settings::set("build-hook", "")?;

    // When testing in the sandbox, the default build dir would be a parent of the storeDir,
    // which causes an error. So we set a custom build dir here.
    nix_util::settings::set("sandbox-build-dir", "/custom-build-dir-for-test").unwrap();
    std::env::set_var("_NIX_TEST_NO_SANDBOX", "1");
}

#[cfg(test)]
mod tests {
    use super::*;
    use ctor::ctor;
    use std::collections::HashMap;
    use std::fs::read_dir;
    use std::io::Write as _;

    #[ctor]
    fn setup() {
        test_init();
    }

    #[test]
    fn eval_state_new_and_drop() {
        gc_registering_current_thread(|| {
            // very basic test: make sure initialization doesn't crash
            let store = Store::open("auto", HashMap::new()).unwrap();
            let _e = EvalState::new(store, []).unwrap();
        })
        .unwrap();
    }

    #[test]
    fn eval_state_lookup_path() {
        let import_expression = "import <test_file0> + import <test_file1>";
        let integer0 = 83;
        let integer1 = 103;
        let mut test_file0 = tempfile::NamedTempFile::new().unwrap();
        let mut test_file1 = tempfile::NamedTempFile::new().unwrap();
        writeln!(test_file0, "{integer0}").unwrap();
        writeln!(test_file1, "{integer1}").unwrap();
        gc_registering_current_thread(|| {
            let mut es = EvalState::new(Store::open("auto", HashMap::new()).unwrap(), []).unwrap();
            assert!(es.eval_from_string(import_expression, "<test>").is_err());

            let mut es = EvalState::new(
                Store::open("auto", HashMap::new()).unwrap(),
                [
                    format!("test_file0={}", test_file0.path().to_str().unwrap()).as_str(),
                    format!("test_file1={}", test_file1.path().to_str().unwrap()).as_str(),
                ],
            )
            .unwrap();
            let ie = &es.eval_from_string(import_expression, "<test>").unwrap();
            let v = es.require_int(ie).unwrap();
            assert_eq!(v, integer0 + integer1);
        })
        .unwrap();
        test_file0.close().unwrap();
        test_file1.close().unwrap();
    }

    #[test]
    fn eval_state_eval_from_string() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("1", "<test>").unwrap();
            let v2 = v.clone();
            es.force(&v).unwrap();
            let t = es.value_type_unforced(&v);
            assert!(t == Some(ValueType::Int));
            let t2 = es.value_type_unforced(&v2);
            assert!(t2 == Some(ValueType::Int));
            gc_now();
        })
        .unwrap();
    }

    #[test]
    fn eval_state_value_bool() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("true", "<test>").unwrap();
            es.force(&v).unwrap();
            let t = es.value_type_unforced(&v);
            assert!(t == Some(ValueType::Bool));
        })
        .unwrap();
    }

    #[test]
    fn eval_state_value_int() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("1", "<test>").unwrap();
            es.force(&v).unwrap();
            let t = es.value_type(&v).unwrap();
            assert!(t == ValueType::Int);
            let i = es.require_int(&v).unwrap();
            assert!(i == 1);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_require_int_forces_thunk() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let f = es.eval_from_string("x: x + 1", "<test>").unwrap();
            let a = es.eval_from_string("2", "<test>").unwrap();
            let v = es.new_value_apply(&f, &a).unwrap();
            let t = es.value_type_unforced(&v);
            assert!(t == None);
            let i = es.require_int(&v).unwrap();
            assert!(i == 3);
        })
        .unwrap();
    }

    /// A helper that turns an expression into a thunk.
    fn make_thunk(es: &mut EvalState, expr: &str) -> Value {
        // This would be silly in real code, but it works for the current Nix implementation.
        // A Nix implementation that applies the identity function eagerly would be a valid
        // Nix implementation, but annoying because we'll have to change this helper to do
        // something more complicated that isn't optimized away.
        let f = es.eval_from_string("x: x", "<test>").unwrap();
        let v = es.eval_from_string(expr, "<test>").unwrap();
        es.new_value_apply(&f, &v).unwrap()
    }

    #[test]
    fn make_thunk_helper_works() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = make_thunk(&mut es, "1");
            let t = es.value_type_unforced(&v);
            assert!(t == None);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_value_attrs_names_empty() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("{ }", "<test>").unwrap();
            es.force(&v).unwrap();
            let t = es.value_type_unforced(&v);
            assert!(t == Some(ValueType::AttrSet));
            let attrs = es.require_attrs_names(&v).unwrap();
            assert_eq!(attrs.len(), 0);
        })
        .unwrap()
    }

    #[test]
    fn eval_state_require_attrs_names_forces_thunk() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = make_thunk(&mut es, "{ a = 1; b = 2; }");
            let t = es.value_type_unforced(&v);
            assert!(t == None);
            let attrs = es.require_attrs_names(&v).unwrap();
            assert_eq!(attrs.len(), 2);
        })
        .unwrap()
    }

    #[test]
    fn eval_state_require_attrs_names_bad_type() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("1", "<test>").unwrap();
            es.force(&v).unwrap();
            let r = es.require_attrs_names(&v);
            assert!(r.is_err());
            assert_eq!(
                r.unwrap_err().to_string(),
                "expected an attrset, but got a Int"
            );
        })
        .unwrap()
    }

    #[test]
    fn eval_state_value_attrs_names_example() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let expr = r#"{ a = throw "nope a"; b = throw "nope b"; }"#;
            let v = es.eval_from_string(expr, "<test>").unwrap();
            let attrs = es.require_attrs_names(&v).unwrap();
            assert_eq!(attrs.len(), 2);
            assert_eq!(attrs[0], "a");
            assert_eq!(attrs[1], "b");
        })
        .unwrap();
    }

    #[test]
    fn eval_state_require_attrs_select() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let expr = r#"{ a = "aye"; b = "bee"; }"#;
            let v = es.eval_from_string(expr, "<test>").unwrap();
            let a = es.require_attrs_select(&v, "a").unwrap();
            let b = es.require_attrs_select(&v, "b").unwrap();
            assert_eq!(es.require_string(&a).unwrap(), "aye");
            assert_eq!(es.require_string(&b).unwrap(), "bee");
            let missing = es.require_attrs_select(&v, "c");
            match missing {
                Ok(_) => panic!("expected an error"),
                Err(e) => {
                    let s = format!("{e:#}");
                    // TODO: bad error message from Nix
                    if !s.contains("missing attribute") {
                        eprintln!("unexpected error message: {}", s);
                        assert!(false);
                    }
                }
            }
        })
        .unwrap()
    }

    #[test]
    fn eval_state_require_attrs_select_forces_thunk() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let expr = r#"{ a = "aye"; b = "bee"; }"#;
            let v = make_thunk(&mut es, expr);
            assert!(es.value_type_unforced(&v).is_none());
            let r = es.require_attrs_select(&v, "a");
            assert!(r.is_ok());
        })
        .unwrap()
    }

    #[test]
    fn eval_state_require_attrs_select_error() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let expr = r#"{ a = throw "oh no the error"; }"#;
            let v = es.eval_from_string(expr, "<test>").unwrap();
            let r = es.require_attrs_select(&v, "a");
            match r {
                Ok(_) => panic!("expected an error"),
                Err(e) => {
                    if !e.to_string().contains("oh no the error") {
                        eprintln!("unexpected error message: {}", e);
                        assert!(false);
                    }
                }
            }
        })
        .unwrap()
    }

    #[test]
    fn eval_state_require_attrs_select_opt() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let expr = r#"{ a = "aye"; b = "bee"; }"#;
            let v = es.eval_from_string(expr, "<test>").unwrap();
            let a = es.require_attrs_select_opt(&v, "a").unwrap().unwrap();
            let b = es.require_attrs_select_opt(&v, "b").unwrap().unwrap();
            assert_eq!(es.require_string(&a).unwrap(), "aye");
            assert_eq!(es.require_string(&b).unwrap(), "bee");
            let c = es.require_attrs_select_opt(&v, "c").unwrap();
            assert!(c.is_none());
        })
        .unwrap()
    }

    #[test]
    fn eval_state_require_attrs_select_opt_forces_thunk() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let expr = r#"{ a = "aye"; b = "bee"; }"#;
            let v = make_thunk(&mut es, expr);
            assert!(es.value_type_unforced(&v).is_none());
            let r = es.require_attrs_select_opt(&v, "a");
            assert!(r.is_ok());
        })
        .unwrap()
    }

    #[test]
    fn eval_state_require_attrs_select_opt_error() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let expr = r#"{ a = throw "oh no the error"; }"#;
            let v = es.eval_from_string(expr, "<test>").unwrap();
            let r = es.require_attrs_select_opt(&v, "a");
            match r {
                Ok(_) => panic!("expected an error"),
                Err(e) => {
                    if !e.to_string().contains("oh no the error") {
                        eprintln!("unexpected error message: {}", e);
                        assert!(false);
                    }
                }
            }
        })
        .unwrap()
    }

    #[test]
    fn eval_state_value_string() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("\"hello\"", "<test>").unwrap();
            es.force(&v).unwrap();
            let t = es.value_type_unforced(&v);
            assert!(t == Some(ValueType::String));
            let s = es.require_string(&v).unwrap();
            assert!(s == "hello");
        })
        .unwrap();
    }

    #[test]
    fn eval_state_value_string_forces_thunk() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = make_thunk(&mut es, "\"hello\"");
            assert!(es.value_type_unforced(&v).is_none());
            let s = es.require_string(&v).unwrap();
            assert!(s == "hello");
        })
        .unwrap();
    }

    #[test]
    fn eval_state_value_string_unexpected_bool() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("true", "<test>").unwrap();
            es.force(&v).unwrap();
            let r = es.require_string(&v);
            assert!(r.is_err());
            // TODO: safe print value (like Nix would)
            assert_eq!(
                r.unwrap_err().to_string(),
                "expected a string, but got a Bool"
            );
        })
        .unwrap()
    }

    #[test]
    fn eval_state_value_string_unexpected_path_value() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("/foo", "<test>").unwrap();
            es.force(&v).unwrap();
            let r = es.require_string(&v);
            assert!(r.is_err());
            assert_eq!(
                r.unwrap_err().to_string(),
                "expected a string, but got a Path"
            );
        })
        .unwrap()
    }

    #[test]
    fn eval_state_value_string_bad_utf() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es
                .eval_from_string("builtins.substring 0 1 \"ü\"", "<test>")
                .unwrap();
            es.force(&v).unwrap();
            let t = es.value_type_unforced(&v);
            assert!(t == Some(ValueType::String));
            let r = es.require_string(&v);
            assert!(r.is_err());
            assert!(r
                .unwrap_err()
                .to_string()
                .contains("Nix string is not valid UTF-8"));
        })
        .unwrap();
    }

    #[test]
    fn eval_state_value_string_unexpected_context() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es
                .eval_from_string("(derivation { name = \"hello\"; system = \"dummy\"; builder = \"cmd.exe\"; }).outPath", "<test>")
                .unwrap();
            es.force(&v).unwrap();
            let t = es.value_type_unforced(&v);
            assert!(t == Some(ValueType::String));
            // TODO
            // let r = es.require_string_without_context(&v);
            // assert!(r.is_err());
            // assert!(r.unwrap_err().to_string().contains("unexpected context"));
        })
        .unwrap();
    }

    #[test]
    fn eval_state_new_string() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.new_value_str("hello").unwrap();
            es.force(&v).unwrap();
            let t = es.value_type_unforced(&v);
            assert!(t == Some(ValueType::String));
            let s = es.require_string(&v).unwrap();
            assert!(s == "hello");
        })
        .unwrap();
    }

    #[test]
    fn eval_state_new_string_empty() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.new_value_str("").unwrap();
            es.force(&v).unwrap();
            let t = es.value_type_unforced(&v);
            assert!(t == Some(ValueType::String));
            let s = es.require_string(&v).unwrap();
            assert!(s == "");
        })
        .unwrap();
    }

    #[test]
    fn eval_state_new_string_invalid() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let r = es.new_value_str("hell\0no");
            match r {
                Ok(_) => panic!("expected an error"),
                Err(e) => {
                    if !e.to_string().contains("contains null byte") {
                        eprintln!("{}", e);
                        assert!(false);
                    }
                }
            }
        })
        .unwrap();
    }

    #[test]
    fn eval_state_new_int() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.new_value_int(42).unwrap();
            es.force(&v).unwrap();
            let t = es.value_type_unforced(&v);
            assert!(t == Some(ValueType::Int));
            let i = es.require_int(&v).unwrap();
            assert!(i == 42);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_value_attrset() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("{ }", "<test>").unwrap();
            es.force(&v).unwrap();
            let t = es.value_type_unforced(&v);
            assert!(t == Some(ValueType::AttrSet));
        })
        .unwrap();
    }

    #[test]
    fn eval_state_value_list() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("[ ]", "<test>").unwrap();
            es.force(&v).unwrap();
            let t = es.value_type_unforced(&v);
            assert!(t == Some(ValueType::List));
        })
        .unwrap();
    }

    #[test]
    fn eval_state_realise_string() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let expr = r#"
                ''
                    a derivation output: ${
                        derivation { name = "letsbuild";
                            system = builtins.currentSystem;
                            builder = "/bin/sh";
                            args = [ "-c" "echo foo > $out" ];
                            }}
                    a path: ${builtins.toFile "just-a-file" "ooh file good"}
                    a derivation path by itself: ${
                        builtins.unsafeDiscardOutputDependency 
                            (derivation {
                                name = "not-actually-built-yet";
                                system = builtins.currentSystem;
                                builder = "/bin/sh";
                                args = [ "-c" "echo foo > $out" ];
                            }).drvPath}
                ''
            "#;
            let v = es.eval_from_string(expr, "<test>").unwrap();
            es.force(&v).unwrap();
            let rs = es.realise_string(&v, false).unwrap();

            assert!(rs.s.starts_with("a derivation output:"));
            assert!(rs.s.contains("-letsbuild\n"));
            assert!(!rs.s.contains("-letsbuild.drv"));
            assert!(rs.s.contains("a path:"));
            assert!(rs.s.contains("-just-a-file"));
            assert!(!rs.s.contains("-just-a-file.drv"));
            assert!(!rs.s.contains("ooh file good"));
            assert!(rs.s.ends_with("-not-actually-built-yet.drv\n"));

            assert_eq!(rs.paths.len(), 3);
            let mut names: Vec<String> = rs.paths.iter().map(|p| p.name().unwrap()).collect();
            names.sort();
            assert_eq!(names[0], "just-a-file");
            assert_eq!(names[1], "letsbuild");
            assert_eq!(names[2], "not-actually-built-yet.drv");
        })
        .unwrap();
    }

    #[test]
    fn eval_state_call() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let f = es.eval_from_string("x: x + 1", "<test>").unwrap();
            let a = es.eval_from_string("2", "<test>").unwrap();
            let v = es.call(f, a).unwrap();
            es.force(&v).unwrap();
            let t = es.value_type_unforced(&v);
            assert!(t == Some(ValueType::Int));
            let i = es.require_int(&v).unwrap();
            assert!(i == 3);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_apply() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            // This is a function that takes two arguments.
            let f = es.eval_from_string("x: x + 1", "<test>").unwrap();
            let a = es.eval_from_string("2", "<test>").unwrap();
            let v = es.new_value_apply(&f, &a).unwrap();
            assert!(es.value_type_unforced(&v) == None);
            es.force(&v).unwrap();
            let t = es.value_type_unforced(&v);
            assert!(t == Some(ValueType::Int));
            let i = es.require_int(&v).unwrap();
            assert!(i == 3);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_call_fail_body() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let f = es.eval_from_string("x: x + 1", "<test>").unwrap();
            let a = es.eval_from_string("true", "<test>").unwrap();
            let r = es.call(f, a);
            match r {
                Ok(_) => panic!("expected an error"),
                Err(e) => {
                    if !e.to_string().contains("cannot coerce") {
                        eprintln!("{}", e);
                        assert!(false);
                    }
                }
            }
        })
        .unwrap();
    }

    #[test]
    fn eval_state_apply_fail_body() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let f = es.eval_from_string("x: x + 1", "<test>").unwrap();
            let a = es.eval_from_string("true", "<test>").unwrap();
            // Lazy => no error
            let r = es.new_value_apply(&f, &a).unwrap();
            // Force it => error
            let res = es.force(&r);
            match res {
                Ok(_) => panic!("expected an error"),
                Err(e) => {
                    if !e.to_string().contains("cannot coerce") {
                        eprintln!("{}", e);
                        assert!(false);
                    }
                }
            }
        })
        .unwrap();
    }

    /// This tests the behavior of `call`, which is strict, unlike `new_value_apply`.
    #[test]
    fn eval_state_call_fail_args() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let f = es.eval_from_string("{x}: x + 1", "<test>").unwrap();
            let a = es.eval_from_string("{}", "<test>").unwrap();
            let r = es.call(f, a);
            match r {
                Ok(_) => panic!("expected an error"),
                Err(e) => {
                    if !e.to_string().contains("called without required argument") {
                        eprintln!("{}", e);
                        assert!(false);
                    }
                }
            }
        })
        .unwrap();
    }

    /// This tests the behavior of `new_value_apply`, which is lazy, unlike `call`.
    #[test]
    fn eval_state_apply_fail_args_lazy() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let f = es.eval_from_string("{x}: x + 1", "<test>").unwrap();
            let a = es.eval_from_string("{}", "<test>").unwrap();
            // Lazy => no error
            let r = es.new_value_apply(&f, &a).unwrap();
            // Force it => error
            let res = es.force(&r);
            match res {
                Ok(_) => panic!("expected an error"),
                Err(e) => {
                    if !e.to_string().contains("called without required argument") {
                        eprintln!("{}", e);
                        assert!(false);
                    }
                }
            }
        })
        .unwrap();
    }

    #[test]
    fn store_open_params() {
        gc_registering_current_thread(|| {
            let store = tempfile::tempdir().unwrap();
            let store_path = store.path().to_str().unwrap();
            let state = tempfile::tempdir().unwrap();
            let state_path = state.path().to_str().unwrap();
            let log = tempfile::tempdir().unwrap();
            let log_path = log.path().to_str().unwrap();

            let mut es = EvalState::new(
                Store::open(
                    "local",
                    HashMap::from([
                        ("store", store_path),
                        ("state", state_path),
                        ("log", log_path),
                    ])
                    .iter()
                    .map(|(a, b)| (*a, *b)),
                )
                .unwrap(),
                [],
            )
            .unwrap();

            let expr = r#"
                ''
                    a derivation output: ${
                        derivation { name = "letsbuild";
                            system = builtins.currentSystem;
                            builder = "/bin/sh";
                            args = [ "-c" "echo foo > $out" ];
                            }}
                    a path: ${builtins.toFile "just-a-file" "ooh file good"}
                    a derivation path by itself: ${
                        builtins.unsafeDiscardOutputDependency
                            (derivation {
                                name = "not-actually-built-yet";
                                system = builtins.currentSystem;
                                builder = "/bin/sh";
                                args = [ "-c" "echo foo > $out" ];
                            }).drvPath}
                ''
            "#;
            let derivations: [&[u8]; 3] = [
                b"letsbuild.drv",
                b"just-a-file",
                b"not-actually-built-yet.drv",
            ];
            let _ = es.eval_from_string(expr, "<test>").unwrap();

            // assert that all three `derivations` are inside the store and the `state` directory is not empty either.
            let store_contents: Vec<_> = read_dir(store.path())
                .unwrap()
                .map(|dir_entry| dir_entry.unwrap().file_name())
                .collect();
            for derivation in derivations {
                assert!(store_contents
                    .iter()
                    .find(|f| f.as_encoded_bytes().ends_with(derivation))
                    .is_some());
            }
            assert!(!empty(read_dir(state.path()).unwrap()));

            store.close().unwrap();
            state.close().unwrap();
            log.close().unwrap();
        })
        .unwrap();
    }
    fn empty(foldable: impl IntoIterator) -> bool {
        foldable.into_iter().all(|_| false)
    }
}
