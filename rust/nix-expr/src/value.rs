use nix_c_raw as raw;
use nix_util::context::Context;
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

#[derive(Eq, PartialEq, Debug)]
pub enum ValueTypeOrThunk {
    ValueType(ValueType),
    Thunk,
}

impl ValueTypeOrThunk {
    pub(crate) fn from_raw(raw: raw::ValueType) -> ValueTypeOrThunk {
        match raw {
            raw::ValueType_NIX_TYPE_ATTRS => ValueTypeOrThunk::ValueType(ValueType::AttrSet),
            raw::ValueType_NIX_TYPE_BOOL => ValueTypeOrThunk::ValueType(ValueType::Bool),
            raw::ValueType_NIX_TYPE_EXTERNAL => ValueTypeOrThunk::ValueType(ValueType::External),
            raw::ValueType_NIX_TYPE_FLOAT => ValueTypeOrThunk::ValueType(ValueType::Float),
            raw::ValueType_NIX_TYPE_FUNCTION => ValueTypeOrThunk::ValueType(ValueType::Function),
            raw::ValueType_NIX_TYPE_INT => ValueTypeOrThunk::ValueType(ValueType::Int),
            raw::ValueType_NIX_TYPE_LIST => ValueTypeOrThunk::ValueType(ValueType::List),
            raw::ValueType_NIX_TYPE_NULL => ValueTypeOrThunk::ValueType(ValueType::Null),
            raw::ValueType_NIX_TYPE_PATH => ValueTypeOrThunk::ValueType(ValueType::Path),
            raw::ValueType_NIX_TYPE_STRING => ValueTypeOrThunk::ValueType(ValueType::String),
            raw::ValueType_NIX_TYPE_THUNK => ValueTypeOrThunk::Thunk,
            // This would happen if a new type of value is added in Nix.
            _ => ValueTypeOrThunk::ValueType(ValueType::Unknown),
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
        unsafe {
            // ignoring error because the only failure mode is leaking memory
            raw::gc_decref(null_mut(), self.inner.as_ptr());
        }
    }
}
impl Clone for Value {
    fn clone(&self) -> Self {
        let context = Context::new();
        unsafe { raw::gc_incref(context.ptr(), self.inner.as_ptr()) };
        // can't return an error here, but we don't want to ignore the error either as it means we could use-after-free
        context.check_err().unwrap();
        Value { inner: self.inner }
    }
}

// Tested in eval_state.rs
