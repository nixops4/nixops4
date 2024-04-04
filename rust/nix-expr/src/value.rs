use nix_c_raw as raw;
use nix_util::context::Context;
use std::ptr::NonNull;

// TODO: test: cloning a thunk does not duplicate the evaluation.

/** The type of a value (or thunk) */
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
    Thunk,
    Unknown,
}

impl ValueType {
    pub(crate) fn from_raw(raw: raw::ValueType) -> ValueType {
        match raw {
            raw::ValueType_NIX_TYPE_ATTRS => ValueType::AttrSet,
            raw::ValueType_NIX_TYPE_BOOL => ValueType::Bool,
            raw::ValueType_NIX_TYPE_EXTERNAL => ValueType::External,
            raw::ValueType_NIX_TYPE_FLOAT => ValueType::Float,
            raw::ValueType_NIX_TYPE_FUNCTION => ValueType::Function,
            raw::ValueType_NIX_TYPE_INT => ValueType::Int,
            raw::ValueType_NIX_TYPE_LIST => ValueType::List,
            raw::ValueType_NIX_TYPE_NULL => ValueType::Null,
            raw::ValueType_NIX_TYPE_PATH => ValueType::Path,
            raw::ValueType_NIX_TYPE_STRING => ValueType::String,
            raw::ValueType_NIX_TYPE_THUNK => ValueType::Thunk,
            _ => ValueType::Unknown,
        }
    }
}

/* A pointer to a value or thunk, to be used with EvalState methods. */
pub struct Value {
    inner: NonNull<raw::Value>,
}
impl Value {
    pub(crate) fn new(inner: *mut raw::Value) -> Self {
        Value {
            inner: NonNull::new(inner).unwrap(),
        }
    }
    pub(crate) fn raw_ptr(&self) -> *mut raw::Value {
        self.inner.as_ptr()
    }
}
impl Drop for Value {
    fn drop(&mut self) {
        let context = Context::new();
        unsafe {
            raw::nix_gc_decref(context.ptr(), self.inner.as_ptr());
        }
        // ignore error from context, because drop should not panic
    }
}
impl Clone for Value {
    fn clone(&self) -> Self {
        let context = Context::new();
        unsafe { raw::nix_gc_incref(context.ptr(), self.inner.as_ptr()) };
        context.check_err().unwrap();
        Value { inner: self.inner }
    }
}

// Tested in eval_state.rs
