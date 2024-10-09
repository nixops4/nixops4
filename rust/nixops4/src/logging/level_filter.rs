use std::{collections::HashSet, sync::Mutex};

use tracing::{level_filters::LevelFilter, span, Event, Subscriber};
use tracing_subscriber::{layer::Context, Layer};

pub(crate) struct LevelFilter2<FmtLayer> {
    filter: LevelFilter,
    spans_ok: Mutex<HashSet<span::Id>>,
    format_layer: FmtLayer,
}

impl<FmtLayer> LevelFilter2<FmtLayer> {
    pub(crate) fn new(filter: LevelFilter, format_layer: FmtLayer) -> Self {
        Self {
            filter,
            spans_ok: Mutex::new(HashSet::new()),
            format_layer,
        }
    }

    fn is_span_id_enabled(&self, _span: &span::Id) -> bool {
        self.spans_ok
            .lock()
            .expect("mutex poisoned")
            .contains(_span)
    }

    fn set_span_id_enabled(&self, id: span::Id) {
        self.spans_ok.lock().expect("mutex poisoned").insert(id);
    }
}

/// TracingEvents from tracing_tunnel aren't filtered properly.
/// This might be a bug in `tracing-subscriber` or `tracing`, probably due to
/// the optimization where `register_callsite`'s return value is central.
///
/// As a workaround we compose the `LevelFilter` and `FmtLayer` by hand into a
///  single `Layer` with the right behavior.
///
/// Broken:
/// ```
/// Subscriber::builder()
///     .with_max_level(filter)
///     .with_span_events(span_events)
///     .finish()
/// ```
///
/// Similarly for `Registry::default().with(filter_layer).with(fmt_layer)`.
impl<S: Subscriber, FmtLayer: Layer<S>> Layer<S> for LevelFilter2<FmtLayer>
where
    Self: 'static,
    for<'lookup> S: tracing_subscriber::registry::LookupSpan<'lookup>,
{
    fn on_register_dispatch(&self, subscriber: &tracing::Dispatch) {
        Layer::<S>::on_register_dispatch(&self.filter, subscriber);
    }

    fn on_layer(&mut self, subscriber: &mut S) {
        self.filter.on_layer(subscriber);
        self.format_layer.on_layer(subscriber);
    }

    fn register_callsite(
        &self,
        metadata: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        Layer::<S>::register_callsite(&self.filter, metadata);
        tracing::subscriber::Interest::sometimes()
    }

    fn enabled(&self, metadata: &tracing::Metadata<'_>, ctx: Context<'_, S>) -> bool {
        self.filter.enabled(metadata, ctx)
    }

    fn on_new_span(&self, attrs: &span::Attributes<'_>, id: &span::Id, ctx: Context<'_, S>) {
        if self.filter.enabled(attrs.metadata(), ctx.clone()) {
            self.set_span_id_enabled(id.clone());
            self.format_layer.on_new_span(attrs, id, ctx);
        }
    }

    fn on_record(&self, _span: &span::Id, _values: &span::Record<'_>, _ctx: Context<'_, S>) {
        if self.is_span_id_enabled(_span) {
            self.format_layer.on_record(_span, _values, _ctx);
        }
    }

    fn on_follows_from(&self, _span: &span::Id, _follows: &span::Id, _ctx: Context<'_, S>) {
        if self.is_span_id_enabled(_span) {
            self.format_layer.on_follows_from(_span, _follows, _ctx);
        }
    }

    fn event_enabled(&self, _event: &Event<'_>, _ctx: Context<'_, S>) -> bool {
        self.filter.enabled(_event.metadata(), _ctx.clone()) &&
            // Also ask original filter and the next layer
            self.filter.event_enabled(_event, _ctx)
    }

    fn on_event(&self, _event: &Event<'_>, _ctx: Context<'_, S>) {
        if self.filter.enabled(_event.metadata(), _ctx.clone()) {
            self.format_layer.on_event(_event, _ctx);
        }
    }

    fn on_enter(&self, _id: &span::Id, _ctx: Context<'_, S>) {
        if self.is_span_id_enabled(_id) {
            self.format_layer.on_enter(_id, _ctx)
        }
    }

    fn on_exit(&self, _id: &span::Id, _ctx: Context<'_, S>) {
        if self.is_span_id_enabled(_id) {
            self.format_layer.on_exit(_id, _ctx)
        }
    }

    fn on_close(&self, _id: span::Id, _ctx: Context<'_, S>) {
        if self.is_span_id_enabled(&_id) {
            self.format_layer.on_close(_id, _ctx)
        }
    }

    fn on_id_change(&self, _old: &span::Id, _new: &span::Id, _ctx: Context<'_, S>) {
        {
            let mut spans_ok = self.spans_ok.lock().expect("mutex poisoned");
            if spans_ok.contains(_old) {
                // Docs suggest this was a clone, so we probably shouldn't remove _old
                spans_ok.insert(_new.clone());
                self.format_layer.on_id_change(_old, _new, _ctx.clone());
            } else {
                return;
            }
        }
        self.filter.on_id_change(_old, _new, _ctx)
    }
}
