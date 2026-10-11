//! Browser console logging. Bevy's `LogPlugin` on wasm also records every span
//! as a `performance.mark`/`measure` pair, a string round trip into JS several
//! hundred times a frame; this layer only forwards log events to the console.
use bevy::log::tracing::{Event, Level, Subscriber, field::{Field, Visit}};
use bevy::log::tracing_subscriber::{EnvFilter, Layer, layer::Context, layer::SubscriberExt, util::SubscriberInitExt};
use std::fmt::Write;

struct Console;

struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?}");
        } else {
            let _ = write!(self.0, " {}={value:?}", field.name());
        }
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0.push_str(value);
        } else {
            let _ = write!(self.0, " {}={value}", field.name());
        }
    }
}

impl<S: Subscriber> Layer<S> for Console {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let meta = event.metadata();
        let mut text = Message(format!("{} {}: ", meta.level(), meta.target()));
        event.record(&mut text);
        let text = wasm_bindgen::JsValue::from_str(&text.0);
        match *meta.level() {
            Level::ERROR => web_sys::console::error_1(&text),
            Level::WARN => web_sys::console::warn_1(&text),
            Level::INFO => web_sys::console::info_1(&text),
            _ => web_sys::console::debug_1(&text),
        }
    }
}

pub(super) fn install() {
    let filter = EnvFilter::new("info,wgpu=error,naga=warn");
    let _ = bevy::log::tracing_subscriber::registry().with(filter).with(Console).try_init();
}
