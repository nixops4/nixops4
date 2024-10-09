mod headless;
mod level_filter;

use anyhow::Result;

pub(crate) struct Options {
    pub verbose: bool,
    pub color: bool,
}

pub(crate) trait Frontend {
    fn set_up(&self, options: &Options) -> Result<()>;
}

pub(crate) fn set_up(options: Options) -> Result<Box<dyn Frontend>> {
    let logger = headless::HeadlessLogger {};
    logger.set_up(&options)?;
    Ok(Box::new(logger))
}
