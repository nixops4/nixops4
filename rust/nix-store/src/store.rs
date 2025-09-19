use anyhow::{bail, Error, Result};
use lazy_static::lazy_static;
use nix_c_raw as raw;
use nix_util::context::Context;
use nix_util::string_return::{callback_get_result_string, callback_get_result_string_data};
use nix_util::{check_call, result_string_init};
use std::collections::HashMap;
use std::ffi::{c_char, CString};
use std::ptr::null_mut;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, Weak};

use crate::path::StorePath;

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
    /// # Safety
    ///
    /// The returned pointer is only valid as long as the `StoreRef` is alive.
    pub unsafe fn ptr(&self) -> *mut raw::Store {
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
unsafe impl Send for StoreRef {}
/// Unlike pointers in general, operations on raw::Store are thread safe and it is therefore safe to share them between threads.
unsafe impl Sync for StoreRef {}

/// A [Weak] reference to a store.
pub struct StoreWeak {
    inner: Weak<StoreRef>,
}
impl StoreWeak {
    /// Upgrade the weak reference to a proper [Store].
    ///
    /// If no normal reference to the [Store] is around anymore elsewhere, this fails by returning `None`.
    pub fn upgrade(&self) -> Option<Store> {
        self.inner.upgrade().map(|inner| Store {
            inner,
            context: Context::new(),
        })
    }
}

/// Protects against https://github.com/NixOS/nix/issues/11979 (unless different parameters are passed, in which case it's up to luck, but you do get your own parameters as you asked for).
type StoreCacheMap = HashMap<(Option<String>, Vec<(String, String)>), StoreWeak>;

lazy_static! {
    static ref STORE_CACHE: Arc<Mutex<StoreCacheMap>> = Arc::new(Mutex::new(HashMap::new()));
}

unsafe extern "C" fn callback_get_result_store_path_set(
    user_data: *mut std::os::raw::c_void,
    store_path: *const raw::StorePath,
) {
    let ret = user_data as *mut Vec<StorePath>;
    let ret: &mut Vec<StorePath> = &mut *ret;

    let store_path = raw::store_path_clone(store_path);

    let store_path =
        NonNull::new(store_path).expect("nix_store_parse_path returned a null pointer");
    let store_path = StorePath::new_raw(store_path);
    ret.push(store_path);
}

fn callback_get_result_store_path_set_data(vec: &mut Vec<StorePath>) -> *mut std::os::raw::c_void {
    vec as *mut Vec<StorePath> as *mut std::os::raw::c_void
}

pub struct Store {
    inner: Arc<StoreRef>,
    /* An error context to reuse. This way we don't have to allocate them for each store operation. */
    context: Context,
}
impl Store {
    /// Open a store.
    ///
    /// See [nix_c_raw::store_open] for more information.
    #[doc(alias = "nix_store_open")]
    pub fn open<'a, 'b>(
        url: Option<&str>,
        params: impl IntoIterator<Item = (&'a str, &'b str)>,
    ) -> Result<Self> {
        let params = params
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect::<Vec<(String, String)>>();
        let params2 = params.clone();
        let mut store_cache = STORE_CACHE
            .lock()
            .map_err(|_| Error::msg("Failed to lock store cache. This should never happen."))?;
        match store_cache.entry((url.map(Into::into), params)) {
            std::collections::hash_map::Entry::Occupied(mut e) => {
                if let Some(store) = e.get().upgrade() {
                    Ok(store)
                } else {
                    let store = Self::open_uncached(
                        url,
                        params2.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                    )?;
                    e.insert(store.weak_ref());
                    Ok(store)
                }
            }
            std::collections::hash_map::Entry::Vacant(e) => {
                let store = Self::open_uncached(
                    url,
                    params2.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                )?;
                e.insert(store.weak_ref());
                Ok(store)
            }
        }
    }
    fn open_uncached<'a, 'b>(
        url: Option<&str>,
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

        let uri_cstring = match url {
            Some(url) => Some(CString::new(url)?),
            None => None,
        };
        let uri_ptr = uri_cstring
            .as_ref()
            .map(|s| s.as_ptr())
            .unwrap_or(null_mut());

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

        let store =
            unsafe { check_call!(raw::store_open(&mut context, uri_ptr, params.as_mut_ptr())) }?;
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

    /// # Safety
    ///
    /// The returned pointer is only valid as long as the `Store` is alive.
    pub unsafe fn raw_ptr(&self) -> *mut raw::Store {
        self.inner.ptr()
    }

    #[doc(alias = "nix_store_get_uri")]
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

    #[cfg(nix_at_least = "2.26")]
    #[doc(alias = "nix_store_get_storedir")]
    pub fn get_storedir(&mut self) -> Result<String> {
        let mut r = result_string_init!();
        unsafe {
            check_call!(raw::store_get_storedir(
                &mut self.context,
                self.inner.ptr(),
                Some(callback_get_result_string),
                callback_get_result_string_data(&mut r)
            ))
        }?;
        r
    }

    #[doc(alias = "nix_store_parse_path")]
    pub fn parse_store_path(&mut self, path: &str) -> Result<StorePath> {
        let path = CString::new(path)?;
        unsafe {
            let store_path = check_call!(raw::store_parse_path(
                &mut self.context,
                self.inner.ptr(),
                path.as_ptr()
            ))?;
            let store_path =
                NonNull::new(store_path).expect("nix_store_parse_path returned a null pointer");
            Ok(StorePath::new_raw(store_path))
        }
    }

    #[doc(alias = "nix_store_get_fs_closure")]
    pub fn get_fs_closure(
        &mut self,
        store_path: &StorePath,
        flip_direction: bool,
        include_outputs: bool,
        include_derivers: bool,
    ) -> Result<Vec<StorePath>> {
        let mut r = Vec::new();
        unsafe {
            check_call!(raw::store_get_fs_closure(
                &mut self.context,
                self.inner.ptr(),
                store_path.as_ptr(),
                flip_direction,
                include_outputs,
                include_derivers,
                callback_get_result_store_path_set_data(&mut r),
                Some(callback_get_result_store_path_set)
            ))
        }?;
        Ok(r)
    }

    pub fn weak_ref(&self) -> StoreWeak {
        StoreWeak {
            inner: Arc::downgrade(&self.inner),
        }
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
    fn none_works() {
        let res = Store::open(None, HashMap::new());
        res.unwrap();
    }

    #[test]
    fn auto_works() {
        // This is not actually a given.
        // Maybe whatever is in NIX_REMOTE or nix.conf is really important.
        let res = Store::open(Some("auto"), HashMap::new());
        res.unwrap();
    }

    #[test]
    fn invalid_uri_fails() {
        let res = Store::open(Some("invalid://uri"), HashMap::new());
        assert!(res.is_err());
    }

    #[test]
    fn get_uri() {
        let mut store = Store::open(None, HashMap::new()).unwrap();
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
        let mut store = Store::open(Some("https://cache.nixos.org/"), HashMap::new()).unwrap();
        let uri = store.get_uri().unwrap();
        assert_eq!(uri, "https://cache.nixos.org");
    }

    #[test]
    #[cfg(nix_at_least = "2.26" /* get_storedir */)]
    fn parse_store_path_ok() {
        let mut store = crate::store::Store::open(Some("dummy://"), []).unwrap();
        let store_dir = store.get_storedir().unwrap();
        let store_path_string =
            format!("{store_dir}/rdd4pnr4x9rqc9wgbibhngv217w2xvxl-bash-interactive-5.2p26");
        let store_path = store.parse_store_path(store_path_string.as_str()).unwrap();
        assert_eq!(store_path.name().unwrap(), "bash-interactive-5.2p26");
    }

    #[test]
    fn parse_store_path_fail() {
        let mut store = crate::store::Store::open(Some("dummy://"), []).unwrap();
        let store_path_string = format!("bash-interactive-5.2p26");
        let r = store.parse_store_path(store_path_string.as_str());
        match r {
            Err(e) => {
                assert!(e.to_string().contains("bash-interactive-5.2p26"));
            }
            _ => panic!("Expected error"),
        }
    }

    #[test]
    fn weak_ref() {
        let mut store = Store::open(None, HashMap::new()).unwrap();
        let uri = store.get_uri().unwrap();
        let weak = store.weak_ref();
        let mut store2 = weak.upgrade().unwrap();
        assert_eq!(store2.get_uri().unwrap(), uri);
    }
    #[test]
    fn weak_ref_gone() {
        let weak = {
            // Concurrent tests calling Store::open will keep the weak reference to auto alive,
            // so for this test we need to bypass the global cache.
            let store = Store::open_uncached(None, HashMap::new()).unwrap();
            store.weak_ref()
        };
        assert!(weak.upgrade().is_none());
        assert!(weak.inner.upgrade().is_none());
    }
}
