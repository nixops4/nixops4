use std::{
    error::Error,
    fmt::Display,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

#[derive(Clone, Debug)]
pub struct InterruptState {
    interrupted: Arc<AtomicBool>,
}

#[derive(Clone, Debug)]
pub struct InterruptedError {}
impl Display for InterruptedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "interrupted")
    }
}
impl Error for InterruptedError {}

impl InterruptState {
    pub fn new() -> Self {
        Self {
            interrupted: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn set_interrupted(&self) {
        self.interrupted.store(true, Ordering::SeqCst);
    }

    pub fn is_interrupted(&self) -> bool {
        self.interrupted.load(Ordering::SeqCst)
    }

    pub fn check_interrupted_raw(&self) -> Result<(), InterruptedError> {
        if self.is_interrupted() {
            Err(InterruptedError {})
        } else {
            Ok(())
        }
    }

    pub fn check_interrupted(&self) -> anyhow::Result<()> {
        self.check_interrupted_raw().map_err(|x| x.into())
    }
}

fn set_process_interrupt_handler(interrupted: &InterruptState) {
    let interrupted = interrupted.clone();
    ctrlc::set_handler(move || {
        interrupted.set_interrupted();
    })
    .expect("Error setting interrupt handler");
}

pub fn set_up_process_interrupt_handler() -> InterruptState {
    let interrupt_state = InterruptState::new();
    set_process_interrupt_handler(&interrupt_state);
    interrupt_state
}
