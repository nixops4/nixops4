use crate::value::{Int, Value, ValueType};
use anyhow::Context as _;
use anyhow::{bail, Result};
use lazy_static::lazy_static;
use nix_c_raw as raw;
use nix_store::path::StorePath;
use nix_store::store::Store;
use nix_util::context::Context;
use nix_util::result_string_init;
use nix_util::string_return::{callback_get_result_string, callback_get_result_string_data};
use std::ffi::{c_char, CString};
use std::os::raw::c_uint;
use std::ptr::{null, null_mut, NonNull};

lazy_static! {
    static ref INIT: Result<()> = {
        unsafe {
            raw::GC_allow_register_threads();
        }
        let context: Context = Context::new();
        unsafe {
            raw::libexpr_init(context.ptr());
        }
        context.check_err()?;
        Ok(())
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
        let context = Context::new();

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

        let eval_state =
            unsafe { raw::state_create(context.ptr(), lookup_path.as_mut_ptr(), store.raw_ptr()) };
        context.check_err()?;
        if eval_state.is_null() {
            panic!("nix_state_create returned a null pointer without an error");
        }
        Ok(EvalState {
            eval_state: NonNull::new(eval_state).unwrap(),
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
    /// # let es = EvalState::new(Store::open("auto", [])?, [])?;
    /// let v: Value = es.eval_from_string("42", ".")?;
    /// assert_eq!(es.require_int(&v)?, 42);
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias = "nix_expr_eval_from_string")]
    pub fn eval_from_string(&self, expr: &str, path: &str) -> Result<Value> {
        let expr_ptr =
            CString::new(expr).with_context(|| "eval_from_string: expr contains null byte")?;
        let path_ptr =
            CString::new(path).with_context(|| "eval_from_string: path contains null byte")?;
        let value = self.new_value_uninitialized();
        unsafe {
            let ctx_ptr = self.context.ptr();
            raw::expr_eval_from_string(
                ctx_ptr,
                self.raw_ptr(),
                expr_ptr.as_ptr(),
                path_ptr.as_ptr(),
                value.raw_ptr(),
            );
        };
        self.context.check_err()?;
        Ok(value)
    }
    /// Try turn any Value into a Value that isn't a Thunk.
    pub fn force(&self, v: &Value) -> Result<()> {
        unsafe {
            raw::value_force(self.context.ptr(), self.raw_ptr(), v.raw_ptr());
        }
        self.context.check_err()
    }
    pub fn value_is_thunk(&self, value: &Value) -> bool {
        let r = unsafe {
            raw::get_type(self.context.ptr(), value.raw_ptr()) == raw::ValueType_NIX_TYPE_THUNK
        };
        self.context.check_err().unwrap();
        r
    }
    pub fn value_type(&self, value: &Value) -> Result<ValueType> {
        if self.value_is_thunk(value) {
            self.force(value)?;
        }
        let r = unsafe { raw::get_type(self.context.ptr(), value.raw_ptr()) };
        Ok(ValueType::from_raw(r))
    }
    pub fn require_int(&self, v: &Value) -> Result<Int> {
        let t = self.value_type(v).unwrap();
        if t != ValueType::Int {
            bail!("expected an int, but got a {:?}", t);
        }
        let i = unsafe { raw::get_int(self.context.ptr(), v.raw_ptr()) };
        Ok(i)
    }
    /// Evaluate, and require that the value is an attrset.
    /// Returns a list of the keys in the attrset.
    pub fn require_attrs_names(&self, v: &Value) -> Result<Vec<String>> {
        let t = self.value_type(v)?;
        if t != ValueType::AttrSet {
            bail!("expected an attrset, but got a {:?}", t);
        }
        let n = unsafe { raw::get_attrs_size(self.context.ptr(), v.raw_ptr()) as usize };
        self.context.check_err()?;
        let mut attrs = Vec::with_capacity(n);
        unsafe {
            for i in 0..n {
                let cstr_ptr: *const i8 = raw::get_attr_name_byidx(
                    self.context.ptr(),
                    v.raw_ptr(),
                    self.raw_ptr(),
                    i as c_uint,
                );
                self.context.check_err()?;
                let cstr = std::ffi::CStr::from_ptr(cstr_ptr);
                let s = cstr.to_str().map_err(|e| {
                    anyhow::format_err!("Nix attrset key is not valid UTF-8: {}", e)
                })?;
                attrs.insert(i, s.to_owned());
            }
        }
        Ok(attrs)
    }

    pub fn require_attrs_select(&self, v: &Value, attr_name: &str) -> Result<Value> {
        let r = self.require_attrs_select_opt(v, attr_name)?;
        match r {
            Some(v) => Ok(v),
            None => self.context.check_err().and_then(|_| {
                // should be unreachable
                bail!("attribute not found: {}", attr_name)
            }),
        }
    }

    /// Evaluate, require that the value is an attrset, and select an attribute by name.
    pub fn require_attrs_select_opt(&self, v: &Value, attr_name: &str) -> Result<Option<Value>> {
        let t = self.value_type(v)?;
        if t != ValueType::AttrSet {
            bail!("expected an attrset, but got a {:?}", t);
        }
        let attr_name = CString::new(attr_name)
            .with_context(|| "require_attrs_select_opt: attrName contains null byte")?;
        // c_void should be Value; why was void generated?
        let v = unsafe {
            raw::get_attr_byname(
                self.context.ptr(),
                v.raw_ptr(),
                self.raw_ptr(),
                attr_name.as_ptr(),
            )
        };
        if self.context.is_key_error() {
            Ok(None)
        } else {
            self.context.check_err()?;
            Ok(Some(Value::new(v)))
        }
    }

    /// Create a new value containing the passed string.
    /// Returns a string value without any string context.
    pub fn new_value_str(&self, s: &str) -> Result<Value> {
        let s = CString::new(s).with_context(|| "new_value_str: contains null byte")?;
        let v = unsafe {
            let value = self.new_value_uninitialized();
            raw::init_string(self.context.ptr(), value.raw_ptr(), s.as_ptr());
            value
        };
        self.context.check_err()?;
        Ok(v)
    }

    pub fn new_value_int(&self, i: Int) -> Result<Value> {
        let v = unsafe {
            let value = self.new_value_uninitialized();
            raw::init_int(self.context.ptr(), value.raw_ptr(), i);
            value
        };
        self.context.check_err()?;
        Ok(v)
    }

    /// Not exposed, because the caller must always explicitly handle the context or not accept one at all.
    fn get_string(&self, value: &Value) -> Result<String> {
        let mut r = result_string_init!();
        unsafe {
            raw::get_string(
                self.context.ptr(),
                value.raw_ptr(),
                Some(callback_get_result_string),
                callback_get_result_string_data(&mut r),
            )
        };
        self.context.check_err()?;
        r
    }
    /// NOTE: this will be replaced by two methods, one that also returns the context, and one that checks that the context is empty
    pub fn require_string(&self, value: &Value) -> Result<String> {
        let t = self.value_type(value)?;
        if t != ValueType::String {
            bail!("expected a string, but got a {:?}", t);
        }
        self.get_string(value)
    }
    pub fn realise_string(
        &self,
        value: &Value,
        is_import_from_derivation: bool,
    ) -> Result<RealisedString> {
        let t = self.value_type(value)?;
        if t != ValueType::String {
            bail!("expected a string, but got a {:?}", t);
        }

        let rs = unsafe {
            raw::string_realise(
                self.context.ptr(),
                self.raw_ptr(),
                value.raw_ptr(),
                is_import_from_derivation,
            )
        };
        self.context.check_err()?;

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
    pub fn call(&self, f: Value, a: Value) -> Result<Value> {
        let v = unsafe {
            let value = self.new_value_uninitialized();
            raw::value_call(
                self.context.ptr(),
                self.raw_ptr(),
                f.raw_ptr(),
                a.raw_ptr(),
                value.raw_ptr(),
            );
            value
        };
        self.context.check_err()?;
        Ok(v)
    }

    /// Apply a function to an argument, but don't evaluate the result just yet.
    ///
    /// For an eager version, see [`call`][`EvalState::call`].
    pub fn new_value_apply(&self, f: &Value, a: &Value) -> Result<Value> {
        let value = self.new_value_uninitialized();
        unsafe {
            raw::init_apply(
                self.context.ptr(),
                value.raw_ptr(),
                f.raw_ptr(),
                a.raw_ptr(),
            );
        };
        self.context.check_err()?;
        Ok(value)
    }

    fn new_value_uninitialized(&self) -> Value {
        let value = unsafe { raw::alloc_value(self.context.ptr(), self.raw_ptr()) };
        Value::new(value)
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
            let es = EvalState::new(Store::open("auto", HashMap::new()).unwrap(), []).unwrap();
            assert!(es.eval_from_string(import_expression, "<test>").is_err());

            let es = EvalState::new(
                Store::open("auto", HashMap::new()).unwrap(),
                [
                    format!("test_file0={}", test_file0.path().to_str().unwrap()).as_str(),
                    format!("test_file1={}", test_file1.path().to_str().unwrap()).as_str(),
                ],
            )
            .unwrap();
            let v = es
                .require_int(&es.eval_from_string(import_expression, "<test>").unwrap())
                .unwrap();
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
            let es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("1", "<test>").unwrap();
            let v2 = v.clone();
            es.force(&v).unwrap();
            let t = es.value_type(&v).unwrap();
            assert!(t == ValueType::Int);
            let t2 = es.value_type(&v2).unwrap();
            assert!(t2 == ValueType::Int);
            gc_now();
        })
        .unwrap();
    }

    #[test]
    fn eval_state_value_bool() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("true", "<test>").unwrap();
            es.force(&v).unwrap();
            let t = es.value_type(&v).unwrap();
            assert!(t == ValueType::Bool);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_value_int() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let es = EvalState::new(store, []).unwrap();
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
    fn eval_state_value_attrs_names_empty() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("{ }", "<test>").unwrap();
            es.force(&v).unwrap();
            let t = es.value_type(&v).unwrap();
            assert!(t == ValueType::AttrSet);
            let attrs = es.require_attrs_names(&v).unwrap();
            assert_eq!(attrs.len(), 0);
        })
        .unwrap()
    }

    #[test]
    fn eval_state_require_attrs_names_bad_type() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let es = EvalState::new(store, []).unwrap();
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
            let es = EvalState::new(store, []).unwrap();
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
            let es = EvalState::new(store, []).unwrap();
            let expr = r#"{ a = "aye"; b = "bee"; }"#;
            let v = es.eval_from_string(expr, "<test>").unwrap();
            let a = es.require_attrs_select(&v, "a").unwrap();
            let b = es.require_attrs_select(&v, "b").unwrap();
            assert_eq!(es.require_string(&a).unwrap(), "aye");
            assert_eq!(es.require_string(&b).unwrap(), "bee");
        })
        .unwrap()
    }

    #[test]
    fn eval_state_require_attrs_select_error() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let es = EvalState::new(store, []).unwrap();
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
            let es = EvalState::new(store, []).unwrap();
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
    fn eval_state_require_attrs_select_opt_error() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let es = EvalState::new(store, []).unwrap();
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
            let es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("\"hello\"", "<test>").unwrap();
            es.force(&v).unwrap();
            let t = es.value_type(&v).unwrap();
            assert!(t == ValueType::String);
            let s = es.require_string(&v).unwrap();
            assert!(s == "hello");
        })
        .unwrap();
    }

    #[test]
    fn eval_state_value_string_unexpected_bool() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let es = EvalState::new(store, []).unwrap();
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
            let es = EvalState::new(store, []).unwrap();
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
            let es = EvalState::new(store, []).unwrap();
            let v = es
                .eval_from_string("builtins.substring 0 1 \"ü\"", "<test>")
                .unwrap();
            es.force(&v).unwrap();
            let t = es.value_type(&v).unwrap();
            assert!(t == ValueType::String);
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
            let es = EvalState::new(store, []).unwrap();
            let v = es
                .eval_from_string("(derivation { name = \"hello\"; system = \"dummy\"; builder = \"cmd.exe\"; }).outPath", "<test>")
                .unwrap();
            es.force(&v).unwrap();
            let t = es.value_type(&v).unwrap();
            assert!(t == ValueType::String);
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
            let es = EvalState::new(store, []).unwrap();
            let v = es.new_value_str("hello").unwrap();
            es.force(&v).unwrap();
            let t = es.value_type(&v).unwrap();
            assert!(t == ValueType::String);
            let s = es.require_string(&v).unwrap();
            assert!(s == "hello");
        })
        .unwrap();
    }

    #[test]
    fn eval_state_new_string_empty() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let es = EvalState::new(store, []).unwrap();
            let v = es.new_value_str("").unwrap();
            es.force(&v).unwrap();
            let t = es.value_type(&v).unwrap();
            assert!(t == ValueType::String);
            let s = es.require_string(&v).unwrap();
            assert!(s == "");
        })
        .unwrap();
    }

    #[test]
    fn eval_state_new_string_invalid() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let es = EvalState::new(store, []).unwrap();
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
            let es = EvalState::new(store, []).unwrap();
            let v = es.new_value_int(42).unwrap();
            es.force(&v).unwrap();
            let t = es.value_type(&v).unwrap();
            assert!(t == ValueType::Int);
            let i = es.require_int(&v).unwrap();
            assert!(i == 42);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_value_attrset() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("{ }", "<test>").unwrap();
            es.force(&v).unwrap();
            let t = es.value_type(&v).unwrap();
            assert!(t == ValueType::AttrSet);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_value_list() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("[ ]", "<test>").unwrap();
            es.force(&v).unwrap();
            let t = es.value_type(&v).unwrap();
            assert!(t == ValueType::List);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_realise_string() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let es = EvalState::new(store, []).unwrap();
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
            let es = EvalState::new(store, []).unwrap();
            let f = es.eval_from_string("x: x + 1", "<test>").unwrap();
            let a = es.eval_from_string("2", "<test>").unwrap();
            let v = es.call(f, a).unwrap();
            es.force(&v).unwrap();
            let t = es.value_type(&v).unwrap();
            assert!(t == ValueType::Int);
            let i = es.require_int(&v).unwrap();
            assert!(i == 3);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_apply() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let es = EvalState::new(store, []).unwrap();
            // This is a function that takes two arguments.
            let f = es.eval_from_string("x: x + 1", "<test>").unwrap();
            let a = es.eval_from_string("2", "<test>").unwrap();
            let v = es.new_value_apply(&f, &a).unwrap();
            assert!(es.value_is_thunk(&v));
            es.force(&v).unwrap();
            let t = es.value_type(&v).unwrap();
            assert!(t == ValueType::Int);
            let i = es.require_int(&v).unwrap();
            assert!(i == 3);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_call_fail_body() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let es = EvalState::new(store, []).unwrap();
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
            let es = EvalState::new(store, []).unwrap();
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

    #[test]
    fn eval_state_call_fail_args() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let es = EvalState::new(store, []).unwrap();
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

    #[test]
    fn eval_state_apply_fail_args() {
        gc_registering_current_thread(|| {
            let store = Store::open("auto", HashMap::new()).unwrap();
            let es = EvalState::new(store, []).unwrap();
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

            let es = EvalState::new(
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
