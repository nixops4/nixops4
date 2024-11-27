mod headless;
pub mod interactive;
mod level_filter;

use anyhow::Result;

use crate::interrupt::InterruptState;

pub(crate) struct Options {
    pub verbose: bool,
    pub color: bool,
    pub interactive: bool,
}

pub(crate) trait Frontend {
    fn set_up(&mut self, options: &Options) -> Result<()>;
    fn tear_down(&mut self) -> Result<()>;
    fn get_panic_handler(&self) -> Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync>;
}

pub(crate) fn set_up(
    interrupt_state: &InterruptState,
    options: Options,
) -> Result<Box<dyn Frontend>> {
    let mut logger: Box<dyn Frontend>;
    if options.interactive {
        logger = Box::new(interactive::InteractiveLogger::new(interrupt_state.clone()));
    } else {
        logger = Box::new(headless::HeadlessLogger {});
    }
    logger.set_up(&options)?;
    std::panic::set_hook(logger.get_panic_handler());
    Ok(logger)
}
