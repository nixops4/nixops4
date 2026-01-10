//! Async task scheduling with cycle detection and memoization.
//! See [`TaskTracker`] for more details.

use super::thunk::Thunk;
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
    sync::Arc,
};
use tokio::sync::Mutex;

/// `TaskWork` is implemented by the user to provide the work to be done.
/// The `work` method is called with a `TaskContext` that provides the capability
/// to spawn new tasks and add dependencies to the current task.
/// The `Key` type is used to identify tasks, and the `Output` type is the result
/// of the work done by the task.
#[async_trait::async_trait]
pub trait TaskWork {
    type Output: Send + Sync;
    type Key: Clone + Ord + std::fmt::Display + Send;
    // This could perhaps default to Cycle<Key> when associated type defaults are available in Rust
    type CycleError;

    /// Perform the work for the task indicated by `key`.
    ///
    /// The `context` parameter provides the capability to spawn new tasks and
    /// add dependencies to the current task.
    async fn work(&self, context: TaskContext<Self>, key: Self::Key) -> Self::Output;

    /// Convert a Cycle error into an error type of your choice.
    fn cycle_error(&self, cycle: Cycle<Self::Key>) -> Self::CycleError;
}

struct TaskState<Work: TaskWork + ?Sized> {
    result: Thunk<Work::Output>,
    dependencies: Vec<Work::Key>,
}

/// The mutable state of `TaskTracker`.
struct InnerState<Work: TaskWork + ?Sized> {
    tasks: BTreeMap<Work::Key, TaskState<Work>>,

    // This isn't mutated, so it could be moved to `TaskTracker`. It's only here
    // because it saves another Arc indirection.
    work_context: Arc<Work>,
}

/// Task scheduling with memoization and cycle detection.
///
/// It can be used for top-down dynamic programming, where each task represents
/// a subproblem, and tasks can spawn new tasks as dependencies.
///
/// Your implementation of the [`TaskWork`] trait is responsible for the actual
/// work. It uses the [`TaskContext`] provided to it to `spawn` or immediately
/// `require` new tasks.
pub struct TaskTracker<Work: TaskWork + ?Sized> {
    state: Arc<Mutex<InnerState<Work>>>,
}
impl<Work: TaskWork + Send + Sync + 'static> TaskTracker<Work>
where
    Work::Output: Clone + Send + Sync,
    Work::Key: Clone + Send + Sync,
{
    pub fn new(work_context: Arc<Work>) -> Self {
        TaskTracker {
            state: Arc::new(Mutex::new(InnerState {
                tasks: BTreeMap::new(),
                work_context,
            })),
        }
    }

    /// Create a new task for the given key. Work is not performed or started -
    /// instead, a `Thunk` is returned that can be used to force the task to run.
    pub async fn create(&self, key: Work::Key) -> Thunk<Work::Output> {
        // Look up the task
        let mut state = self.state.lock().await;
        let task = state.tasks.get(&key);
        if let Some(task) = task {
            // If the task is already running, return the result
            return task.result.clone();
        }
        // If the task is not found, create a new one
        let result = {
            let context = TaskContext {
                tracker: self.clone(),
                key: key.clone(),
            };
            let closure = state.work_context.clone();
            let key2 = key.clone();
            Thunk::new(async move { closure.work(context, key2).await })
        };
        let task = TaskState {
            // key: key.clone(),
            result,
            dependencies: Vec::new(),
        };
        state.tasks.insert(key.clone(), task);
        state.tasks.get(&key).unwrap().result.clone()
    }

    /// Run the task for the given key. This will block until the task completes.
    pub async fn run(&self, key: Work::Key) -> Work::Output {
        // Look up the task
        let thunk = self.create(key).await;
        // ^ closed the lock on tasks
        thunk.force().await.clone()
    }
}

// Derived `Clone` has unnecessary constraints, so we implement it manually
impl<Work> Clone for TaskTracker<Work>
where
    Work: TaskWork + ?Sized,
{
    fn clone(&self) -> Self {
        TaskTracker {
            state: self.state.clone(),
        }
    }
}

/// `TaskContext` is used while performing the work for a task.
/// It provides the capability to spawn new tasks and add dependencies to the current task.
/// The `TaskContext` is created by the [`TaskTracker`] and passed to the [`TaskWork::work`] method
pub struct TaskContext<Work: TaskWork + ?Sized> {
    tracker: TaskTracker<Work>,
    key: Work::Key,
}
impl<Work: TaskWork + ?Sized> Clone for TaskContext<Work> {
    fn clone(&self) -> Self {
        TaskContext {
            tracker: self.tracker.clone(),
            key: self.key.clone(),
        }
    }
}
impl<Work: TaskWork + Send + Sync + 'static> TaskContext<Work>
where
    Work::Output: Clone + Send + Sync,
    Work::Key: Clone + Send + Sync,
{
    async fn add_dependency(&self, key: Work::Key) -> Result<(), Work::CycleError> {
        let mut state = self.tracker.state.lock().await;

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
                std::slice::from_ref(&key),
            ) {
                path.reverse();
                Err(state.work_context.cycle_error(Cycle { path }))?;
            }

            let task = state.tasks.get_mut(&self.key).unwrap();

            task.dependencies.push(key);
            Ok(())
        } else {
            panic!("TaskContext: current task disappeared");
        }
    }

    pub async fn spawn(
        &self,
        key: Work::Key,
    ) -> Result<Thunk<<Work as TaskWork>::Output>, Work::CycleError> {
        let this = self.clone();

        this.add_dependency(key.clone()).await?;
        let thunk = self.tracker.create(key).await;
        let thunk_for_thread = thunk.clone();
        tokio::spawn(async move {
            thunk_for_thread.force().await;
        });
        Ok(thunk)
    }

    #[allow(clippy::type_complexity)]
    pub fn require(
        &self,
        key: Work::Key,
    ) -> Pin<Box<dyn Future<Output = Result<Work::Output, Work::CycleError>> + Send + '_>>
    where
        Work: 'static,
    {
        Box::pin(async move {
            self.add_dependency(key.clone()).await?;
            Ok(self.tracker.run(key).await)
        })
    }
}

// Naive cycle detection. It searches the whole graph without tracking anything
// clever across runs. This tends to be fast enough for our pretty small graphs.
// Most nodes are new nodes without any dependencies anyway, and we expect
// outgoing edges to be just one.
// I haven't analyzed this in practice, but certainly for now, the graphs are
// small enough.
fn find_path_to<Work: TaskWork>(
    seen: &mut BTreeSet<<Work as TaskWork>::Key>,
    tasks: &BTreeMap<Work::Key, TaskState<Work>>,
    needle: &<Work as TaskWork>::Key,
    outgoing: &[<Work as TaskWork>::Key],
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

/// A type that represents a cycle in the task graph, implementing `std::fmt::Display`
/// so that it can be printed in a human-readable format.
/// The cycle is represented as a vector of keys, where the first element depends on the second element, and so forth.
/// The last element depends on the first element, but the first element is not repeated in `path()`.
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
        assert!(
            !self.path.is_empty(),
            "Cycle must have at least one element"
        );
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

    // Fibonacci function has empty closure, so the `Fibonacci` value (a TaskWork) is also empty.
    struct Fibonacci {}
    #[async_trait]
    impl TaskWork for Fibonacci {
        type Output = u128;
        type Key = u64;
        type CycleError = Cycle<Self::Key>;

        async fn work(&self, context: TaskContext<Self>, key: Self::Key) -> Self::Output {
            match key {
                0 => 0,
                1 => 1,
                _ => {
                    let a = context.require(key - 1).await.unwrap();
                    let b = context.require(key - 2).await.unwrap();
                    a + b
                }
            }
        }

        fn cycle_error(&self, cycle: Cycle<Self::Key>) -> Cycle<Self::Key> {
            cycle
        }
    }

    #[tokio::test]
    async fn test_fibonacci() {
        let fib = TaskTracker::new(Arc::new(Fibonacci {}));
        assert_eq!(fib.run(0).await, 0);
        assert_eq!(fib.run(1).await, 1);
        assert_eq!(fib.run(2).await, 1);
        assert_eq!(fib.run(3).await, 2);
        assert_eq!(fib.run(4).await, 3);
        assert_eq!(fib.run(5).await, 5);

        assert_eq!(fib.run(20).await, 6765);

        // u128 max:                           340282366920938463463374607431768211455u128
        assert_eq!(
            fib.run(186).await,
            332825110087067562321196029789634457848u128
        );
    }

    struct Cyclic {
        modulo: u64,
    }
    #[async_trait]
    impl TaskWork for Cyclic {
        type Output = Result<u64, Cycle<u64>>;
        type Key = u64;
        type CycleError = Cycle<Self::Key>;

        async fn work(&self, context: TaskContext<Self>, key: Self::Key) -> Self::Output {
            context.require((key + 1) % self.modulo).await?
        }

        fn cycle_error(&self, cycle: Cycle<Self::Key>) -> Cycle<Self::Key> {
            cycle
        }
    }

    #[tokio::test]
    async fn test_cyclic() {
        let tasks = TaskTracker::new(Arc::new(Cyclic { modulo: 10 }));
        let r = tasks.run(0).await;
        let e = r.unwrap_err();
        let expected: Vec<u64> = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9];
        assert_eq!(e.path(), &expected);
    }
}
