//! # Nix Expression Evaluation
//!
//! This module provides the core [`EvalState`] type for evaluating Nix expressions
//! and extracting typed values from the results.
//!
//! ## Overview
//!
//! The [`EvalState`] manages the evaluation context for Nix expressions, including:
//! - Expression parsing and evaluation with [`eval_from_string`](EvalState::eval_from_string)
//! - Type-safe value extraction with [`require_*`](EvalState#implementations) methods
//! - Memory management and garbage collection integration
//! - Store integration for derivations and store paths
//! - Custom function creation with [`new_value_primop`](EvalState::new_value_primop) and [`new_value_thunk`](EvalState::new_value_thunk)
//!
//! ### Construction
//!
//! Create an [`EvalState`] using [`EvalState::new`] or [`EvalStateBuilder`] for advanced configuration:
//!
//! ```rust
//! # use nix_expr::eval_state::{EvalState, EvalStateBuilder, test_init, gc_register_my_thread};
//! # use nix_store::store::Store;
//! # use std::collections::HashMap;
//! # fn example() -> anyhow::Result<()> {
//! # test_init(); let guard = gc_register_my_thread()?;
//! let store = Store::open(None, HashMap::new())?;
//!
//! // Simple creation
//! let mut es = EvalState::new(store.clone(), [])?;
//!
//! // With custom lookup paths
//! let mut es = EvalStateBuilder::new(store)?
//!     .lookup_path(["nixpkgs=/path/to/nixpkgs"])?
//!     .build()?;
//! # drop(guard);
//! # Ok(())
//! # }
//! ```
//!
//! ## Value Extraction
//!
//! All `require_*` methods perform these steps:
//! 1. **Evaluation**: Force evaluation of thunks as needed
//! 2. **Type checking**: Verify the value matches the expected type
//! 3. **Extraction**: Return the typed Rust value or an error
//!
//! Methods with `_strict` in their name also evaluate their return values before returning them.
//!
//! ### Evaluation Strictness
//!
//! - **Lazy methods** (e.g., [`require_list_size`](EvalState::require_list_size)):
//!   Evaluate only the structure needed
//! - **Strict methods** (e.g., [`require_list_strict`](EvalState::require_list_strict)):
//!   Force full evaluation of all contained values
//! - **Selective methods** (e.g., [`require_list_select_idx_strict`](EvalState::require_list_select_idx_strict)):
//!   Evaluate only the accessed elements
//!
//! ## Laziness and Strictness
//!
//! The terms "lazy" and "strict" in this API refer to Nix's [Weak Head Normal Form (WHNF)](https://nix.dev/manual/nix/latest/language/evaluation.html#values)
//! evaluation model, not the kind of deep strictness that is exercised by functions such as `builtins.toJSON` or `builtins.deepSeq`.
//!
//! - **WHNF evaluation**: Values are evaluated just enough to determine their type and basic structure
//! - **Deep evaluation**: All nested values are recursively forced (like `builtins.deepSeq`)
//!
//! For example, a list in WHNF has its length determined but individual elements may remain unevaluated thunks.
//! Methods marked as "strict" in this API force WHNF evaluation of their results, but do not perform deep evaluation
//! of arbitrarily nested structures unless explicitly documented otherwise.
//!
//! ### Thread Safety and Memory Management
//!
//! Before using [`EvalState`] in a thread, register it with the (process memory) garbage collector:
//!
//! ```rust,no_run
//! # use nix_expr::eval_state::{init, gc_register_my_thread, test_init};
//! # fn example() -> anyhow::Result<()> {
//! # test_init(); // Use test_init() in tests
//! init()?; // Initialize Nix library
//! let guard = gc_register_my_thread()?; // Register thread with GC
//! // Now safe to use EvalState in this thread
//! drop(guard);
//! # Ok(())
//! # }
//! ```
//!
//! ## Error Handling
//!
//! Evaluation methods return [`Result`] types. Common error scenarios include:
//! - **Type mismatches**: Expected type doesn't match actual value type
//! - **Evaluation errors**: Nix expressions that throw or have undefined behavior
//! - **Bounds errors**: Out-of-range access for indexed operations
//!
//! ## Examples
//!
//! ```rust
//! use nix_expr::eval_state::{EvalState, test_init, gc_register_my_thread};
//! use nix_store::store::Store;
//! use std::collections::HashMap;
//!
//! # fn main() -> anyhow::Result<()> {
//! test_init(); // init() in non-test code
//! let guard = gc_register_my_thread()?;
//!
//! let store = Store::open(None, HashMap::new())?;
//! let mut es = EvalState::new(store, [])?;
//!
//! // Evaluate a list expression
//! let list_value = es.eval_from_string("[1 2 3]", "<example>")?;
//!
//! // Check the size (lazy - doesn't evaluate elements)
//! let size = es.require_list_size(&list_value)?;
//! println!("List has {} elements", size);
//!
//! // Access specific elements (evaluates only accessed elements)
//! if let Some(first) = es.require_list_select_idx_strict(&list_value, 0)? {
//!     let value = es.require_int(&first)?;
//!     println!("First element: {}", value);
//! }
//!
//! // Process all elements (evaluates all elements)
//! let all_elements: Vec<_> = es.require_list_strict(&list_value)?;
//! for element in all_elements {
//!     let value = es.require_int(&element)?;
//!     println!("Element: {}", value);
//! }
//!
//! drop(guard);
//! # Ok(())
//! # }
//! ```

use crate::primop;
use crate::value::{Int, Value, ValueType};
use anyhow::Context as _;
use anyhow::{bail, Result};
use cstr::cstr;
use lazy_static::lazy_static;
use nix_c_raw as raw;
use nix_store::path::StorePath;
use nix_store::store::{Store, StoreWeak};
use nix_util::context::Context;
use nix_util::string_return::{callback_get_result_string, callback_get_result_string_data};
use nix_util::{check_call, check_call_opt_key, result_string_init};
use std::ffi::{c_char, CString};
use std::iter::FromIterator;
use std::os::raw::c_uint;
use std::ptr::{null, null_mut, NonNull};
use std::sync::{Arc, Weak};

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
            Err(anyhow::format_err!("nix_expr::init error: {}", e))
        }
    }
}

/// A string value with its associated [store paths](https://nix.dev/manual/nix/stable/store/store-path.html).
///
/// Represents a Nix string with references to store paths.
pub struct RealisedString {
    /// The string content.
    pub s: String,
    /// Store paths referenced by the string.
    pub paths: Vec<StorePath>,
}

/// A [Weak] reference to an [EvalState].
pub struct EvalStateWeak {
    inner: Weak<EvalStateRef>,
    store: StoreWeak,
}
impl EvalStateWeak {
    /// Upgrade the weak reference to a proper [EvalState].
    ///
    /// If no normal reference to the [EvalState] is around anymore elsewhere, this fails by returning `None`.
    pub fn upgrade(&self) -> Option<EvalState> {
        self.inner.upgrade().and_then(|eval_state| {
            self.store.upgrade().map(|store| EvalState {
                eval_state,
                store,
                context: Context::new(),
            })
        })
    }
}

struct EvalStateRef {
    eval_state: NonNull<raw::EvalState>,
}
impl EvalStateRef {
    /// Returns a raw pointer to the underlying EvalState.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the pointer is not used beyond the lifetime of the underlying [raw::EvalState].
    unsafe fn as_ptr(&self) -> *mut raw::EvalState {
        self.eval_state.as_ptr()
    }
}
impl Drop for EvalStateRef {
    fn drop(&mut self) {
        unsafe {
            raw::state_free(self.eval_state.as_ptr());
        }
    }
}
/// Builder for configuring and creating an [`EvalState`].
///
/// Provides advanced configuration options for evaluation context setup.
/// Use [`EvalState::new`] for simple cases or this builder for custom configuration.
///
/// # Examples
///
/// ```rust
/// # use nix_expr::eval_state::{EvalState, EvalStateBuilder, test_init, gc_register_my_thread};
/// # use nix_store::store::Store;
/// # use std::collections::HashMap;
/// # fn example() -> anyhow::Result<()> {
/// # test_init();
/// # let guard = gc_register_my_thread()?;
/// let store = Store::open(None, HashMap::new())?;
///
/// let mut es: EvalState = EvalStateBuilder::new(store)?
///     .lookup_path(["nixpkgs=/path/to/nixpkgs", "home-manager=/path/to/hm"])?
///     .build()?;
///
/// let value = es.eval_from_string("<nixpkgs>", /* path display: */ "in-memory")?;
/// # drop(guard);
/// # Ok(())
/// # }
/// ```
pub struct EvalStateBuilder {
    eval_state_builder: *mut raw::eval_state_builder,
    lookup_path: Vec<CString>,
    store: Store,
}
impl Drop for EvalStateBuilder {
    fn drop(&mut self) {
        unsafe {
            raw::eval_state_builder_free(self.eval_state_builder);
        }
    }
}
impl EvalStateBuilder {
    /// Creates a new [`EvalStateBuilder`].
    pub fn new(store: Store) -> Result<EvalStateBuilder> {
        let mut context = Context::new();
        let eval_state_builder =
            unsafe { check_call!(raw::eval_state_builder_new(&mut context, store.raw_ptr())) }?;
        Ok(EvalStateBuilder {
            store,
            eval_state_builder,
            lookup_path: Vec::new(),
        })
    }
    /// Sets the [lookup path](https://nix.dev/manual/nix/latest/language/constructs/lookup-path.html) for Nix expression evaluation.
    pub fn lookup_path<'a>(mut self, path: impl IntoIterator<Item = &'a str>) -> Result<Self> {
        let lookup_path: Vec<CString> = path
            .into_iter()
            .map(|path| {
                CString::new(path).with_context(|| {
                    format!("EvalStateBuilder::lookup_path: path `{path}` contains null byte")
                })
            })
            .collect::<Result<_>>()?;
        self.lookup_path = lookup_path;
        Ok(self)
    }
    /// Builds the configured [`EvalState`].
    pub fn build(&self) -> Result<EvalState> {
        // Make sure the library is initialized
        init()?;

        let mut context = Context::new();

        // Note: these raw C string pointers borrow from self.lookup_path
        let mut lookup_path: Vec<*const c_char> = self
            .lookup_path
            .iter()
            .map(|s| s.as_ptr())
            .chain(std::iter::once(null())) // signal the end of the array
            .collect();

        unsafe {
            check_call!(raw::eval_state_builder_set_lookup_path(
                &mut context,
                self.eval_state_builder,
                lookup_path.as_mut_ptr()
            ))?;
        }

        let eval_state =
            unsafe { check_call!(raw::eval_state_build(&mut context, self.eval_state_builder)) }?;
        Ok(EvalState {
            eval_state: Arc::new(EvalStateRef {
                eval_state: NonNull::new(eval_state).unwrap_or_else(|| {
                    panic!("nix_state_create returned a null pointer without an error")
                }),
            }),
            store: self.store.clone(),
            context,
        })
    }
    /// Returns a raw pointer to the underlying eval state builder.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the pointer is not used beyond the lifetime of this builder.
    // TODO: This function should be marked `unsafe`.
    pub fn raw_ptr(&self) -> *mut raw::eval_state_builder {
        self.eval_state_builder
    }
}

pub struct EvalState {
    eval_state: Arc<EvalStateRef>,
    store: Store,
    pub(crate) context: Context,
}
impl EvalState {
    /// Creates a new EvalState with basic configuration.
    ///
    /// For more options, use [EvalStateBuilder].
    pub fn new<'a>(store: Store, lookup_path: impl IntoIterator<Item = &'a str>) -> Result<Self> {
        EvalStateBuilder::new(store)?
            .lookup_path(lookup_path)?
            .build()
    }

    /// Returns a raw pointer to the raw Nix C API EvalState.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the pointer is not used beyond the lifetime of this `EvalState`.
    pub unsafe fn raw_ptr(&self) -> *mut raw::EvalState {
        self.eval_state.as_ptr()
    }

    /// Returns a reference to the Store that's used for instantiation, import from derivation, etc.
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Creates a weak reference to this EvalState.
    pub fn weak_ref(&self) -> EvalStateWeak {
        EvalStateWeak {
            inner: Arc::downgrade(&self.eval_state),
            store: self.store.weak_ref(),
        }
    }

    /// Parses and evaluates a Nix expression `expr`.
    ///
    /// Expressions can contain relative paths such as `./.` that are resolved relative to the given `path`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use nix_expr::eval_state::{EvalState, test_init, gc_register_my_thread};
    /// use nix_store::store::Store;
    /// use nix_expr::value::Value;
    /// use std::collections::HashMap;
    ///
    /// # fn main() -> anyhow::Result<()> {
    /// # test_init();
    /// # let guard = gc_register_my_thread()?;
    /// # let mut es = EvalState::new(Store::open(None, HashMap::new())?, [])?;
    /// let v: Value = es.eval_from_string("42", ".")?;
    /// assert_eq!(es.require_int(&v)?, 42);
    /// # drop(guard);
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias = "nix_expr_eval_from_string")]
    #[doc(alias = "parse")]
    #[doc(alias = "eval")]
    #[doc(alias = "evaluate")]
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

    /// Forces [evaluation](https://nix.dev/manual/nix/latest/language/evaluation.html) of a value to [weak head normal form](https://nix.dev/manual/nix/latest/language/evaluation.html?highlight=WHNF#values).
    ///
    /// Converts [thunks](https://nix.dev/manual/nix/latest/language/evaluation.html#laziness) to their evaluated form. Does not modify already-evaluated values.
    ///
    /// Does not perform deep evaluation of nested structures.
    #[doc(alias = "evaluate")]
    #[doc(alias = "strict")]
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

    /// Returns the type of a value without forcing [evaluation](https://nix.dev/manual/nix/latest/language/evaluation.html).
    ///
    /// Returns [`None`] if the value is an unevaluated [thunk](https://nix.dev/manual/nix/latest/language/evaluation.html#laziness).
    ///
    /// Returns [`Some`] if the value is already evaluated.
    #[doc(alias = "type_of")]
    #[doc(alias = "value_type_lazy")]
    #[doc(alias = "nix_get_type")]
    #[doc(alias = "get_type")]
    #[doc(alias = "nix_value_type")]
    pub fn value_type_unforced(&mut self, value: &Value) -> Option<ValueType> {
        let r = unsafe { check_call!(raw::get_type(&mut self.context, value.raw_ptr())) };
        // .unwrap(): no reason for this to fail, as it does not evaluate
        ValueType::from_raw(r.unwrap())
    }
    /// Returns the [type][`ValueType`] of a value, [forcing][`EvalState::force`] [evaluation](https://nix.dev/manual/nix/latest/language/evaluation.html) if necessary.
    ///
    /// Forces evaluation if the value is an unevaluated [thunk](https://nix.dev/manual/nix/latest/language/evaluation.html#laziness).
    ///
    /// Evaluation may fail, producing an [`Err`].
    ///
    /// Guarantees a definitive result if [`Ok`], thanks to the language being [pure](https://nix.dev/manual/nix/latest/language/index.html?highlight=pure#nix-language) and [lazy](https://nix.dev/manual/nix/latest/language/index.html?highlight=lazy#nix-language).
    #[doc(alias = "type_of")]
    #[doc(alias = "value_type_strict")]
    #[doc(alias = "nix_get_type")]
    #[doc(alias = "get_type")]
    #[doc(alias = "nix_value_type_strict")]
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
    /// Extracts the value from an [integer][`ValueType::Int`] Nix value.
    ///
    /// Forces [evaluation](https://nix.dev/manual/nix/latest/language/evaluation.html) and verifies the value is an integer.
    ///
    /// Returns the integer value if successful, or an [`Err`] if evaluation failed or the value is not an integer.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use nix_expr::eval_state::{EvalState, test_init, gc_register_my_thread};
    /// # use nix_store::store::Store;
    /// # use std::collections::HashMap;
    /// # fn example() -> anyhow::Result<()> {
    /// # test_init();
    /// # let guard = gc_register_my_thread()?;
    /// let store = Store::open(None, HashMap::new())?;
    /// let mut es = EvalState::new(store, [])?;
    ///
    /// let value = es.eval_from_string("42", "<example>")?;
    /// let int_val = es.require_int(&value)?;
    /// assert_eq!(int_val, 42);
    /// # drop(guard);
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias = "integer")]
    #[doc(alias = "number")]
    #[doc(alias = "nix_get_int")]
    #[doc(alias = "get_int")]
    pub fn require_int(&mut self, v: &Value) -> Result<Int> {
        let t = self.value_type(v)?;
        if t != ValueType::Int {
            bail!("expected an int, but got a {:?}", t);
        }
        unsafe { check_call!(raw::get_int(&mut self.context, v.raw_ptr())) }
    }

    /// Extracts the value from a [boolean][`ValueType::Bool`] Nix value.
    ///
    /// Forces [evaluation](https://nix.dev/manual/nix/latest/language/evaluation.html) and verifies the value is a boolean.
    ///
    /// Returns the boolean value if successful, or an [`Err`] if evaluation failed or the value is not a boolean.
    #[doc(alias = "boolean")]
    #[doc(alias = "nix_get_bool")]
    #[doc(alias = "get_bool")]
    pub fn require_bool(&mut self, v: &Value) -> Result<bool> {
        let t = self.value_type(v)?;
        if t != ValueType::Bool {
            bail!("expected a bool, but got a {:?}", t);
        }
        unsafe { check_call!(raw::get_bool(&mut self.context, v.raw_ptr())) }
    }

    /// Extracts all elements from a [list][`ValueType::List`] Nix value.
    ///
    /// Forces [evaluation](https://nix.dev/manual/nix/latest/language/evaluation.html) and verifies the value is a list.
    ///
    /// Returns the contained values in the specified container type (e.g., [`Vec`], [`VecDeque`][`std::collections::VecDeque`], etc.).
    ///
    /// This is [strict](https://nix.dev/manual/nix/latest/language/evaluation.html#strictness) - all list elements will be evaluated.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use nix_expr::value::Value;
    /// # use std::collections::{VecDeque, LinkedList};
    /// # fn example(es: &mut nix_expr::eval_state::EvalState, list_value: &Value) -> anyhow::Result<()> {
    /// let vec: Vec<Value> = es.require_list_strict(&list_value)?;
    /// let deque: VecDeque<Value> = es.require_list_strict(&list_value)?;
    /// let linked_list = es.require_list_strict::<LinkedList<Value>>(&list_value)?;
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias = "collect")]
    #[doc(alias = "to_vec")]
    #[doc(alias = "all")]
    #[doc(alias = "nix_get_list_size")]
    #[doc(alias = "nix_get_list_byidx")]
    pub fn require_list_strict<C>(&mut self, value: &Value) -> Result<C>
    where
        C: FromIterator<Value>,
    {
        let t = self.value_type(value)?;
        if t != ValueType::List {
            bail!("expected a list, but got a {:?}", t);
        }
        let size = unsafe { check_call!(raw::get_list_size(&mut self.context, value.raw_ptr())) }?;

        (0..size)
            .map(|i| {
                let element_ptr = unsafe {
                    check_call!(raw::get_list_byidx(
                        &mut self.context,
                        value.raw_ptr(),
                        self.eval_state.as_ptr(),
                        i
                    ))
                }?;
                Ok(unsafe { Value::new(element_ptr) })
            })
            .collect()
    }

    /// Evaluate, and require that the [`Value`] is a Nix [`ValueType::AttrSet`].
    ///
    /// Returns a list of the keys in the attrset.
    ///
    /// NOTE: this currently implements its own sorting, which probably matches Nix's implementation, but is not guaranteed.
    #[doc(alias = "keys")]
    #[doc(alias = "attributes")]
    #[doc(alias = "fields")]
    pub fn require_attrs_names(&mut self, v: &Value) -> Result<Vec<String>> {
        self.require_attrs_names_unsorted(v).map(|mut v| {
            v.sort();
            v
        })
    }

    /// For when [`EvalState::require_attrs_names`] isn't fast enough.
    ///
    /// Only use when it's ok that the keys are returned in an arbitrary order.
    #[doc(alias = "keys_unsorted")]
    #[doc(alias = "attributes_unsorted")]
    pub fn require_attrs_names_unsorted(&mut self, v: &Value) -> Result<Vec<String>> {
        let t = self.value_type(v)?;
        if t != ValueType::AttrSet {
            bail!("expected an attrset, but got a {:?}", t);
        }
        let n = unsafe { check_call!(raw::get_attrs_size(&mut self.context, v.raw_ptr())) }?;
        let mut attrs = Vec::with_capacity(n as usize);
        for i in 0..n {
            let cstr_ptr: *const c_char = unsafe {
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

    /// Extracts an attribute value from an [attribute set][`ValueType::AttrSet`] Nix value.
    ///
    /// Forces [evaluation](https://nix.dev/manual/nix/latest/language/evaluation.html) and verifies the value is an attribute set.
    ///
    /// Returns the attribute value if found, or an [`Err`] if evaluation failed, the attribute doesn't exist, or the value is not an attribute set.
    #[doc(alias = "get_attr")]
    #[doc(alias = "attribute")]
    #[doc(alias = "field")]
    pub fn require_attrs_select(&mut self, v: &Value, attr_name: &str) -> Result<Value> {
        let t = self.value_type(v)?;
        if t != ValueType::AttrSet {
            bail!("expected an attrset, but got a {:?}", t);
        }
        let attr_name = CString::new(attr_name)
            .with_context(|| "require_attrs_select: attrName contains null byte")?;
        unsafe {
            let v2 = check_call!(raw::get_attr_byname(
                &mut self.context,
                v.raw_ptr(),
                self.eval_state.as_ptr(),
                attr_name.as_ptr()
            ));
            match v2 {
                Ok(v2) => Ok(Value::new(v2)),
                Err(e) => {
                    // As of Nix 2.26, the error message is not helpful when it
                    // is simply missing, so we provide a better one. (Note that
                    // missing attributes requested by Nix expressions OTOH is a
                    // different error message which works fine.)
                    if e.to_string() == "missing attribute" {
                        bail!("attribute `{}` not found", attr_name.to_string_lossy());
                    } else {
                        Err(e)
                    }
                }
            }
        }
    }

    /// Extracts an optional attribute value from an [attribute set][`ValueType::AttrSet`] Nix value.
    ///
    /// Forces [evaluation](https://nix.dev/manual/nix/latest/language/evaluation.html) and verifies the value is an attribute set.
    ///
    /// Returns [`Err`] if evaluation failed or the value is not an attribute set.
    ///
    /// Returns [`Ok(None)`] if the attribute is not present.
    ///
    /// Returns [`Ok(Some(value))`] if the attribute is present.
    #[doc(alias = "nix_get_attr_byname")]
    #[doc(alias = "get_attr_byname")]
    #[doc(alias = "get_attr_opt")]
    #[doc(alias = "try_get")]
    #[doc(alias = "maybe_get")]
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
        Ok(v2.map(|x| unsafe { Value::new(x) }))
    }

    /// Returns the number of elements in a [list][`ValueType::List`] Nix value.
    ///
    /// Forces [evaluation](https://nix.dev/manual/nix/latest/language/evaluation.html) of the list structure and verifies the value is a list.
    ///
    /// Individual elements remain as lazy [thunks](https://nix.dev/manual/nix/latest/language/evaluation.html#laziness) and are not evaluated.
    #[doc(alias = "length")]
    #[doc(alias = "count")]
    #[doc(alias = "len")]
    #[doc(alias = "nix_get_list_size")]
    #[doc(alias = "get_list_size")]
    pub fn require_list_size(&mut self, v: &Value) -> Result<u32> {
        let t = self.value_type(v)?;
        if t != ValueType::List {
            bail!("expected a list, but got a {:?}", t);
        }
        let ret = unsafe { check_call!(raw::get_list_size(&mut self.context, v.raw_ptr())) }?;
        Ok(ret)
    }

    /// Extracts an element from a [list][`ValueType::List`] Nix value by index.
    ///
    /// Forces [evaluation](https://nix.dev/manual/nix/latest/language/evaluation.html) and verifies the value is a list.
    /// Forces evaluation of the selected element, similar to [`require_attrs_select`].
    ///
    /// Returns [`Ok(Some(value))`] if the element is found.
    ///
    /// Returns [`Ok(None)`] if the index is out of bounds.
    ///
    /// Returns [`Err`] if evaluation failed, the element contains an error (e.g., `throw`), or the value is not a list.
    #[doc(alias = "get")]
    #[doc(alias = "index")]
    #[doc(alias = "at")]
    #[doc(alias = "nix_get_list_byidx")]
    #[doc(alias = "get_list_byidx")]
    pub fn require_list_select_idx_strict(&mut self, v: &Value, idx: u32) -> Result<Option<Value>> {
        let t = self.value_type(v)?;
        if t != ValueType::List {
            bail!("expected a list, but got a {:?}", t);
        }

        // TODO: Remove this bounds checking once https://github.com/NixOS/nix/pull/14030
        // is merged, which will add proper bounds checking to the underlying C API.
        // Currently we perform bounds checking in Rust to avoid undefined behavior.
        let size = unsafe { check_call!(raw::get_list_size(&mut self.context, v.raw_ptr())) }?;

        if idx >= size {
            return Ok(None);
        }

        let v2 = unsafe {
            check_call_opt_key!(raw::get_list_byidx(
                &mut self.context,
                v.raw_ptr(),
                self.eval_state.as_ptr(),
                idx
            ))
        }?;
        Ok(v2.map(|x| unsafe { Value::new(x) }))
    }

    /// Creates a new [string][`ValueType::String`] Nix value.
    ///
    /// Returns a string value without any [string context](https://nix.dev/manual/nix/latest/language/string-context.html).
    #[doc(alias = "make_string")]
    #[doc(alias = "create_string")]
    #[doc(alias = "string_value")]
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

    /// Creates a new [integer][`ValueType::Int`] Nix value.
    #[doc(alias = "make_int")]
    #[doc(alias = "create_int")]
    #[doc(alias = "int_value")]
    #[doc(alias = "integer_value")]
    pub fn new_value_int(&mut self, i: Int) -> Result<Value> {
        let v = unsafe {
            let value = self.new_value_uninitialized()?;
            check_call!(raw::init_int(&mut self.context, value.raw_ptr(), i))?;
            value
        };
        Ok(v)
    }

    /// Creates a new [thunk](https://nix.dev/manual/nix/latest/language/evaluation.html#laziness) Nix value.
    ///
    /// The [thunk](https://nix.dev/manual/nix/latest/language/evaluation.html#laziness) will lazily evaluate to the result of the given Rust function when forced.
    /// The Rust function will be called with the current [`EvalState`] and must not return a thunk.
    ///
    /// The name is shown in stack traces.
    #[doc(alias = "make_thunk")]
    #[doc(alias = "create_thunk")]
    #[doc(alias = "lazy_value")]
    pub fn new_value_thunk(
        &mut self,
        name: &str,
        f: Box<dyn Fn(&mut EvalState) -> Result<Value>>,
    ) -> Result<Value> {
        // Nix doesn't have a function for creating a thunk, so we have to
        // create a function and pass it a dummy argument.
        let name = CString::new(name).with_context(|| "new_thunk: name contains null byte")?;
        let primop = primop::PrimOp::new(
            self,
            primop::PrimOpMeta {
                // name is observable in stack traces, ie if the thunk returns Err
                name: name.as_c_str(),
                // doc is unlikely to be observable, so we provide a constant one for simplicity.
                doc: cstr!("Performs an on demand computation, implemented outside the Nix language in native code."),
                // like doc, unlikely to be observed
                args: [CString::new("internal_unused").unwrap().as_c_str()],
            },
            Box::new(move |eval_state, _dummy: &[Value; 1]| f(eval_state)),
        )?;

        let p = self.new_value_primop(primop)?;
        self.new_value_apply(&p, &p)
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
    /// Extracts a string value from a [string][`ValueType::String`] Nix value.
    ///
    /// Forces [evaluation](https://nix.dev/manual/nix/latest/language/evaluation.html) and verifies the value is a string.
    /// Returns the string value if successful, or an [`Err`] if evaluation failed or the value is not a string.
    ///
    /// NOTE: this will be replaced by two methods, one that also returns the context, and one that checks that the context is empty.
    #[doc(alias = "str")]
    #[doc(alias = "text")]
    #[doc(alias = "nix_get_string")]
    #[doc(alias = "get_string")]
    pub fn require_string(&mut self, value: &Value) -> Result<String> {
        let t = self.value_type(value)?;
        if t != ValueType::String {
            bail!("expected a string, but got a {:?}", t);
        }
        self.get_string(value)
    }
    /// Realises a [string][`ValueType::String`] Nix value with context information.
    ///
    /// Forces [evaluation](https://nix.dev/manual/nix/latest/language/evaluation.html), verifies the value is a string, and builds any derivations
    /// referenced in the [string context](https://nix.dev/manual/nix/latest/language/string-context.html) if required.
    #[doc(alias = "realize_string")]
    #[doc(alias = "string_with_context")]
    #[doc(alias = "build_string")]
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
                let path = NonNull::new(path as *mut raw::StorePath).ok_or_else(|| {
                    anyhow::format_err!(
                        "nix_realised_string_get_store_path returned a null pointer"
                    )
                })?;
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

    /// Applies a function to an argument and returns the result.
    ///
    /// Forces [evaluation](https://nix.dev/manual/nix/latest/language/evaluation.html) of the function application.
    /// For a lazy version, see [`new_value_apply`].
    #[doc(alias = "nix_value_call")]
    #[doc(alias = "value_call")]
    #[doc(alias = "apply")]
    #[doc(alias = "invoke")]
    #[doc(alias = "execute")]
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

    /// Apply a sequence of [function applications](https://nix.dev/manual/nix/latest/language/operators.html#function-application).
    ///
    /// When argument `f` is a curried function, this applies each argument in sequence.
    /// Equivalent to the Nix expression `f arg1 arg2 arg3`.
    ///
    /// Returns a [`Value`] in at least weak head normal form if successful.
    ///
    /// Returns an [`Err`]
    /// - if `f` did not evaluate to a function
    /// - if `f arg1` had any problems
    /// - if `f arg1` did not evaluate to a function (for `(f arg1) arg2`)
    /// - etc
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use nix_expr::eval_state::{EvalState, test_init, gc_register_my_thread};
    /// # use nix_store::store::Store;
    /// # use std::collections::HashMap;
    /// # fn example() -> anyhow::Result<()> {
    /// # test_init();
    /// # let guard = gc_register_my_thread()?;
    /// let store = Store::open(None, HashMap::new())?;
    /// let mut es = EvalState::new(store, [])?;
    ///
    /// // Create a curried function: x: y: x + y
    /// let f = es.eval_from_string("x: y: x + y", "<example>")?;
    /// let arg1 = es.eval_from_string("5", "<example>")?;
    /// let arg2 = es.eval_from_string("3", "<example>")?;
    ///
    /// // Equivalent to: (x: y: x + y) 5 3
    /// let result = es.call_multi(&f, &[arg1, arg2])?;
    /// let value = es.require_int(&result)?;
    /// assert_eq!(value, 8);
    /// # drop(guard);
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias = "nix_value_call_multi")]
    #[doc(alias = "value_call_multi")]
    #[doc(alias = "apply_multi")]
    #[doc(alias = "curry")]
    #[doc(alias = "call_with_args")]
    pub fn call_multi(&mut self, f: &Value, args: &[Value]) -> Result<Value> {
        let value = self.new_value_uninitialized()?;
        unsafe {
            let mut args_ptrs = args.iter().map(|a| a.raw_ptr()).collect::<Vec<_>>();
            check_call!(raw::value_call_multi(
                &mut self.context,
                self.eval_state.as_ptr(),
                f.raw_ptr(),
                args_ptrs.len(),
                args_ptrs.as_mut_ptr(),
                value.raw_ptr()
            ))
        }?;
        Ok(value)
    }

    /// Applies a function to an argument lazily, creating a [thunk](https://nix.dev/manual/nix/latest/language/evaluation.html#laziness).
    ///
    /// Does not force [evaluation](https://nix.dev/manual/nix/latest/language/evaluation.html) of the function application.
    /// For an eager version, see [`call`].
    #[doc(alias = "lazy_apply")]
    #[doc(alias = "thunk_apply")]
    #[doc(alias = "defer_call")]
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
        unsafe {
            let value = check_call!(raw::alloc_value(
                &mut self.context,
                self.eval_state.as_ptr()
            ))?;
            Ok(Value::new(value))
        }
    }

    /// Creates a new [function][`ValueType::Function`] Nix value implemented by a Rust function.
    ///
    /// This is also known as a "primop" in Nix, short for primitive operation.
    /// Most of the `builtins.*` values are examples of primops, but this function
    /// does not affect `builtins`.
    #[doc(alias = "make_primop")]
    #[doc(alias = "create_function")]
    #[doc(alias = "builtin")]
    pub fn new_value_primop(&mut self, primop: primop::PrimOp) -> Result<Value> {
        let value = self.new_value_uninitialized()?;
        unsafe {
            check_call!(raw::init_primop(
                &mut self.context,
                value.raw_ptr(),
                primop.ptr
            ))?;
        };
        Ok(value)
    }

    /// Creates a new [attribute set][`ValueType::Attrs`] Nix value from an iterator of name-value pairs.
    ///
    /// Accepts any iterator that yields `(String, Value)` pairs and has an exact size.
    /// Common usage includes [`Vec`], [`HashMap`], and array literals.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use nix_expr::eval_state::{EvalState, test_init, gc_register_my_thread};
    /// # use nix_store::store::Store;
    /// # use std::collections::HashMap;
    /// # fn example() -> anyhow::Result<()> {
    /// # test_init();
    /// # let guard = gc_register_my_thread()?;
    /// let store = Store::open(None, HashMap::new())?;
    /// let mut es = EvalState::new(store, [])?;
    /// let a = es.new_value_int(1)?;
    /// let b = es.new_value_int(2)?;
    /// let c = es.new_value_int(3)?;
    /// let d = es.new_value_int(4)?;
    ///
    /// // From array
    /// let attrs1 = es.new_value_attrs([
    ///     ("x".to_string(), a),
    ///     ("y".to_string(), b)
    /// ])?;
    ///
    /// // From HashMap
    /// let mut map = HashMap::new();
    /// map.insert("foo".to_string(), c);
    /// map.insert("bar".to_string(), d);
    /// let attrs2 = es.new_value_attrs(map)?;
    /// # drop(guard);
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias = "make_attrs")]
    #[doc(alias = "create_attrset")]
    #[doc(alias = "object")]
    #[doc(alias = "record")]
    pub fn new_value_attrs<I>(&mut self, attrs: I) -> Result<Value>
    where
        I: IntoIterator<Item = (String, Value)>,
        I::IntoIter: ExactSizeIterator,
    {
        let iter = attrs.into_iter();
        let size = iter.len();
        let bindings_builder = BindingsBuilder::new(self, size)?;
        for (name, value) in iter {
            let name =
                CString::new(name).with_context(|| "new_value_attrs: name contains null byte")?;
            unsafe {
                check_call!(raw::bindings_builder_insert(
                    &mut self.context,
                    bindings_builder.ptr,
                    name.as_ptr(),
                    value.raw_ptr()
                ))?;
            }
        }
        let value = self.new_value_uninitialized()?;
        unsafe {
            check_call!(raw::make_attrs(
                &mut self.context,
                value.raw_ptr(),
                bindings_builder.ptr
            ))?;
        }
        Ok(value)
    }
}

// Internal RAII helper; could be refactored and made pub
struct BindingsBuilder {
    ptr: *mut raw::BindingsBuilder,
}
impl Drop for BindingsBuilder {
    fn drop(&mut self) {
        unsafe {
            raw::bindings_builder_free(self.ptr);
        }
    }
}
impl BindingsBuilder {
    fn new(eval_state: &mut EvalState, capacity: usize) -> Result<Self> {
        let ptr = unsafe {
            check_call!(raw::make_bindings_builder(
                &mut eval_state.context,
                eval_state.eval_state.as_ptr(),
                capacity
            ))
        }?;
        Ok(BindingsBuilder { ptr })
    }
}

/// Triggers garbage collection immediately.
#[doc(alias = "garbage_collect")]
#[doc(alias = "collect")]
#[doc(alias = "gc")]
pub fn gc_now() {
    unsafe {
        raw::gc_now();
    }
}

/// RAII guard for thread registration with the garbage collector.
///
/// Automatically unregisters the thread when dropped.
pub struct ThreadRegistrationGuard {
    must_unregister: bool,
}
impl Drop for ThreadRegistrationGuard {
    fn drop(&mut self) {
        if self.must_unregister {
            unsafe {
                raw::GC_unregister_my_thread();
            }
        }
    }
}

fn gc_register_my_thread_do_it() -> Result<()> {
    unsafe {
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

#[doc(alias = "register_thread")]
#[doc(alias = "thread_setup")]
#[doc(alias = "gc_register")]
pub fn gc_register_my_thread() -> Result<ThreadRegistrationGuard> {
    init()?;
    unsafe {
        let already_done = raw::GC_thread_is_registered();
        if already_done != 0 {
            return Ok(ThreadRegistrationGuard {
                must_unregister: false,
            });
        }
        gc_register_my_thread_do_it()?;
        Ok(ThreadRegistrationGuard {
            must_unregister: true,
        })
    }
}

impl Clone for EvalState {
    fn clone(&self) -> Self {
        EvalState {
            eval_state: self.eval_state.clone(),
            store: self.store.clone(),
            context: Context::new(),
        }
    }
}

/// Initialize the Nix library for testing. This includes some modifications to the Nix settings, that must not be used in production.
/// Use at your own peril, in rust test suites.
#[doc(alias = "test_initialize")]
#[doc(alias = "test_setup")]
pub fn test_init() {
    init().unwrap();

    // During development, we encountered a problem where the build hook
    // would cause the test suite to reinvokes itself, causing an infinite loop.
    // While _NIX_TEST_NO_SANDBOX=1 should prevent this, we may also set the
    // build hook to "" to prevent this.
    nix_util::settings::set("build-hook", "").unwrap();

    // When testing in the sandbox, the default build dir would be a parent of the storeDir,
    // which causes an error. So we set a custom build dir here.
    // Only available on linux
    if cfg!(target_os = "linux") {
        nix_util::settings::set("sandbox-build-dir", "/custom-build-dir-for-test").unwrap();
    }
    std::env::set_var("_NIX_TEST_NO_SANDBOX", "1");

    // The tests run offline
    nix_util::settings::set("substituters", "").unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use cstr::cstr;
    use ctor::ctor;
    use std::collections::HashMap;
    use std::fs::read_dir;
    use std::io::Write as _;
    use std::sync::{Arc, Mutex};

    #[ctor]
    fn setup() {
        test_init();
    }

    /// Run a function while making sure that the current thread is registered with the GC.
    pub fn gc_registering_current_thread<F, R>(f: F) -> Result<R>
    where
        F: FnOnce() -> R,
    {
        let guard = gc_register_my_thread()?;
        let r = f();
        drop(guard);
        Ok(r)
    }

    #[test]
    fn eval_state_new_and_drop() {
        gc_registering_current_thread(|| {
            // very basic test: make sure initialization doesn't crash
            let store = Store::open(None, HashMap::new()).unwrap();
            let _e = EvalState::new(store, []).unwrap();
        })
        .unwrap();
    }

    #[test]
    fn weak_ref() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
            let es = EvalState::new(store, []).unwrap();
            let weak = es.weak_ref();
            let _es = weak.upgrade().unwrap();
        })
        .unwrap();
    }

    #[test]
    fn weak_ref_gone() {
        gc_registering_current_thread(|| {
            let weak = {
                // Use a slightly different URL which is unique in the test suite, to bypass the global store cache
                let store = Store::open(Some("auto?foo=bar"), HashMap::new()).unwrap();
                let es = EvalState::new(store, []).unwrap();
                es.weak_ref()
            };
            assert!(weak.upgrade().is_none());
            assert!(weak.store.upgrade().is_none());
            assert!(weak.inner.upgrade().is_none());
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
            let mut es = EvalState::new(Store::open(None, HashMap::new()).unwrap(), []).unwrap();
            assert!(es.eval_from_string(import_expression, "<test>").is_err());

            let mut es = EvalState::new(
                Store::open(None, HashMap::new()).unwrap(),
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
            let store = Store::open(None, HashMap::new()).unwrap();
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
            let store = Store::open(None, HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("true", "<test>").unwrap();
            es.force(&v).unwrap();
            let t = es.value_type_unforced(&v);
            assert!(t == Some(ValueType::Bool));
            let b = es.require_bool(&v).unwrap();
            assert!(b);

            let v = es.eval_from_string("false", "<test>").unwrap();
            es.require_bool(&v).unwrap();
            let b = es.require_bool(&v).unwrap();
            assert!(!b);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_value_int() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
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
            let store = Store::open(None, HashMap::new()).unwrap();
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

    #[test]
    fn eval_state_require_bool_forces_thunk() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let f = es.eval_from_string("x: !x", "<test>").unwrap();
            let a = es.eval_from_string("true", "<test>").unwrap();
            let v = es.new_value_apply(&f, &a).unwrap();
            let t = es.value_type_unforced(&v);
            assert!(t == None);
            let i = es.require_bool(&v).unwrap();
            assert!(i == false);
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
            let store = Store::open(None, HashMap::new()).unwrap();
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
            let store = Store::open(None, HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("{ }", "<test>").unwrap();
            es.force(&v).unwrap();
            let t = es.value_type_unforced(&v);
            assert!(t == Some(ValueType::AttrSet));
            let attrs = es.require_attrs_names_unsorted(&v).unwrap();
            assert_eq!(attrs.len(), 0);
        })
        .unwrap()
    }

    #[test]
    fn eval_state_require_attrs_names_unsorted_forces_thunk() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = make_thunk(&mut es, "{ a = 1; b = 2; }");
            let t = es.value_type_unforced(&v);
            assert!(t == None);
            let attrs = es.require_attrs_names_unsorted(&v).unwrap();
            assert_eq!(attrs.len(), 2);
        })
        .unwrap()
    }

    #[test]
    fn eval_state_require_attrs_names_unsorted_bad_type() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("1", "<test>").unwrap();
            es.force(&v).unwrap();
            let r = es.require_attrs_names_unsorted(&v);
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
            let store = Store::open(None, HashMap::new()).unwrap();
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
            let store = Store::open(None, HashMap::new()).unwrap();
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
                    if !s.contains("attribute `c` not found") {
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
            let store = Store::open(None, HashMap::new()).unwrap();
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
            let store = Store::open(None, HashMap::new()).unwrap();
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
            let store = Store::open(None, HashMap::new()).unwrap();
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
            let store = Store::open(None, HashMap::new()).unwrap();
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
            let store = Store::open(None, HashMap::new()).unwrap();
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
            let store = Store::open(None, HashMap::new()).unwrap();
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
            let store = Store::open(None, HashMap::new()).unwrap();
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
            let store = Store::open(None, HashMap::new()).unwrap();
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
            let store = Store::open(None, HashMap::new()).unwrap();
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
            let store = Store::open(None, HashMap::new()).unwrap();
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
            let store = Store::open(None, HashMap::new()).unwrap();
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
            let store = Store::open(None, HashMap::new()).unwrap();
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
            let store = Store::open(None, HashMap::new()).unwrap();
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
            let store = Store::open(None, HashMap::new()).unwrap();
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
            let store = Store::open(None, HashMap::new()).unwrap();
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
            let store = Store::open(None, HashMap::new()).unwrap();
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
            let store = Store::open(None, HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("[ ]", "<test>").unwrap();
            es.force(&v).unwrap();
            let t = es.value_type_unforced(&v);
            assert!(t == Some(ValueType::List));
        })
        .unwrap();
    }

    #[test]
    fn eval_state_value_list_strict_empty() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("[]", "<test>").unwrap();
            es.force(&v).unwrap();
            let list: Vec<Value> = es.require_list_strict(&v).unwrap();
            assert_eq!(list.len(), 0);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_value_list_strict_int() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("[42]", "<test>").unwrap();
            es.force(&v).unwrap();
            let list: Vec<Value> = es.require_list_strict(&v).unwrap();
            assert_eq!(list.len(), 1);
            assert_eq!(es.require_int(&list[0]).unwrap(), 42);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_value_list_strict_int_bool() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("[42 true]", "<test>").unwrap();
            es.force(&v).unwrap();
            let list: Vec<Value> = es.require_list_strict(&v).unwrap();
            assert_eq!(list.len(), 2);
            assert_eq!(es.require_int(&list[0]).unwrap(), 42);
            assert_eq!(es.require_bool(&list[1]).unwrap(), true);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_value_list_strict_error() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string(r#"[(throw "_evaluated_item_")]"#, "<test>").unwrap();
            es.force(&v).unwrap();
            // This should fail because require_list_strict evaluates all elements
            let result: Result<Vec<Value>, _> = es.require_list_strict(&v);
            assert!(result.is_err());
            match result {
                Err(error_msg) => {
                    let error_str = error_msg.to_string();
                    assert!(error_str.contains("_evaluated_item_"));
                }
                Ok(_) => panic!("unexpected success. The item should have been evaluated and its error propagated.")
            }
        })
        .unwrap();
    }

    #[test]
    fn eval_state_value_list_strict_generic_container() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("[1 2 3]", "<test>").unwrap();

            // Test with Vec
            let vec: Vec<Value> = es.require_list_strict(&v).unwrap();
            assert_eq!(vec.len(), 3);

            // Test with VecDeque
            let deque: std::collections::VecDeque<Value> = es.require_list_strict(&v).unwrap();
            assert_eq!(deque.len(), 3);

            // Verify contents are the same
            assert_eq!(es.require_int(&vec[0]).unwrap(), 1);
            assert_eq!(es.require_int(&deque[0]).unwrap(), 1);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_realise_string() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
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
            let store = Store::open(None, HashMap::new()).unwrap();
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
    fn eval_state_call_multi() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            // This is a function that takes two arguments.
            let f = es.eval_from_string("x: y: x - y", "<test>").unwrap();
            let a = es.eval_from_string("2", "<test>").unwrap();
            let b = es.eval_from_string("3", "<test>").unwrap();
            let v = es.call_multi(&f, &[a, b]).unwrap();
            let t = es.value_type_unforced(&v);
            assert!(t == Some(ValueType::Int));
            let i = es.require_int(&v).unwrap();
            assert!(i == -1);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_apply() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
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
            let store = Store::open(None, HashMap::new()).unwrap();
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
    fn eval_state_call_multi_fail_body() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            // This is a function that takes two arguments.
            let f = es.eval_from_string("x: y: x - y", "<test>").unwrap();
            let a = es.eval_from_string("2", "<test>").unwrap();
            let b = es.eval_from_string("true", "<test>").unwrap();
            let r = es.call_multi(&f, &[a, b]);
            match r {
                Ok(_) => panic!("expected an error"),
                Err(e) => {
                    if !e.to_string().contains("expected an integer but found") {
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
            let store = Store::open(None, HashMap::new()).unwrap();
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
            let store = Store::open(None, HashMap::new()).unwrap();
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

    #[test]
    fn eval_state_call_multi_fail_args() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            // This is a function that takes two arguments.
            let f = es.eval_from_string("{x}: {y}: x - y", "<test>").unwrap();
            let a = es.eval_from_string("{x = 2;}", "<test>").unwrap();
            let b = es.eval_from_string("{}", "<test>").unwrap();
            let r = es.call_multi(&f, &[a, b]);
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
            let store = Store::open(None, HashMap::new()).unwrap();
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
                    Some("local"),
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

    #[test]
    fn eval_state_primop_anon_call() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, []).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let bias: Arc<Mutex<Int>> = Arc::new(Mutex::new(0));
            let bias_control = bias.clone();

            let primop = primop::PrimOp::new(
                &mut es,
                primop::PrimOpMeta {
                    name: cstr!("testFunction"),
                    args: [cstr!("a"), cstr!("b")],
                    doc: cstr!("anonymous test function"),
                },
                Box::new(move |es, [a, b]| {
                    let a = es.require_int(a)?;
                    let b = es.require_int(b)?;
                    let c = *bias.lock().unwrap();
                    Ok(es.new_value_int(a + b + c)?)
                }),
            )
            .unwrap();

            let f = es.new_value_primop(primop).unwrap();

            {
                *bias_control.lock().unwrap() = 10;
            }
            let a = es.new_value_int(2).unwrap();
            let b = es.new_value_int(3).unwrap();
            let fa = es.call(f, a).unwrap();
            let v = es.call(fa, b).unwrap();
            es.force(&v).unwrap();
            let t = es.value_type(&v).unwrap();
            assert!(t == ValueType::Int);
            let i = es.require_int(&v).unwrap();
            assert!(i == 15);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_primop_anon_call_throw() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, []).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let f = {
                let es: &mut EvalState = &mut es;
                let prim = primop::PrimOp::new(
                    es,
                    primop::PrimOpMeta {
                        name: cstr!("throwingTestFunction"),
                        args: [cstr!("arg")],
                        doc: cstr!("anonymous test function"),
                    },
                    Box::new(move |es, [a]| {
                        let a = es.require_int(a)?;
                        bail!("error with arg [{}]", a);
                    }),
                )
                .unwrap();

                es.new_value_primop(prim)
            }
            .unwrap();
            let a = es.new_value_int(2).unwrap();
            let r = es.call(f, a);
            match r {
                Ok(_) => panic!("expected an error"),
                Err(e) => {
                    if !e.to_string().contains("error with arg [2]") {
                        eprintln!("unexpected error message: {}", e);
                        assert!(false);
                    }
                }
            }
        })
        .unwrap();
    }

    #[test]
    fn eval_state_primop_anon_call_no_args() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, []).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es
                .new_value_thunk(
                    "test_thunk",
                    Box::new(move |es: &mut EvalState| Ok(es.new_value_int(42)?)),
                )
                .unwrap();
            es.force(&v).unwrap();
            let t = es.value_type(&v).unwrap();
            eprintln!("{:?}", t);
            assert!(t == ValueType::Int);
            let i = es.require_int(&v).unwrap();
            assert!(i == 42);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_primop_anon_call_no_args_lazy() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, []).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es
                .new_value_thunk(
                    "test_thunk",
                    Box::new(move |_| {
                        bail!("error message in test case eval_state_primop_anon_call_no_args_lazy")
                    }),
                )
                .unwrap();
            let r = es.force(&v);
            match r {
                Ok(_) => panic!("expected an error"),
                Err(e) => {
                    if !e.to_string().contains(
                        "error message in test case eval_state_primop_anon_call_no_args_lazy",
                    ) {
                        eprintln!("unexpected error message: {}", e);
                        assert!(false);
                    }
                    if !e.to_string().contains("test_thunk") {
                        eprintln!("unexpected error message: {}", e);
                        assert!(false);
                    }
                }
            }
        })
        .unwrap();
    }

    #[test]
    pub fn eval_state_primop_custom() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, []).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let primop = primop::PrimOp::new(
                &mut es,
                primop::PrimOpMeta {
                    name: cstr!("frobnicate"),
                    doc: cstr!("Frobnicates widgets"),
                    args: [cstr!("x"), cstr!("y")],
                },
                Box::new(|es, args| {
                    let a = es.require_int(&args[0])?;
                    let b = es.require_int(&args[1])?;
                    Ok(es.new_value_int(a + b)?)
                }),
            )
            .unwrap();
            let f = es.new_value_primop(primop).unwrap();
            let a = es.new_value_int(2).unwrap();
            let b = es.new_value_int(3).unwrap();
            let fa = es.call(f, a).unwrap();
            let fb = es.call(fa, b).unwrap();
            es.force(&fb).unwrap();
            let t = es.value_type(&fb).unwrap();
            assert!(t == ValueType::Int);
            let i = es.require_int(&fb).unwrap();
            assert!(i == 5);
        })
        .unwrap();
    }

    #[test]
    pub fn eval_state_primop_custom_throw() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, []).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let primop = primop::PrimOp::new(
                &mut es,
                primop::PrimOpMeta {
                    name: cstr!("frobnicate"),
                    doc: cstr!("Frobnicates widgets"),
                    args: [cstr!("x")],
                },
                Box::new(|_es, _args| bail!("The frob unexpectedly fizzled")),
            )
            .unwrap();
            let f = es.new_value_primop(primop).unwrap();
            let a = es.new_value_int(0).unwrap();
            match es.call(f, a) {
                Ok(_) => panic!("expected an error"),
                Err(e) => {
                    if !e.to_string().contains("The frob unexpectedly fizzled") {
                        eprintln!("unexpected error message: {}", e);
                        assert!(false);
                    }
                    if !e.to_string().contains("frobnicate") {
                        eprintln!("unexpected error message: {}", e);
                        assert!(false);
                    }
                }
            }
        })
        .unwrap();
    }

    #[test]
    pub fn eval_state_new_value_attrs_from_slice_empty() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, []).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let attrs = es.new_value_attrs([]).unwrap();
            let t = es.value_type(&attrs).unwrap();
            assert!(t == ValueType::AttrSet);
            let names = es.require_attrs_names(&attrs).unwrap();
            assert!(names.is_empty());
        })
        .unwrap();
    }

    #[test]
    pub fn eval_state_new_value_attrs_from_vec() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, []).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let attrs = {
                let a = es.new_value_int(1).unwrap();
                let b = es.new_value_int(2).unwrap();
                es.new_value_attrs(vec![("a".to_string(), a), ("b".to_string(), b)])
                    .unwrap()
            };
            let t = es.value_type(&attrs).unwrap();
            assert!(t == ValueType::AttrSet);
            let names = es.require_attrs_names(&attrs).unwrap();
            assert_eq!(names.len(), 2);
            assert_eq!(names[0], "a");
            assert_eq!(names[1], "b");
            let a = es.require_attrs_select(&attrs, "a").unwrap();
            let b = es.require_attrs_select(&attrs, "b").unwrap();
            let i = es.require_int(&a).unwrap();
            assert_eq!(i, 1);
            let i = es.require_int(&b).unwrap();
            assert_eq!(i, 2);
        })
        .unwrap();
    }

    #[test]
    pub fn eval_state_new_value_attrs_from_hashmap() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, []).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let attrs = {
                let a = es.new_value_int(1).unwrap();
                let b = es.new_value_int(2).unwrap();
                es.new_value_attrs(HashMap::from([("a".to_string(), a), ("b".to_string(), b)]))
                    .unwrap()
            };
            let t = es.value_type(&attrs).unwrap();
            assert!(t == ValueType::AttrSet);
            let names = es.require_attrs_names(&attrs).unwrap();
            assert_eq!(names.len(), 2);
            assert_eq!(names[0], "a");
            assert_eq!(names[1], "b");
            let a = es.require_attrs_select(&attrs, "a").unwrap();
            let b = es.require_attrs_select(&attrs, "b").unwrap();
            let i = es.require_int(&a).unwrap();
            assert_eq!(i, 1);
            let i = es.require_int(&b).unwrap();
            assert_eq!(i, 2);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_require_list_select_idx_strict_basic() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("[ 10 20 30 ]", "<test>").unwrap();

            let elem0 = es.require_list_select_idx_strict(&v, 0).unwrap().unwrap();
            let elem1 = es.require_list_select_idx_strict(&v, 1).unwrap().unwrap();
            let elem2 = es.require_list_select_idx_strict(&v, 2).unwrap().unwrap();

            assert_eq!(es.require_int(&elem0).unwrap(), 10);
            assert_eq!(es.require_int(&elem1).unwrap(), 20);
            assert_eq!(es.require_int(&elem2).unwrap(), 30);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_require_list_select_idx_strict_out_of_bounds() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("[ 1 2 3 ]", "<test>").unwrap();

            let out_of_bounds = es.require_list_select_idx_strict(&v, 3).unwrap();
            assert!(out_of_bounds.is_none());

            // Test boundary case - the last valid index
            let last_elem = es.require_list_select_idx_strict(&v, 2).unwrap().unwrap();
            assert_eq!(es.require_int(&last_elem).unwrap(), 3);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_require_list_select_idx_strict_empty_list() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("[ ]", "<test>").unwrap();

            // Test that the safe version properly handles empty list access
            let elem = es.require_list_select_idx_strict(&v, 0).unwrap();
            assert!(elem.is_none());

            // Verify we can get the size of an empty list
            let size = es.require_list_size(&v).unwrap();
            assert_eq!(size, 0);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_require_list_select_idx_strict_forces_thunk() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = make_thunk(&mut es, "[ 42 ]");
            assert!(es.value_type_unforced(&v).is_none());

            let elem = es.require_list_select_idx_strict(&v, 0).unwrap().unwrap();
            assert_eq!(es.require_int(&elem).unwrap(), 42);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_require_list_select_idx_strict_error_element() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();

            let v = es
                .eval_from_string("[ (1 + 1) (throw \"error\") (3 + 3) ]", "<test>")
                .unwrap();

            let elem0 = es.require_list_select_idx_strict(&v, 0).unwrap().unwrap();
            assert_eq!(es.require_int(&elem0).unwrap(), 2);

            let elem2 = es.require_list_select_idx_strict(&v, 2).unwrap().unwrap();
            assert_eq!(es.require_int(&elem2).unwrap(), 6);

            let elem1_result = es.require_list_select_idx_strict(&v, 1);
            match elem1_result {
                Ok(_) => panic!("expected an error from throw during selection"),
                Err(e) => {
                    assert!(e.to_string().contains("error"));
                }
            }
        })
        .unwrap();
    }

    #[test]
    fn eval_state_require_list_select_idx_strict_wrong_type() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("42", "<test>").unwrap();

            let r = es.require_list_select_idx_strict(&v, 0);
            match r {
                Ok(_) => panic!("expected an error"),
                Err(e) => {
                    let err_msg = e.to_string();
                    assert!(err_msg.contains("expected a list, but got a"));
                }
            }
        })
        .unwrap();
    }

    #[test]
    fn eval_state_require_list_size_basic() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();

            let empty = es.eval_from_string("[ ]", "<test>").unwrap();
            assert_eq!(es.require_list_size(&empty).unwrap(), 0);

            let three_elem = es.eval_from_string("[ 1 2 3 ]", "<test>").unwrap();
            assert_eq!(es.require_list_size(&three_elem).unwrap(), 3);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_require_list_size_forces_thunk() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = make_thunk(&mut es, "[ 1 2 3 4 5 ]");
            assert!(es.value_type_unforced(&v).is_none());

            let size = es.require_list_size(&v).unwrap();
            assert_eq!(size, 5);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_require_list_size_lazy_elements() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();

            let v = es
                .eval_from_string(
                    "[ (throw \"error1\") (throw \"error2\") (throw \"error3\") ]",
                    "<test>",
                )
                .unwrap();

            let size = es.require_list_size(&v).unwrap();
            assert_eq!(size, 3);
        })
        .unwrap();
    }

    #[test]
    fn eval_state_require_list_size_wrong_type() {
        gc_registering_current_thread(|| {
            let store = Store::open(None, HashMap::new()).unwrap();
            let mut es = EvalState::new(store, []).unwrap();
            let v = es.eval_from_string("\"not a list\"", "<test>").unwrap();

            let r = es.require_list_size(&v);
            match r {
                Ok(_) => panic!("expected an error"),
                Err(e) => {
                    let err_msg = e.to_string();
                    assert!(err_msg.contains("expected a list, but got a"));
                }
            }
        })
        .unwrap();
    }
}
