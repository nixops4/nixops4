use anyhow::{bail, Result};
use lazy_static::lazy_static;
use nix_c_raw as raw;
use nix_util::context::Context;
use nix_util::string_return::callback_get_vec_u8;
use std::ffi::CString;
use std::ptr::null_mut;
use std::ptr::NonNull;

/* TODO make Nix itself thread safe */
lazy_static! {
    static ref INIT: Result<()> = {
        unsafe {
            let context: Context = Context::new();
            raw::libstore_init(context.ptr());
            context.check_err()
        }
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
    inner: StoreRef,
    /* An error context to reuse. This way we don't have to allocate them for each store operation. */
    context: Context,
}
impl Store {
    pub fn open(url: &str) -> Result<Self> {
        let x = INIT.as_ref();
        match x {
            Ok(_) => {}
            Err(e) => {
                // Couldn't just clone the error, so we have to print it here.
                bail!("nix_libstore_init error: {}", e);
            }
        }

        let context: Context = Context::new();

        let uri_ptr = CString::new(url)?;
        let store = unsafe {
            raw::store_open(
                context.ptr(),
                uri_ptr.as_ptr(),
                null_mut::<*mut *const i8>(),
            )
        };
        context.check_err()?;
        if store.is_null() {
            panic!("nix_c_store_open returned a null pointer without an error");
        }
        let store = Store {
            inner: StoreRef {
                inner: NonNull::new(store).unwrap(),
            },
            context,
        };
        Ok(store)
    }

    pub fn raw_ptr(&self) -> *mut raw::Store {
        self.inner.ptr()
    }

    pub fn get_uri(&self) -> Result<String> {
        let mut raw_buffer: Vec<u8> = Vec::new();
        unsafe {
            raw::store_get_uri(
                self.context.ptr(),
                self.inner.ptr(),
                Some(callback_get_vec_u8),
                &mut raw_buffer as *mut Vec<u8> as *mut std::ffi::c_void,
            )
        };
        self.context.check_err()?;
        String::from_utf8(raw_buffer).map_err(|e| e.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_works() {
        let res = Store::open("auto");
        res.unwrap();
    }

    #[test]
    fn invalid_uri_fails() {
        let res = Store::open("invalid://uri");
        assert!(res.is_err());
    }

    #[test]
    fn get_uri() {
        let store = Store::open("auto").unwrap();
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
        let store = Store::open("https://cache.nixos.org/").unwrap();
        let uri = store.get_uri().unwrap();
        assert_eq!(uri, "https://cache.nixos.org");
    }
}
