use anyhow::{bail, Result};
use chrono::Utc;
use serde_json::{de::IoRead, Deserializer, StreamDeserializer};
use std::{
    fs::{File, OpenOptions},
    io::{self, Seek as _, Write},
    path::Path,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct StateEvent {
    pub index: u64,
    pub meta: StateEventMeta,
    pub patch: json_patch::Patch,
    // #[serde(flatten)]
    // unknown_fields: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct StateEventMeta {
    pub time: String,
    #[serde(flatten)]
    pub other_fields: serde_json::Value,
}

pub struct StateEventStream<'a, R: io::Read> {
    iter: StreamDeserializer<'a, IoRead<R>, StateEvent>,
    /// Save the validated first event for processing by our caller
    /// (basically prepend it to the iterator)
    first_event: Option<StateEvent>,
}
impl<'a, R: io::Read> StateEventStream<'a, R> {
    pub fn open_from_reader(reader: R) -> Result<StateEventStream<'a, R>> {
        let deserializer = Deserializer::from_reader(reader);
        let mut iter = deserializer.into_iter();
        let first_event = match iter.next() {
            Some(Ok(ev @ StateEvent { index, .. })) => {
                if index != 0 {
                    bail!("Expected initial state event with index 0, got {}", index);
                }
                ev
            }
            Some(Err(e)) => bail!(
                "State file invalid: error parsing initial state event: {}",
                e
            ),
            None => bail!("State file invalid: no initial state event"),
        };
        Ok(StateEventStream {
            iter,
            first_event: Some(first_event),
        })
    }
}

// TODO: structured logging for providers

/// Monitor a task which may take a long time, and write messages to the console
/// as needed.
/// After SILENT_INTERVAL: print activity
/// After LOG_INTERVAL: print activity and duration
pub struct WaitMonitor {
    done: Arc<AtomicBool>,
}
impl WaitMonitor {
    const SILENT_INTERVAL: Duration = Duration::from_millis(500);
    const LOG_INTERVAL: Duration = Duration::from_secs(5);

    pub fn new(activity: String) -> WaitMonitor {
        Self::new_by_ref(Arc::new(AtomicBool::new(false)), activity)
    }
    pub fn new_by_ref(done: Arc<AtomicBool>, activity: String) -> WaitMonitor {
        // Start thread
        let r = WaitMonitor { done: done.clone() };
        std::thread::spawn(|| {
            WaitMonitor::run(done, activity);
        });
        r
    }
    fn run(done: Arc<AtomicBool>, activity: String) {
        let start = std::time::Instant::now();
        let mut next_log = start.checked_add(Self::SILENT_INTERVAL).unwrap();
        loop {
            if done.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            let now = std::time::Instant::now();
            let next_log_interval = next_log.duration_since(now);
            std::thread::sleep(next_log_interval);
            if done.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            if next_log.duration_since(start) < Self::SILENT_INTERVAL {
                eprintln!("{}", activity);
                // Reset before adding the large scale increment
                next_log = start;
            } else {
                // Current time is next_log (or very slightly past)
                eprintln!(
                    "{} ({} s)",
                    activity,
                    next_log.duration_since(start).as_secs()
                );
            }
            next_log = next_log.checked_add(Self::LOG_INTERVAL).unwrap();
        }
    }
    pub fn done(&self) {
        self.done.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
impl Drop for WaitMonitor {
    fn drop(&mut self) {
        self.done()
    }
}

pub struct StateHandle {
    file: Arc<File>,
    locking: fd_lock::RwLock<Arc<File>>,
    expected_size: Option<u64>,
}
impl StateHandle {
    pub fn open<P: AsRef<Path>>(name: P, create_new: bool) -> Result<StateHandle> {
        let file = OpenOptions::new()
            .read(true)
            .append(true)
            .create_new(create_new)
            .open(name)?;
        let file = Arc::new(file);
        let locking = fd_lock::RwLock::new(file.clone());
        let mut handle = StateHandle {
            file,
            locking,
            expected_size: None,
        };
        if create_new {
            handle.append(&[&Self::init_event()])?;
        }
        Ok(handle)
    }
    fn init_event() -> StateEvent {
        let now = Utc::now();
        let iso_datetime = now.to_rfc3339();
        StateEvent {
            index: 0,
            meta: StateEventMeta {
                time: iso_datetime,
                other_fields: serde_json::json!({}),
            },
            patch: json_patch::Patch(vec![json_patch::PatchOperation::Add(
                json_patch::AddOperation {
                    path: "".parse().expect("empty path"),
                    value: serde_json::json!({
                        "_type": "nixopsState",
                        "resources": {},
                        "deployments": {},
                    }),
                },
            )]),
        }
    }
    fn lock_write(
        locking: &mut fd_lock::RwLock<Arc<File>>,
    ) -> Result<fd_lock::RwLockWriteGuard<Arc<File>>> {
        let lock_wait_mon = WaitMonitor::new("Waiting for state file write lock".to_owned());
        let lock = locking.write()?;
        lock_wait_mon.done();
        Ok(lock)
    }
    pub fn append(&mut self, event: &[&StateEvent]) -> Result<()> {
        let lock_guard = Self::lock_write(&mut self.locking)?;
        let pos = self.file.seek(io::SeekFrom::End(0))?;
        match self.expected_size {
            None => self.expected_size = Some(pos),
            Some(expected_size) => {
                if pos != expected_size {
                    eprintln!(
                        "Detected concurrent writing. Foreign bytes: [{}..{})",
                        expected_size, pos
                    );
                    eprintln!("CRITICAL: concurrent state manipulation may require manual intervention to avoid data loss, orphaned infrastructure and cloud cost overruns");
                }
            }
        }
        let mut writer = io::BufWriter::new(self.file.clone());

        for event in event {
            // We prettify to make it more human readable. Potentially slightly
            // harder to parse by other tools, but worth the tradeoff.
            // In contrast, the provider protocol messages are entirely ephemeral
            // and only ever read by developers.
            serde_json::to_writer_pretty(&mut writer, event)?;
            writer.write_all(b"\n")?;
        }
        writer.flush()?;

        self.expected_size = Some(self.file.stream_position()?);

        drop(lock_guard);
        Ok(())
    }
}

impl<'a, R: io::Read> Iterator for StateEventStream<'a, R> {
    type Item = Result<StateEvent>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.first_event.is_some() {
            self.first_event.take().map(Ok)
        } else {
            self.iter.next().map(|r| r.map_err(Into::into))
        }
    }
}

pub fn apply_state_event(state: &mut serde_json::Value, event: &StateEvent) -> Result<()> {
    json_patch::patch(state, event.patch.0.as_slice()).map_err(Into::into)
}

pub fn apply_state_events(
    state: &mut serde_json::Value,
    events: impl Iterator<Item = Result<StateEvent>>,
) -> Result<()> {
    for event in events {
        apply_state_event(state, &event?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use io::{stdout, Read};

    use super::*;

    const BASIC_EXAMPLE: &str = r#"
    {
        "index": 0,
        "meta": {"time":"2019-03-04T07:40:00Z"},
        "patch": [
            {
                "op": "add",
                "value": { "_type": "nixopsState", "resources": {}, "deployments": {} },
                "path": ""
            }
        ]
    }
    {
        "index": 1,
        "meta": {"time":"2019-03-04T07:41:00Z"},
        "patch": [
            {
                "op": "add",
                "value": {
                    "type": "file",
                    "inputProperties": {
                        "contents": "Hi there"
                    },
                    "outputProperties": { }
                },
                "path": "/resources/a"
            }
        ]
    }
"#;

    #[test]
    fn test_open_state_stream() {
        let stream = StateEventStream::open_from_reader(BASIC_EXAMPLE.as_bytes()).unwrap();
        let events: Vec<_> = stream.collect();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].as_ref().unwrap().index, 0);
        assert_eq!(
            events[0].as_ref().unwrap().meta.time,
            "2019-03-04T07:40:00Z"
        );
    }

    #[test]
    fn test_resolve_state() {
        let stream = StateEventStream::open_from_reader(BASIC_EXAMPLE.as_bytes()).unwrap();
        let mut state = serde_json::json!({});
        apply_state_events(&mut state, stream).unwrap();
        assert_eq!(
            state,
            serde_json::json!({
                "_type": "nixopsState",
                "resources": {
                    "a": {
                        "type": "file",
                        "inputProperties": {
                            "contents": "Hi there"
                        },
                        "outputProperties": { }
                    }
                },
                "deployments": { }
            })
        );
    }

    #[test]
    fn test_open_state_stream_invalid_index() {
        let input = r#"{"index":1,"meta":{"time":"2019-06-04T07:40:00Z"},"patch":[]}"#;
        let stream = StateEventStream::open_from_reader(input.as_bytes());
        assert!(stream.is_err());
    }

    #[test]
    fn test_open_state_stream_no_index() {
        let input = r#"{"meta":{"time":"2019-06-04T07:40:00Z"},"patch":[]}"#;
        let stream = StateEventStream::open_from_reader(input.as_bytes());
        assert!(stream.is_err());
    }

    #[test]
    fn test_open_state_stream_invalid_json() {
        let input = r#"{"index":0,"meta":{"time":"2019-06-04T07:40:00Z"},"patch":[]"#;
        let stream = StateEventStream::open_from_reader(input.as_bytes());
        assert!(stream.is_err());
    }

    #[test]
    fn test_open_state_stream_empty() {
        let input = r#""#;
        let stream = StateEventStream::open_from_reader(input.as_bytes());
        assert!(stream.is_err());
    }

    #[test]
    fn test_open_state_stream_empty_array() {
        let input = r#"[]"#;
        let stream = StateEventStream::open_from_reader(input.as_bytes());
        assert!(stream.is_err());
    }

    #[test]
    fn test_open_state_stream_no_patch() {
        let input = r#"{"index":0,"meta":{"time":"2019-06-04T07:40:00Z"}}"#;
        let stream = StateEventStream::open_from_reader(input.as_bytes());
        assert!(stream.is_err());
    }

    #[test]
    fn test_open_state_stream_invalid_patch() {
        let input = r#"{"index":0,"patch":[{}]}"#;
        let stream = StateEventStream::open_from_reader(input.as_bytes());
        assert!(stream.is_err());
    }

    #[test]
    fn test_invalid_second_1() {
        let input = r#"{"index":0,"meta":{"time":"2019-06-04T07:40:00Z"},"patch":[]}
{"meta":{"time":"2019-06-04T07:40:00Z"}}"#;
        let stream = StateEventStream::open_from_reader(input.as_bytes()).unwrap();
        let vec: Vec<Result<StateEvent>> = stream.collect();
        assert!(vec[1].is_err());
    }

    #[test]
    fn test_invalid_second_2() {
        let input = r#"{"index":0,"meta":{"time":"2019-06-04T07:40:00Z"},"patch":[]}
{"#;
        let stream = StateEventStream::open_from_reader(input.as_bytes()).unwrap();
        let vec: Vec<Result<StateEvent>> = stream.collect();
        assert!(vec[1].is_err());
    }

    #[test]
    #[ignore] // Slow test, no assertions, temporary code
    fn test_wait_monitor() {
        let monitor = WaitMonitor::new("Testing...".to_owned());
        std::thread::sleep(Duration::from_secs(25));
        monitor.done();
    }

    #[test]
    fn test_state_file_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut state = StateHandle::open(&path, true).unwrap();
        state
            .append(&[&StateEvent {
                index: 0,
                meta: StateEventMeta {
                    time: "2019-03-04T07:40:00Z".to_owned(),
                    other_fields: serde_json::json!({}),
                },
                patch: json_patch::Patch(vec![json_patch::PatchOperation::Add(
                    json_patch::AddOperation {
                        path: "".parse().expect("empty path"),
                        value: serde_json::json!({
                            "_type": "nixopsState",
                            "resources": {},
                            "deployments": {},
                        }),
                    },
                )]),
            }])
            .unwrap();
        state
            .append(&[&StateEvent {
                index: 1,
                meta: StateEventMeta {
                    time: "2019-03-04T07:41:00Z".to_owned(),
                    other_fields: serde_json::json!({}),
                },
                patch: json_patch::Patch(vec![json_patch::PatchOperation::Add(
                    json_patch::AddOperation {
                        path: "/resources/a".parse().expect("empty path"),
                        value: serde_json::json!({
                            "type": "file",
                            "inputProperties": {
                                "contents": "Hi there",
                            },
                            "outputProperties": {},
                        }),
                    },
                )]),
            }])
            .unwrap();
        // read and print
        // let mut file = File::open(&path).unwrap();
        // let mut buf = Vec::new();
        // file.read_to_end(&mut buf).unwrap();
        // stdout().write_all(&buf).unwrap();
    }

    #[test]
    #[ignore] // Manual test, no assertions
    fn test_state_file_write_multiple_writers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let mut state = StateHandle::open(&path, true).unwrap();
        let mut state2 = StateHandle::open(&path, false).unwrap();

        state2
            .append(&[&StateEvent {
                index: 1,
                meta: StateEventMeta {
                    time: "2019-03-04T07:41:00Z".to_owned(),
                    other_fields: serde_json::json!({}),
                },
                patch: json_patch::Patch(vec![json_patch::PatchOperation::Add(
                    json_patch::AddOperation {
                        path: "/resources/a".parse().expect("empty path"),
                        value: serde_json::json!({
                            "type": "file",
                            "inputProperties": {
                                "contents": "Hi there",
                            },
                            "outputProperties": {},
                        }),
                    },
                )]),
            }])
            .unwrap();

        state
            .append(&[&StateEvent {
                index: 1,
                meta: StateEventMeta {
                    time: "2019-03-04T07:41:00Z".to_owned(),
                    other_fields: serde_json::json!({}),
                },
                patch: json_patch::Patch(vec![json_patch::PatchOperation::Add(
                    json_patch::AddOperation {
                        path: "/resources/a".parse().expect("empty path"),
                        value: serde_json::json!({
                            "type": "file",
                            "inputProperties": {
                                "contents": "Hi there",
                            },
                            "outputProperties": {},
                        }),
                    },
                )]),
            }])
            .unwrap();
        // read and print
        let mut file = File::open(&path).unwrap();
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).unwrap();
        stdout().write_all(&buf).unwrap();
    }
}
