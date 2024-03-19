use anyhow::{bail, Result};
use lazy_static::lazy_static;
use nix_c_raw as raw;
use nix_util::context::Context;
use std::ffi::CString;
use std::mem::MaybeUninit;
use std::ptr::null_mut;
use std::ptr::NonNull;

/* TODO make Nix itself thread safe */
lazy_static! {
    static ref INIT: Result<()> = {
        unsafe {
            let context: Context = Context::new();
            raw::nix_libstore_init(context.ptr());
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
            raw::nix_store_free(self.inner.as_ptr());
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
            raw::nix_store_open(
                context.ptr(),
                uri_ptr.as_ptr(),
                null_mut::<*mut *const i8>(),
            )
        };
        context.check_err()?;
        if store.is_null() {
            bail!("nix_c_store_open returned a null pointer");
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
        const N: usize = 1024;
        let mut buffer: [MaybeUninit<u8>; N] = unsafe { MaybeUninit::uninit().assume_init() };
        unsafe {
            raw::nix_store_get_uri(
                self.context.ptr(),
                self.inner.ptr(),
                buffer.as_mut_ptr() as *mut i8,
                N as u32,
            )
        };
        self.context.check_err()?;
        unsafe {
            // copy the c string from buffer
            let cstr = core::ffi::CStr::from_ptr(buffer.as_ptr() as *const i8);
            cstr.to_str().map(|s| s.to_string()).map_err(|e| e.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_works() {
        let res = Store::open("auto");
        assert!(res.is_ok());
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
    fn get_uri_nixos_cache() {
        let store = Store::open("https://cache.nixos.org/").unwrap();
        let uri = store.get_uri().unwrap();
        assert_eq!(uri, "https://cache.nixos.org");
    }
}
