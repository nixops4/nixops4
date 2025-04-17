//! Bob is a ~~builder.~~ scheduling system that provides async task scheduling with cycle detection.

use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
    sync::Arc,
};
use tokio::sync::Mutex;

/// A lazy thunk that runs a future only once and caches its result.
pub struct Thunk<T> {
    // Once forced, value holds Some(result).
    value: Mutex<Option<Arc<T>>>,
    // The future is kept until evaluation; then we replace it with None.
    future: Mutex<Option<Pin<Box<dyn Future<Output = T> + Send>>>>,
}

impl<T> Thunk<T> {
    pub fn new<F>(future: F) -> Arc<Self>
    where
        F: Future<Output = T> + 'static + Send,
    {
        Arc::new(Thunk {
            value: Mutex::new(None),
            future: Mutex::new(Some(Box::pin(future))),
        })
    }

    /// Force the thunk. The inner future is polled to completion only on the first call.
    /// Subsequent calls return the cached value.
    pub async fn force(self: Arc<Self>) -> Arc<T> {
        // Check cache first.
        if let Some(val) = self.value.lock().await.as_ref() {
            return val.clone();
        }

        // Take the future out – the future must be polled exactly once.
        let fut = self.future.lock().await.take();
        match fut {
            // We got the lock first and it's our responsibility to await the future.
            Some(fut) => {
                let mut l = self.value.lock().await;
                let r = Arc::new(fut.await);
                l.replace(r.clone());
                r
            }
            // The future should have been forced by another thread.
            None => {
                // Get it from the cache
                let l = self.value.lock().await;
                let val = l.as_ref();
                match val {
                    Some(val) => return val.clone(),
                    None => panic!(
                        "Thunk was not forced successfully and future is gone. Can't continue."
                    ),
                }
            }
        }
    }
}

#[async_trait::async_trait]
pub trait BobClosure {
    type Output: Send + Sync;
    type Key: Clone + Ord + std::fmt::Display + Send;

    async fn work(&self, context: BobContext<Self>, key: Self::Key) -> Self::Output;
}

pub struct BobTask<Work: BobClosure + ?Sized> {
    // key: Work::Key,
    result: Arc<Thunk<Work::Output>>,
    dependencies: Vec<Work::Key>,
}

struct InnerState<Work: BobClosure + ?Sized> {
    tasks: BTreeMap<Work::Key, BobTask<Work>>,
}
impl<Work: BobClosure + ?Sized> InnerState<Work> {
    fn new() -> Self {
        InnerState {
            tasks: BTreeMap::new(),
        }
    }
}

pub struct BobState<Work: BobClosure + ?Sized> {
    state: Arc<Mutex<InnerState<Work>>>,
    closure: Arc<Box<Work>>,
}
impl<Work: BobClosure + Send + Sync + 'static> BobState<Work> {
    pub fn new_arc(closure: Arc<Box<Work>>) -> Arc<Self> {
        Arc::new(BobState {
            state: Arc::new(Mutex::new(InnerState::new())),
            closure,
        })
    }
    #[cfg(test)]
    pub fn new(closure: Work) -> Arc<Self> {
        BobState::new_arc(Arc::new(Box::new(closure)))
    }

    pub async fn create(self: &Arc<Self>, key: Work::Key) -> Arc<Thunk<Work::Output>> {
        // Look up the task
        let mut state = self.state.lock().await;
        let task = state.tasks.get(&key);
        if let Some(task) = task {
            // If the task is already running, return the result
            return task.result.clone();
        }
        // If the task is not found, create a new one
        let result = {
            let context = BobContext {
                bob: self.clone(),
                key: key.clone(),
            };
            let closure = self.closure.clone();
            let key2 = key.clone();
            Thunk::new(async move { closure.work(context, key2).await })
        };
        let task = BobTask {
            // key: key.clone(),
            result,
            dependencies: Vec::new(),
        };
        state.tasks.insert(key.clone(), task);
        state.tasks.get(&key).unwrap().result.clone()
    }

    pub async fn run(self: &Arc<Self>, key: Work::Key) -> Arc<Work::Output> {
        // Look up the task
        let thunk = self.create(key).await;
        // ^ closed the lock on tasks
        thunk.force().await
    }
}

pub struct BobContext<Work: BobClosure + ?Sized> {
    // tasks: Arc<Mutex<BTreeMap<Work::Key, BobTask<Work>>>>,
    bob: Arc<BobState<Work>>,
    key: Work::Key,
}
impl<Work: BobClosure + ?Sized> Clone for BobContext<Work> {
    fn clone(&self) -> Self {
        BobContext {
            bob: self.bob.clone(),
            key: self.key.clone(),
        }
    }
}
impl<'a, Work: BobClosure + Send + Sync + 'static> BobContext<Work> {
    pub fn closure(&self) -> &Work {
        &self.bob.closure
    }
    async fn add_dependency(&self, key: Work::Key) -> Result<(), Cycle<Work::Key>> {
        let mut state = self.bob.state.lock().await;

        let task = state.tasks.get(&self.key);

        if let Some(task) = task {
            if task.dependencies.contains(&key) {
                // Already a dependency; nothing to do
                return Ok(());
            }

            if let Some(mut path) = find_path_to(
                &mut BTreeSet::new(),
                &state.tasks,
                &self.key,
                &[key.clone()],
            ) {
                path.reverse();
                Err(Cycle { path })?;
            }

            let task = state.tasks.get_mut(&self.key).unwrap();

            task.dependencies.push(key);
            Ok(())
        } else {
            panic!("Bob: current task disappeared");
        }
    }

    pub async fn spawn(
        &self,
        key: Work::Key,
    ) -> Result<Arc<Thunk<<Work as BobClosure>::Output>>, Cycle<Work::Key>> {
        let bob_clone = self.bob.clone();
        let this = self.clone();

        this.add_dependency(key.clone()).await?;
        let r = bob_clone.create(key).await;
        let r_2 = r.clone();
        tokio::spawn(r_2.force());
        Ok(r)
    }

    pub fn require(
        &self,
        key: Work::Key,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<Work::Output>, Cycle<Work::Key>>> + Send + '_>>
    where
        Work: 'static,
        Work::Output: Sync + Send,
        Work::Key: Sync + Send,
    {
        let bob_clone = self.bob.clone();
        let this = self.clone();
        Box::pin(async move {
            this.add_dependency(key.clone()).await?;
            let r = bob_clone.run(key).await;
            Ok(r)
        })
    }
}

// Naive cycle detection
fn find_path_to<Work: BobClosure>(
    seen: &mut BTreeSet<<Work as BobClosure>::Key>,
    tasks: &BTreeMap<Work::Key, BobTask<Work>>,
    needle: &<Work as BobClosure>::Key,
    outgoing: &[<Work as BobClosure>::Key],
) -> Option<Vec<Work::Key>> {
    // First check if the key is in the dependencies
    if outgoing.contains(needle) {
        return Some(vec![needle.clone()]);
    }
    // Maybe a longer path exists
    for edge in outgoing {
        if seen.contains(edge) {
            continue;
        }
        seen.insert(edge.clone());

        let node = tasks.get(edge);
        if let Some(node) = node {
            if let Some(mut path) = find_path_to(seen, tasks, needle, node.dependencies.as_slice())
            {
                path.push(edge.clone());
                return Some(path);
            }
        }
    }
    None
}

pub struct Cycle<Key> {
    path: Vec<Key>,
}
impl<Key: Clone> Clone for Cycle<Key> {
    fn clone(&self) -> Self {
        Self {
            path: self.path.clone(),
        }
    }
}
impl<Key> Cycle<Key> {
    /// The path that forms a cycle, such that the first element depended on the second element, and so forth.
    /// The last element depends on the first element, but the first element is not repeated in `path()`.
    pub fn path(&self) -> &Vec<Key> {
        &self.path
    }
    fn check(&self) {
        assert!(self.path.len() > 0, "Cycle must have at least one element");
    }
}
impl<Key: std::fmt::Display> std::fmt::Display for Cycle<Key> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.check();
        let mut s = String::new();
        self.path().iter().for_each(|k| {
            s.push_str(&format!("{} -> \n", k));
        });
        s.push_str(&format!("{}", self.path.first().unwrap()));
        write!(f, "{}", s)
    }
}
impl<Key: std::fmt::Debug> std::fmt::Debug for Cycle<Key> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.check();
        let mut s = String::new();
        self.path().iter().for_each(|k| {
            s.push_str(&format!("{:?} -> ", k));
        });
        s.push_str(&format!("{:?}", self.path.first().unwrap()));
        write!(f, "{}", s)
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;

    // Fibonacci function has empty closure, so the BobClosure is also empty.
    struct Fibonacci {}
    #[async_trait]
    impl BobClosure for Fibonacci {
        type Output = u128;

        type Key = u64;

        async fn work(&self, context: BobContext<Self>, key: Self::Key) -> Self::Output {
            match key {
                0 => 0,
                1 => 1,
                _ => {
                    let a = context.require(key - 1).await.unwrap();
                    let b = context.require(key - 2).await.unwrap();
                    *a + *b
                }
            }
        }
    }

    lazy_static::lazy_static! {
        static ref RUNTIME: tokio::runtime::Runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
    }

    fn rt_block_on<Fut: Future>(fut: Fut) -> Fut::Output {
        RUNTIME.block_on(fut)
    }

    #[test]
    fn test_fibonacci() {
        let bob = BobState::new(Fibonacci {});
        assert_eq!(*rt_block_on(bob.run(0)), 0);
        assert_eq!(*rt_block_on(bob.run(1)), 1);
        assert_eq!(*rt_block_on(bob.run(2)), 1);
        assert_eq!(*rt_block_on(bob.run(3)), 2);
        assert_eq!(*rt_block_on(bob.run(4)), 3);
        assert_eq!(*rt_block_on(bob.run(5)), 5);

        // assert_eq!(*rt_block_on(bob.run(20)), 6765);
        // assert_eq!(*rt_block_on(bob.run(30)), 832040);
        // assert_eq!(*rt_block_on(bob.run(40)), 102334155);
        // assert_eq!(*rt_block_on(bob.run(90)), 2880067194370816120u128);

        // u128 max:                           340282366920938463463374607431768211455u128
        assert_eq!(
            *rt_block_on(bob.run(186)),
            332825110087067562321196029789634457848u128
        );
    }

    struct Cyclic {
        modulo: u64,
    }
    #[async_trait]
    impl BobClosure for Cyclic {
        type Output = Result<u64, Cycle<u64>>;

        type Key = u64;

        async fn work(&self, context: BobContext<Self>, key: Self::Key) -> Self::Output {
            let r = context.require((key + 1) % self.modulo).await?;
            let r = r.as_ref();
            match r {
                Ok(v) => Ok(*v),
                Err(cycle) => Err(cycle.clone()),
            }
        }
    }

    #[test]
    fn test_cyclic() {
        let bob = BobState::new(Cyclic { modulo: 10 });
        let r = rt_block_on(bob.run(0));
        let e = r.as_ref().clone().unwrap_err();
        let expected: Vec<u64> = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        assert_eq!(e.path(), &expected);
    }
}
