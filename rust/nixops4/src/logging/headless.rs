use crate::logging::level_filter::LevelFilter2;

use super::Frontend;
use anyhow::Result;
use tracing_subscriber::{
    fmt::{format::FmtSpan, Layer as FmtLayer},
    layer::SubscriberExt as _,
    Registry,
};

pub(crate) struct HeadlessLogger {}

impl Frontend for HeadlessLogger {
    fn set_up(&mut self, options: &super::Options) -> Result<()> {
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

        tracing::subscriber::set_global_default(subscriber)
            .map_err(|e| anyhow::anyhow!("failed to set up tracing: {}", e))?;

        Ok(())
    }

    fn tear_down(&mut self) -> Result<()> {
        Ok(())
    }
}
