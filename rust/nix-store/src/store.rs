use anyhow::{bail, Result};
use lazy_static::lazy_static;
use nix_c_raw as raw;
use nix_util::context::Context;
use nix_util::string_return::{callback_get_result_string, callback_get_result_string_data};
use nix_util::{check_call, result_string_init};
use std::ffi::{c_char, CString};
use std::ptr::null_mut;
use std::ptr::NonNull;
use std::sync::Arc;

/* TODO make Nix itself thread safe */
lazy_static! {
    static ref INIT: Result<()> = unsafe {
        check_call!(raw::libstore_init(&mut Context::new()))?;
        Ok(())
    };
}

struct StoreRef {
    inner: NonNull<raw::Store>,
}
impl StoreRef {
    pub fn ptr(&self) -> *mut raw::Store {
        self.inner.as_ptr()
    }
}
impl Drop for StoreRef {
    fn drop(&mut self) {
        unsafe {
            raw::store_free(self.inner.as_ptr());
        }
    }
}

pub struct Store {
    inner: Arc<StoreRef>,
    /* An error context to reuse. This way we don't have to allocate them for each store operation. */
    context: Context,
}
impl Store {
    pub fn open<'a, 'b>(
        url: &str,
        params: impl IntoIterator<Item = (&'a str, &'b str)>,
    ) -> Result<Self> {
        let x = INIT.as_ref();
        match x {
            Ok(_) => {}
            Err(e) => {
                // Couldn't just clone the error, so we have to print it here.
                bail!("nix_libstore_init error: {}", e);
            }
        }

        let mut context: Context = Context::new();

        let uri_ptr = CString::new(url)?;

        // this intermediate value must be here and must not be moved
        // because it owns the data the `*const c_char` pointers point to.
        let params: Vec<(CString, CString)> = params
            .into_iter()
            .map(|(k, v)| Ok((CString::new(k)?, CString::new(v)?))) // to do. context
            .collect::<Result<_>>()?;
        // this intermediate value owns the data the `*mut *const c_char` pointer points to.
        let mut params: Vec<_> = params
            .iter()
            .map(|(k, v)| [k.as_ptr(), v.as_ptr()])
            .collect();
        // this intermediate value owns the data the `*mut *mut *const c_char` pointer points to.
        let mut params: Vec<*mut *const c_char> = params
            .iter_mut()
            .map(|t| t.as_mut_ptr())
            .chain(std::iter::once(null_mut())) // signal the end of the array
            .collect();

        let store = unsafe {
            check_call!(raw::store_open(
                &mut context,
                uri_ptr.as_ptr(),
                params.as_mut_ptr()
            ))
        }?;
        if store.is_null() {
            panic!("nix_c_store_open returned a null pointer without an error");
        }
        let store = Store {
            inner: Arc::new(StoreRef {
                inner: NonNull::new(store).unwrap(),
            }),
            context,
        };
        Ok(store)
    }

    pub fn raw_ptr(&self) -> *mut raw::Store {
        self.inner.ptr()
    }

    pub fn get_uri(&mut self) -> Result<String> {
        let mut r = result_string_init!();
        unsafe {
            check_call!(raw::store_get_uri(
                &mut self.context,
                self.inner.ptr(),
                Some(callback_get_result_string),
                callback_get_result_string_data(&mut r)
            ))
        }?;
        r
    }
}

impl Clone for Store {
    fn clone(&self) -> Self {
        Store {
            inner: self.inner.clone(),
            context: Context::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn auto_works() {
        let res = Store::open("auto", HashMap::new());
        res.unwrap();
    }

    #[test]
    fn invalid_uri_fails() {
        let res = Store::open("invalid://uri", HashMap::new());
        assert!(res.is_err());
    }

    #[test]
    fn get_uri() {
        let mut store = Store::open("auto", HashMap::new()).unwrap();
        let uri = store.get_uri().unwrap();
        assert!(!uri.is_empty());
        // must be ascii
        assert!(uri.is_ascii());
        // usually something like "daemon", but that's not something we can check here.
        println!("uri: {}", uri);
    }

    #[test]
    #[ignore] // Needs network access
    fn get_uri_nixos_cache() {
        let mut store = Store::open("https://cache.nixos.org/", HashMap::new()).unwrap();
        let uri = store.get_uri().unwrap();
        assert_eq!(uri, "https://cache.nixos.org");
    }
}
