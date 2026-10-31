//! Explicit synthetic-lane diagnostics. No per-packet console or file writes.
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::{field::Visit, Event, Subscriber};
use tracing_subscriber::{
    filter::{filter_fn, LevelFilter},
    layer::{Context, SubscriberExt},
    util::SubscriberInitExt,
    Layer,
};

const MAX_EVENTS: usize = 65_536;
const MAX_BYTES: usize = 32 * 1024 * 1024;
const TARGETS: &[&str] = &[
    "rvoip_webrtc_stack::owned_rtp",
    "rvoip_core::audio_path",
    "rvoip_webrtc::audio_timing",
    "rvoip_webrtc::canonical::media",
    "connect_service::canonical_packet",
    "connect_store::sql_custody",
];
const FIELDS: &[&str] = &[
    "log.target",
    "connection",
    "error",
    "role",
    "detail",
    "owned_file_custody",
    "receiving",
    "private",
    "source_ssrc",
    "target_ssrc",
    "source_timestamp",
    "wire_timestamp",
    "step",
    "message",
    "source",
    "target",
    "stream",
    "stage",
    "count",
    "medium",
    "reason",
    "problem",
    "result",
    "ssrc",
    "sequence",
    "sequence_number",
    "timestamp",
    "rtp_timestamp",
    "timestamp_rtp",
    "payload_type",
    "bytes",
    "samples",
    "sources",
    "source_count",
    "discarded_chunks",
    "accepted",
    "age_us",
    "elapsed_us",
    "file",
    "line",
    "read_us",
    "publish_us",
    "write_us",
    "codec",
    "received_us",
    "original_age_us",
];

struct State {
    rows: Vec<Vec<u8>>,
    bytes: usize,
    omitted: u64,
    suppressed_successes: u64,
    max_events: usize,
    max_bytes: usize,
}
#[derive(Clone)]
struct Capture(Arc<Mutex<State>>);
impl Capture {
    fn new(max_events: usize, max_bytes: usize) -> Self {
        Self(Arc::new(Mutex::new(State {
            rows: Vec::new(),
            bytes: 0,
            omitted: 0,
            suppressed_successes: 0,
            max_events,
            max_bytes,
        })))
    }
}
#[derive(Default)]
struct Fields(BTreeMap<String, serde_json::Value>);
impl Fields {
    fn put(&mut self, field: &tracing::field::Field, value: serde_json::Value) {
        if FIELDS.contains(&field.name()) {
            self.0.insert(field.name().into(), value);
        }
    }
}
impl Visit for Fields {
    fn record_u64(&mut self, f: &tracing::field::Field, v: u64) {
        self.put(f, v.into());
    }
    fn record_i64(&mut self, f: &tracing::field::Field, v: i64) {
        self.put(f, v.into());
    }
    fn record_bool(&mut self, f: &tracing::field::Field, v: bool) {
        self.put(f, v.into());
    }
    fn record_f64(&mut self, f: &tracing::field::Field, v: f64) {
        self.put(f, serde_json::json!(v));
    }
    fn record_str(&mut self, f: &tracing::field::Field, v: &str) {
        if FIELDS.contains(&f.name()) {
            self.put(f, v.chars().take(256).collect::<String>().into());
        }
    }
    fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
        if FIELDS.contains(&f.name()) {
            self.record_str(f, &format!("{v:?}"));
        }
    }
}
impl<S: Subscriber> Layer<S> for Capture {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        let bridged_target = fields
            .0
            .remove("log.target")
            .and_then(|value| value.as_str().map(str::to_owned));
        let target = if event.metadata().target() == "log" {
            match bridged_target.as_deref() {
                Some("rvoip_webrtc_stack::owned_rtp") => "rvoip_webrtc_stack::owned_rtp",
                _ => return,
            }
        } else {
            event.metadata().target()
        };
        if target == "rvoip_webrtc_stack::owned_rtp" {
            let refusal = fields
                .0
                .get("message")
                .and_then(|value| value.as_str())
                .and_then(|message| {
                    message.strip_prefix("Original guarded RTC RTP write refused stage=")
                })
                .and_then(|detail| detail.split_once(" error="))
                .filter(|(stage, _)| {
                    stage.len() <= 64 && stage.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
                })
                .map(|(stage, error)| (stage.to_owned(), error.to_owned()));
            if let Some((stage, error)) = refusal {
                fields.0.insert("stage".into(), stage.into());
                fields.0.insert("error".into(), error.into());
            }
        }
        if target == "connect_service::canonical_packet"
            && fields.0.get("message")
                == Some(&serde_json::json!("Current native packet authority"))
            && !fields.0.contains_key("problem")
        {
            let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
            state.suppressed_successes = state.suppressed_successes.saturating_add(1);
            return;
        }
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let row = serde_json::json!({
            "unix_nanos":at.as_nanos(), "target":target,
            "level":event.metadata().level().as_str(), "fields":fields.0,
        });
        let Ok(bytes) = serde_json::to_vec(&row) else {
            let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
            state.omitted = state.omitted.saturating_add(1);
            return;
        };
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if state.rows.len() >= state.max_events
            || bytes.len() > state.max_bytes.saturating_sub(state.bytes)
        {
            state.omitted = state.omitted.saturating_add(1);
            return;
        }
        state.bytes += bytes.len();
        state.rows.push(bytes);
    }
}

pub(super) struct TraceGuard {
    file: Option<File>,
    capture: Capture,
}
impl TraceGuard {
    fn open(allowed: &Path, directory: &Path, capture: Capture) -> io::Result<Self> {
        let allowed = allowed.canonicalize()?;
        let directory = directory.canonicalize()?;
        if !directory.starts_with(allowed) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Audio diagnostics require an ignored local directory",
            ));
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        Ok(Self {
            file: Some(options.open(directory.join("core-audio-timeline.json"))?),
            capture,
        })
    }
    fn flush(&mut self) -> io::Result<()> {
        let Some(mut file) = self.file.take() else {
            return Ok(());
        };
        let state = self.capture.0.lock().unwrap_or_else(|p| p.into_inner());
        let header = serde_json::json!({
            "scope":"Bounded numeric synthetic-lane source/mixer/publication/write diagnostics; not a normal-profile qualification",
            "maximum_events":state.max_events, "maximum_event_bytes":state.max_bytes,
            "captured_event_bytes":state.bytes, "omitted_events":state.omitted,
            "suppressed_known_successes":state.suppressed_successes,
            "selection":"Only repeated successful Current native packet authority events are suppressed; refusals and source/mixer/publication/write timing remain selected",
        });
        let mut prefix = serde_json::to_vec(&header)?;
        prefix.pop();
        file.write_all(&prefix)?;
        file.write_all(b",\"events\":[")?;
        for (i, row) in state.rows.iter().enumerate() {
            if i != 0 {
                file.write_all(b",")?;
            }
            file.write_all(row)?;
        }
        file.write_all(b"]}\n")
    }
}
impl Drop for TraceGuard {
    fn drop(&mut self) {
        if let Err(error) = self.flush() {
            // A failed diagnostic must never replace the original test panic.
            eprintln!("Private audio timeline could not be written: {error}");
        }
    }
}

pub(super) fn install(root: &Path) -> Option<TraceGuard> {
    let directory = std::env::var_os("CONNECT_GATE6_CANONICAL_AUDIO_TRACE")?;
    let capture = Capture::new(MAX_EVENTS, MAX_BYTES);
    let guard = TraceGuard::open(&root.join(".local"), Path::new(&directory), capture.clone())
        .expect("explicit private audio timeline destination");
    tracing_subscriber::registry()
        .with(capture.with_filter(filter_fn(|meta| {
            TARGETS.contains(&meta.target()) || meta.target() == "log"
        })))
        .with(tracing_subscriber::fmt::layer().with_filter(LevelFilter::WARN))
        .try_init()
        .expect("audio timeline subscriber must be the original test subscriber");
    Some(guard)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_log_bridge_keeps_only_owned_writer_stage_and_error() {
        let root = tempfile::tempdir().unwrap();
        let capture = Capture::new(4, 4096);
        let guard = TraceGuard::open(root.path(), root.path(), capture.clone()).unwrap();
        let subscriber = tracing_subscriber::registry().with(capture);
        tracing::subscriber::with_default(subscriber, || {
            tracing::trace!(target:"log", { "log.target" = "foreign" }, "synthetic-offlist-secret");
            tracing::trace!(target:"log", { "log.target" = "rvoip_webrtc_stack::owned_rtp" },
                "Original guarded RTC RTP write refused stage=clock-custody error=ErrRTPTransceiverCodecUnsupported");
        });
        drop(guard);
        let text = std::fs::read_to_string(root.path().join("core-audio-timeline.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(!text.contains("synthetic-offlist-secret"));
        assert_eq!(value["events"].as_array().unwrap().len(), 1);
        assert_eq!(
            value["events"][0]["target"],
            "rvoip_webrtc_stack::owned_rtp"
        );
        assert_eq!(value["events"][0]["fields"]["stage"], "clock-custody");
        assert_eq!(
            value["events"][0]["fields"]["error"],
            "ErrRTPTransceiverCodecUnsupported"
        );
    }
    #[test]
    fn private_timeline_bounds_actual_events_whitelists_fields_and_flushes_on_unwind() {
        let root = tempfile::tempdir().unwrap();
        let capture = Capture::new(3, 4096);
        let guard = TraceGuard::open(root.path(), root.path(), capture.clone()).unwrap();
        let result = std::panic::catch_unwind(|| {
            let subscriber = tracing_subscriber::registry().with(capture.clone());
            tracing::subscriber::with_default(subscriber, || {
                for count in 0..6u64 {
                    tracing::trace!(target:"rvoip_core::audio_path", count, source="fixture", stage="mixer-target", ignored_secret="synthetic-not-a-key");
                }
            });
            let _guard = guard;
            panic!("original controlled failure");
        });
        assert!(result.is_err());
        let file = root.path().join("core-audio-timeline.json");
        let text = std::fs::read_to_string(&file).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["events"].as_array().unwrap().len(), 3);
        assert_eq!(value["omitted_events"], 3);
        assert!(!text.contains("synthetic-not-a-key") && !text.contains("ignored_secret"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(TraceGuard::open(root.path(), root.path(), capture).is_err());
        assert_eq!(
            std::fs::read_to_string(root.path().join("core-audio-timeline.json")).unwrap(),
            text
        );
    }
    #[test]
    fn timeline_refuses_escaped_paths_and_records_byte_budget_omissions() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let capture = Capture::new(10, 1);
        assert!(TraceGuard::open(root.path(), outside.path(), capture.clone()).is_err());
        #[cfg(unix)]
        {
            let link = root.path().join("outside");
            std::os::unix::fs::symlink(outside.path(), &link).unwrap();
            assert!(TraceGuard::open(root.path(), &link, capture.clone()).is_err());
        }
        let guard = TraceGuard::open(root.path(), root.path(), capture.clone()).unwrap();
        let subscriber = tracing_subscriber::registry().with(capture);
        tracing::subscriber::with_default(subscriber, || {
            tracing::trace!(target:"rvoip_core::audio_path", count=1u64);
        });
        drop(guard);
        let value: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.path().join("core-audio-timeline.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(value["events"], serde_json::json!([]));
        assert_eq!(value["captured_event_bytes"], 0);
        assert_eq!(value["omitted_events"], 1);
    }
    #[test]
    fn repeated_successes_do_not_spend_the_refusal_timeline_budget() {
        let root = tempfile::tempdir().unwrap();
        let capture = Capture::new(1, 4096);
        let guard = TraceGuard::open(root.path(), root.path(), capture.clone()).unwrap();
        let subscriber = tracing_subscriber::registry().with(capture);
        tracing::subscriber::with_default(subscriber, || {
            for _ in 0..2 {
                tracing::trace!(target:"connect_service::canonical_packet", connection="fixture", "Current native packet authority");
            }
            tracing::trace!(target:"connect_service::canonical_packet", connection="fixture", problem="authority-busy", "Current native packet authority");
        });
        drop(guard);
        let value: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.path().join("core-audio-timeline.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(value["suppressed_known_successes"], 2);
        assert_eq!(value["omitted_events"], 0);
        assert_eq!(value["events"].as_array().unwrap().len(), 1);
        assert_eq!(value["events"][0]["fields"]["problem"], "authority-busy");
    }

    #[test]
    fn sql_custody_capture_keeps_bounded_timing_fields_without_query_text() {
        let root = tempfile::tempdir().unwrap();
        let capture = Capture::new(4, 4096);
        let guard = TraceGuard::open(root.path(), root.path(), capture.clone()).unwrap();
        let subscriber = tracing_subscriber::registry()
            .with(capture.with_filter(filter_fn(|metadata| TARGETS.contains(&metadata.target()))));
        tracing::subscriber::with_default(subscriber, || {
            tracing::trace!(target:"connect_store::sql_custody", elapsed_us=1_200u64,
                file="crates/connect-store/src/peer_authority/connection_media/snapshot.rs", line=181u64,
                query="synthetic-not-a-query", "Slow SQL custody unit");
        });
        drop(guard);
        let text = std::fs::read_to_string(root.path().join("core-audio-timeline.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["events"].as_array().unwrap().len(), 1);
        assert_eq!(value["events"][0]["fields"]["elapsed_us"], 1200);
        assert_eq!(value["events"][0]["fields"]["line"], 181);
        assert!(!text.contains("synthetic-not-a-query"));
        assert!(value["events"][0]["fields"].get("query").is_none());
    }
}
