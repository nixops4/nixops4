use crate::logging::level_filter::LevelFilter2;

use super::Frontend;
use anyhow::Result;
use tracing_subscriber::{
    fmt::{format::FmtSpan, Layer as FmtLayer},
    layer::{Layered, SubscriberExt as _},
    Registry,
};

pub(crate) struct HeadlessLogger {}

pub(crate) type Logger = Layered<LevelFilter2<FmtLayer<Registry>>, Registry>;

impl HeadlessLogger {
    pub(crate) fn make_subscriber(&mut self, options: &super::Options) -> Result<Logger> {
        let filter = if options.verbose {
            eprintln!("setting up verbose logging");
            tracing::Level::TRACE
        } else {
            tracing::Level::INFO
        };

        let span_events = if options.verbose {
            // include enter/exit events for detailed tracing
            FmtSpan::FULL
        } else {
            // announce what we do and when we're done
            FmtSpan::NEW | FmtSpan::CLOSE
        };

        let fmt_layer = FmtLayer::new()
            .with_span_events(span_events)
            .with_ansi(options.color);
        let filter_layer = LevelFilter2::new(filter.into(), fmt_layer);
        let subscriber = Registry::default().with(filter_layer);
        Ok(subscriber)
    }

    pub fn handle_panic_no_exit(panic_info: &std::panic::PanicHookInfo<'_>) {
        // This is based on the tracing panic handler:
        //   https://github.com/tokio-rs/tracing/blob/bdbaf8007364ed2a766cca851d63de31b7c47e72/examples/examples/panic_hook.rs

        // If the panic has a source location, record it as structured fields.
        if let Some(location) = panic_info.location() {
            // On nightly Rust, where the `PanicInfo` type also exposes a
            // `message()` method returning just the message, we could record
            // just the message instead of the entire `fmt::Display`
            // implementation, avoiding the duplicated location
            tracing::error!(
                message = %panic_info,
                panic.file = location.file(),
                panic.line = location.line(),
                panic.column = location.column(),
            );
        } else {
            tracing::error!(message = %panic_info);
        }
    }
}

impl Frontend for HeadlessLogger {
    fn set_up(&mut self, options: &super::Options) -> Result<()> {
        let subscriber = self.make_subscriber(options)?;
        tracing::subscriber::set_global_default(subscriber)
            .map_err(|e| anyhow::anyhow!("failed to set up tracing: {}", e))?;

        Ok(())
    }

    fn tear_down(&mut self) -> Result<()> {
        Ok(())
    }

    fn get_panic_handler(&self) -> Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Send + Sync> {
        Box::new(|panic_info| {
            HeadlessLogger::handle_panic_no_exit(panic_info);
            std::process::exit(101);
        })
    }
}
