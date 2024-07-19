use crate::eval_state::{EvalState, EvalStateWeak};
use crate::value::Value;
use anyhow::Result;
use nix_c_raw as raw;
use nix_util::check_call;
use std::ffi::{c_int, c_void, CStr, CString};
use std::mem::ManuallyDrop;
use std::ptr::{null, null_mut};

/// Metadata for a primop, used with `PrimOp::new`.
pub struct PrimOpMeta<'a, const N: usize> {
    /// Name of the primop. Note that primops do not have to be registered as
    /// builtins. Nonetheless, a name is required for documentation purposes, e.g.
    /// :doc in the repl.
    pub name: &'a CStr,

    /// Documentation for the primop. This is displayed in the repl when using
    /// :doc. The format is markdown.
    pub doc: &'a CStr,

    /// The number of arguments the function takes, as well as names for the
    /// arguments, to be presented in the documentation (if applicable, e.g.
    /// :doc in the repl).
    pub args: [&'a CStr; N],
}

pub struct PrimOp {
    pub(crate) ptr: *mut raw::PrimOp,
}
impl Drop for PrimOp {
    fn drop(&mut self) {
        unsafe {
            raw::gc_decref(null_mut(), self.ptr as *mut c_void);
        }
    }
}
impl PrimOp {
    pub fn new<const N: usize>(
        eval_state: &mut EvalState,
        meta: PrimOpMeta<N>,
        f: Box<dyn Fn(&mut EvalState, &[Value; N]) -> Result<Value>>,
    ) -> Result<PrimOp> {
        assert!(N != 0);

        let mut args = Vec::new();
        for arg in meta.args {
            args.push(arg.as_ptr());
        }
        args.push(null());

        // Primops weren't meant to be dynamically created, as of writing.
        // This leaks, and so do the primop fields in Nix internally.
        let user_data = {
            // We'll be leaking this Box.
            // TODO: Use the GC with finalizer, if possible.
            let user_data = ManuallyDrop::new(Box::new(PrimOpContext {
                arity: N,
                function: Box::new(move |eval_state, args| {
                    let r = f(eval_state, args.try_into().unwrap());
                    r
                }),
                eval_state: eval_state.weak_ref(),
            }));
            user_data.as_ref() as *const PrimOpContext as *mut c_void
        };
        let op = unsafe {
            check_call!(raw::alloc_primop(
                &mut eval_state.context,
                FUNCTION_ADAPTER,
                N as c_int,
                meta.name.as_ptr(),
                args.as_mut_ptr(), /* TODO add an extra const to bindings to avoid mut here. */
                meta.doc.as_ptr(),
                user_data
            ))?
        };
        Ok(PrimOp { ptr: op })
    }
}

/// The user_data for our Nix primops
struct PrimOpContext {
    arity: usize,
    function: Box<dyn Fn(&mut EvalState, &[Value]) -> Result<Value>>,
    eval_state: EvalStateWeak,
}

unsafe extern "C" fn function_adapter(
    user_data: *mut ::std::os::raw::c_void,
    context_out: *mut raw::c_context,
    _state: *mut raw::EvalState,
    args: *mut *mut raw::Value,
    ret: *mut raw::Value,
) {
    let primop_info = (user_data as *const PrimOpContext).as_ref().unwrap();
    let mut eval_state = primop_info.eval_state.upgrade().unwrap_or_else(|| {
        panic!("Nix primop called after EvalState was dropped");
    });
    let args_raw_slice = unsafe { std::slice::from_raw_parts(args, primop_info.arity) };
    let args_vec: Vec<Value> = args_raw_slice
        .iter()
        .map(|v| Value::new_borrowed(*v))
        .collect();
    let args_slice = args_vec.as_slice();

    let r = primop_info.function.as_ref()(&mut eval_state, args_slice);

    match r {
        Ok(v) => unsafe {
            raw::copy_value(context_out, ret, v.raw_ptr());
        },
        Err(e) => unsafe {
            let cstr = CString::new(e.to_string()).unwrap_or_else(|_e| {
                CString::new("<rust nix-expr application error message contained null byte>")
                    .unwrap()
            });
            raw::set_err_msg(context_out, raw::err_NIX_ERR_UNKNOWN, cstr.as_ptr());
        },
    }
}

static FUNCTION_ADAPTER: raw::PrimOpFun = Some(function_adapter);
