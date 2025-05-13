//! Module for deferred computations, [`Thunk`].
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc};
use tokio::sync::{Mutex, OnceCell};

/// An impure thunk. Runs a future only once and remembers its result.
///
/// This is like a [std::sync::LazyCell] but for async code, using a `dyn` future.
pub struct Thunk<T> {
    thunk_state: Arc<ThunkState<T>>,
}
impl<T> Thunk<T> {
    /// Create a new thunk containing the given future.
    pub fn new<F>(future: F) -> Self
    where
        F: Future<Output = T> + 'static + Send,
    {
        Thunk {
            thunk_state: Arc::new(ThunkState::new(future)),
        }
    }

    /// Force the thunk. The inner future is polled to completion only on the first call.
    /// Subsequent calls return the cached value.
    pub async fn force(&self) -> &T {
        self.thunk_state.force().await
    }

    /// Consumes a `BTreeMap` of `Thunk`s, returning a `BTreeMap` of the results.
    ///
    /// The `Thunk` value is cloned, as the original `Thunk` may still be shared.
    pub async fn force_into_map<Key: Ord>(map: BTreeMap<Key, Thunk<T>>) -> BTreeMap<Key, T>
    where
        T: Clone,
    {
        let mut result_map: BTreeMap<Key, T> = BTreeMap::new();
        for (id, thunk) in map {
            result_map.insert(id, thunk.force().await.clone());
        }
        result_map
    }
}
impl<T> Clone for Thunk<T> {
    fn clone(&self) -> Self {
        Thunk {
            thunk_state: self.thunk_state.clone(),
        }
    }
}

struct ThunkState<T> {
    /// The future is executed only once, and the result is stored here
    cell: OnceCell<T>,
    /// The future to execute
    /// - Mutex so that we can own it mutably from the thread that ends up running for the `cell`
    /// - Option so that we can drop it after it is executed
    /// - Pin<Box<...>> so that we can use it as a dyn Future
    future: Mutex<Option<Pin<Box<dyn Future<Output = T> + Send + 'static>>>>,
}
impl<T> ThunkState<T> {
    fn new<F>(future: F) -> ThunkState<T>
    where
        F: Future<Output = T> + 'static + Send,
    {
        ThunkState {
            cell: OnceCell::new(),
            future: Mutex::new(Some(Box::pin(future))),
        }
    }
    async fn force(self: &Arc<Self>) -> &T {
        self.cell
            .get_or_init(|| async {
                let mut future = self.future.lock().await;
                match future.take() {
                    Some(future) => future.await,
                    None => panic!("Thunk was forced twice"),
                }
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    /// Tests that the `Thunk` struct correctly evaluates the future only once,
    /// caches the result, and returns the same value on subsequent calls.
    #[tokio::test]
    async fn thunk() {
        let has_run = Arc::new(AtomicBool::new(false));
        let t = Thunk::new(async move {
            if has_run.swap(true, std::sync::atomic::Ordering::SeqCst) {
                panic!("Thunk was forced twice");
            }
            42
        });

        let v1 = t.force().await;
        let v2 = t.force().await;
        assert_eq!(*v1, *v2);
    }

    /// Tests that the `Thunk` struct correctly evaluates the future only once,
    /// caches the result, and returns the same value on subsequent calls -
    /// even when the `Thunk` is cloned.
    #[tokio::test]
    async fn thunk_cloned() {
        let has_run = Arc::new(AtomicBool::new(false));
        let t = Thunk::new(async move {
            if has_run.swap(true, std::sync::atomic::Ordering::SeqCst) {
                panic!("Thunk was forced twice");
            }
            42
        });

        let t2 = t.clone();
        let v1 = t.force().await;
        let v2 = t2.force().await;
        assert_eq!(*v1, *v2);
    }
}
