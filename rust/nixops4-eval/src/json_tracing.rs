use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::{
    span::{self},
    Subscriber,
};
use tracing_serde_structured::AsSerde;

pub struct QueueSubscriber {
    next_id: AtomicUsize, // you need to assign span IDs, so you need a counter
    // queue: Sender<Value>,
    send: Box<dyn Fn(Value) + Send + Sync>,
}

impl QueueSubscriber {
    pub fn new(f: Box<dyn Fn(Value) + Send + Sync>) -> QueueSubscriber {
        QueueSubscriber {
            next_id: AtomicUsize::new(1),
            // queue: logs_tx,
            send: f,
        }
    }

    fn next_id(&self) -> span::Id {
        span::Id::from_u64(self.next_id.fetch_add(1, Ordering::Relaxed) as u64)
    }
}
impl Subscriber for QueueSubscriber {
    fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
        // We log everything call site
        true
    }

    fn new_span(&self, span: &span::Attributes<'_>) -> span::Id {
        let id = self.next_id();
        let values = span.values();
        // build a json object with the span's fields
        struct MyVisitor {
            fields: serde_json::Map<String, Value>,
        }
        impl tracing::field::Visit for MyVisitor {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                let name = field.name();
                let value = format!("{:?}", value);
                self.fields.insert(name.to_string(), Value::String(value));
            }

            fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
                self.fields.insert(
                    field.name().to_string(),
                    Value::Number(serde_json::Number::from_f64(value).unwrap()),
                );
            }

            fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
                self.fields.insert(
                    field.name().to_string(),
                    Value::Number(serde_json::Number::from(value)),
                );
            }

            fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
                self.fields.insert(
                    field.name().to_string(),
                    Value::Number(serde_json::Number::from(value)),
                );
            }

            // Not implemented in serde_json
            // fn record_i128(&mut self, field: &tracing::field::Field, value: i128) {
            //     self.fields.insert(field.name().to_string(), Value::Number(serde_json::Number::from(value)));
            // }

            // Not implemented in serde_json
            // fn record_u128(&mut self, field: &tracing::field::Field, value: u128) {
            //     self.fields.insert(field.name().to_string(), Value::Number(serde_json::Number::from(value)));
            // }

            fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
                self.fields
                    .insert(field.name().to_string(), Value::Bool(value));
            }

            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                self.fields
                    .insert(field.name().to_string(), Value::String(value.to_string()));
            }
        }
        let mut visitor = MyVisitor {
            fields: serde_json::Map::new(),
        };
        values.record(&mut visitor);
        let json = json!({ "type": "NewSpan", "span": span.as_serde(), "values": visitor.fields });
        self.send.as_ref()(json);
        id
    }

    fn record(&self, span: &span::Id, values: &span::Record<'_>) {
        let json =
            json!({ "type": "Record", "span": span.as_serde(), "values": values.as_serde() });
        self.send.as_ref()(json);
    }

    fn record_follows_from(&self, span: &span::Id, follows: &span::Id) {
        let json = json!({ "type": "RecordFollowsFrom", "span": span.as_serde(), "follows": follows.as_serde() });
        self.send.as_ref()(json);
    }

    fn event(&self, event: &tracing::Event<'_>) {
        let json = json!({ "type": "Event", "event": event.as_serde() });
        self.send.as_ref()(json);
    }

    fn enter(&self, span: &span::Id) {
        let json = json!({ "type": "Enter", "span": span.as_serde() });
        self.send.as_ref()(json);
    }

    fn exit(&self, span: &span::Id) {
        let json = json!({ "type": "Exit", "span": span.as_serde() });
        self.send.as_ref()(json);
    }
}
