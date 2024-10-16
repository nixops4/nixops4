use nix_c_raw as raw;
use nix_util::{check_call, context::Context};
use std::ptr::{null_mut, NonNull};

// TODO: test: cloning a thunk does not duplicate the evaluation.

pub type Int = i64;

/// The type of a value (or thunk)
#[derive(Eq, PartialEq, Debug)]
pub enum ValueType {
    AttrSet,
    Bool,
    External,
    Float,
    Function,
    Int,
    List,
    Null,
    Path,
    String,
    Unknown,
}

impl ValueType {
    /// Convert a raw value type to a `ValueType`.
    ///
    /// Return `None` if the Value is still a thunk (i.e. not yet evaluated).
    ///
    /// Return `Some(ValueType::Unknown)` if the value type is not recognized.
    pub(crate) fn from_raw(raw: raw::ValueType) -> Option<ValueType> {
        match raw {
            raw::ValueType_NIX_TYPE_ATTRS => Some(ValueType::AttrSet),
            raw::ValueType_NIX_TYPE_BOOL => Some(ValueType::Bool),
            raw::ValueType_NIX_TYPE_EXTERNAL => Some(ValueType::External),
            raw::ValueType_NIX_TYPE_FLOAT => Some(ValueType::Float),
            raw::ValueType_NIX_TYPE_FUNCTION => Some(ValueType::Function),
            raw::ValueType_NIX_TYPE_INT => Some(ValueType::Int),
            raw::ValueType_NIX_TYPE_LIST => Some(ValueType::List),
            raw::ValueType_NIX_TYPE_NULL => Some(ValueType::Null),
            raw::ValueType_NIX_TYPE_PATH => Some(ValueType::Path),
            raw::ValueType_NIX_TYPE_STRING => Some(ValueType::String),

            raw::ValueType_NIX_TYPE_THUNK => None,

            // This would happen if a new type of value is added in Nix.
            _ => Some(ValueType::Unknown),
        }
    }
}

/* A pointer to a value or thunk, to be used with EvalState methods. */
pub struct Value {
    inner: NonNull<raw::Value>,
}
impl Value {
    /// Take ownership of a new Value.
    ///
    /// This does not call `nix_gc_incref`, but does call `nix_gc_decref` when dropped.
    pub(crate) fn new(inner: *mut raw::Value) -> Self {
        Value {
            inner: NonNull::new(inner).unwrap(),
        }
    }

    /// Borrow a reference to a Value.
    ///
    /// This calls `nix_gc_incref`, and the returned Value will call `nix_gc_decref` when dropped.
    pub(crate) fn new_borrowed(inner: *mut raw::Value) -> Self {
        let v = Value::new(inner);
        unsafe { raw::value_incref(null_mut(), inner) };
        v
    }

    pub(crate) fn raw_ptr(&self) -> *mut raw::Value {
        self.inner.as_ptr()
    }
}
impl Drop for Value {
    fn drop(&mut self) {
        unsafe {
            // ignoring error because the only failure mode is leaking memory
            raw::value_decref(null_mut(), self.inner.as_ptr());
        }
    }
}
impl Clone for Value {
    fn clone(&self) -> Self {
        // TODO: Is it worth allocating a new Context here? Ideally cloning is cheap.
        //       this is very unlikely to error, and it is not recoverable
        //       Maybe try without, and try again with context to report details?
        unsafe {
            check_call!(raw::value_incref(&mut Context::new(), self.inner.as_ptr())).unwrap();
        }
        // can't return an error here, but we don't want to ignore the error either as it means we could use-after-free
        Value { inner: self.inner }
    }
}

// Tested in eval_state.rs
