//! Bounded event summaries for explicit diagnostics, with no per-packet log IO.
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};
use tracing::{field::Visit, Event, Subscriber};
use tracing_subscriber::{layer::Context, prelude::*, Layer};

#[derive(Default, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct Key {
    pipeline: String,
    message: String,
    connection: String,
    source: String,
    target: String,
    stage: String,
    problem: String,
    publishing: Option<bool>,
    receiving: Option<bool>,
    owned_file_custody: Option<bool>,
    batched: Option<bool>,
}
#[derive(Default)]
struct Fields {
    key: Key,
    age: Option<u64>,
    elapsed: Option<u64>,
}
impl Visit for Fields {
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        match field.name() {
            "age_us" => self.age = Some(value),
            "elapsed_us" => self.elapsed = Some(value),
            _ => {}
        }
    }
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        match field.name() {
            "publishing" => self.key.publishing = Some(value),
            "receiving" => self.key.receiving = Some(value),
            "owned_file_custody" => self.key.owned_file_custody = Some(value),
            "batched" => self.key.batched = Some(value),
            _ => {}
        }
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        let slot = match field.name() {
            "message" => &mut self.key.message,
            "connection" => &mut self.key.connection,
            "source" => &mut self.key.source,
            "target" => &mut self.key.target,
            "stage" => &mut self.key.stage,
            "problem" => &mut self.key.problem,
            _ => return,
        };
        *slot = value.chars().take(160).collect();
    }
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if matches!(
            field.name(),
            "message" | "connection" | "source" | "target" | "stage" | "problem"
        ) {
            self.record_str(field, &format!("{value:?}"));
        }
    }
}
#[derive(Serialize)]
struct Distribution {
    count: u64,
    total_us: u64,
    maximum_us: u64,
    // Ten-millisecond bins, with the last bin including all values >=200ms.
    bins: Vec<u64>,
}
impl Default for Distribution {
    fn default() -> Self {
        Self {
            count: 0,
            total_us: 0,
            maximum_us: 0,
            bins: vec![0; 21],
        }
    }
}
impl Distribution {
    fn observe(&mut self, value: u64) {
        self.count += 1;
        self.total_us = self.total_us.saturating_add(value);
        self.maximum_us = self.maximum_us.max(value);
        self.bins[(value / 10000).min(20) as usize] += 1;
    }
}
#[derive(Serialize)]
struct Counts {
    events: u64,
    per_second: Vec<u64>,
    age: Distribution,
    elapsed: Distribution,
}
impl Default for Counts {
    fn default() -> Self {
        Self {
            events: 0,
            per_second: vec![0; 121],
            age: Distribution::default(),
            elapsed: Distribution::default(),
        }
    }
}
struct State {
    started: Instant,
    buckets: Mutex<BTreeMap<Key, Counts>>,
    dropped: AtomicU64,
}
struct SummaryLayer(Arc<State>);
impl<S: Subscriber> Layer<S> for SummaryLayer {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let mut fields = Fields::default();
        fields.key.pipeline = event.metadata().target().into();
        event.record(&mut fields);
        let Ok(mut buckets) = self.0.buckets.try_lock() else {
            self.0.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        };
        if buckets.len() >= 128 && !buckets.contains_key(&fields.key) {
            self.0.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let counts = buckets.entry(fields.key).or_default();
        counts.events += 1;
        counts.per_second[self.0.started.elapsed().as_secs().min(120) as usize] += 1;
        if let Some(age) = fields.age {
            counts.age.observe(age);
        }
        if let Some(elapsed) = fields.elapsed {
            counts.elapsed.observe(elapsed);
        }
    }
}
pub(super) struct Summary {
    state: Arc<State>,
    output: PathBuf,
}
impl Summary {
    pub(super) fn install(algorithm: &str) -> Self {
        let output = PathBuf::from(
            std::env::var_os("CONNECT_GATE6_BROWSER_ARTIFACTS")
                .expect("Explicit private diagnostic output"),
        )
        .join(algorithm);
        let state = Arc::new(State {
            started: Instant::now(),
            buckets: Mutex::new(BTreeMap::new()),
            dropped: AtomicU64::new(0),
        });
        tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new("rvoip_core::audio_path=trace,connect_service::canonical_packet=trace,connect_service::canonical_native_streams=debug,connect_store::sql_custody=trace"))
            .with(SummaryLayer(state.clone()))
            .try_init()
            .expect("One owned diagnostic subscriber");
        Self { state, output }
    }
}
impl Drop for Summary {
    fn drop(&mut self) {
        let Ok(buckets) = self.state.buckets.lock() else {
            return;
        };
        let rows = buckets
            .iter()
            .map(|(key, counts)| serde_json::json!({"key":key,"counts":counts}))
            .collect::<Vec<_>>();
        let report = serde_json::json!({"schema":"connect-gate6-packet-summary/1","scope":"Diagnostic event counts until test guard drop; no media bytes, permission, complete IO/cleanup or performance qualification. Setup and teardown are included. Counts can be incomplete when dropped_events is nonzero.","elapsed_ms":self.state.started.elapsed().as_secs_f64()*1000.,"maximum_buckets":128,"maximum_seconds":121,"dropped_events":self.state.dropped.load(Ordering::Relaxed),"rows":rows});
        if std::fs::create_dir_all(&self.output).is_ok() {
            if let Ok(bytes) = serde_json::to_vec_pretty(&report) {
                let _ = std::fs::write(self.output.join("packet-trace-summary.json"), bytes);
            }
        }
    }
}
