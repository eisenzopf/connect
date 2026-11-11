//! Closed diagnostic labels for two distinct public 503 mapping boundaries.
//! Never retain or emit a request, credential, scope, arbitrary message or grant.
use connect_protocol::Error;

pub(crate) fn observe(
    boundary: &'static str,
    command: &'static str,
    stage: &'static str,
    error: &Error,
) {
    if !matches!(
        error.code.as_str(),
        "authority-busy" | "authority-unavailable"
    ) {
        return;
    }
    let code = if error.code == "authority-busy" {
        "authority-busy"
    } else {
        "authority-unavailable"
    };
    // Match only known internal literals. Unrecognized messages remain private;
    // they must never silently acquire a retry classification or change output.
    let leaf = match error.message.as_str() {
        "Current state is busy" => "primary-sql-mutex",
        "Current authority is busy; retry the same logical operation" => "authority-wait-cutoff",
        "Current authority is busy" => "authority-immediate-busy",
        "Current authority reader registry is busy" => "authority-reader-registry",
        "State file changed while an authority owner is active" => "state-owner-file-changed",
        "State file changed during authority admission" => "state-admission-file-changed",
        "Authority resource ownership is inconsistent" => "authority-resource-inconsistent",
        "Authority file changed" => "authority-file-changed",
        "Authority file unavailable" => "authority-file-unavailable",
        "Unsafe authority file" => "authority-file-unsafe",
        "Authority access changed" => "authority-file-access",
        "Authority lock failed" => "authority-kernel-lock",
        "Hard-linked state files are not supported" => "state-hardlink",
        "Unsafe authority directory" => "authority-directory-unsafe",
        "Authority files must remain private to the state owner" => "authority-file-privacy",
        "Installation ownership is unavailable or its files changed" => "installation-ownership",
        "Native subscription catalog is busy" => "subscription-catalog",
        "Subscription source metadata is busy" => "subscription-source-metadata",
        "Original lifetime snapshot is refreshing" => "lifetime-snapshot-refresh",
        _ => "unclassified-private-message",
    };
    tracing::debug!(target: "connect_service::canonical_replacement::control_unavailable_origin", boundary, command, stage, code, leaf,
        "Original control unavailable origin");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn actual_native_and_browser_runner_filters_admit_closed_control_failure_labels() {
        // Read the actual checked-in launch filters: another source change may
        // not silently disable the only distinction between public503 origins.
        let native = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/check-transport-fixtures.py"
        ));
        let browser = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/rehearse-gate6-browser-matrix.py"
        ));
        let base = native
            .lines()
            .find(|l| l.contains("server_env = dict(env, RUST_LOG="))
            .unwrap()
            .split("RUST_LOG=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let commit=native.lines().find(|l|l.contains("server_env[\"RUST_LOG\"] += \",connect_service::canonical_replacement=debug,rvoip_core::replacement_commit=debug")).unwrap()
            .split(" += \"").nth(1).unwrap().split('"').next().unwrap();
        let browser = browser
            .lines()
            .find(|l| l.contains("env[\"RUST_LOG\"] = \"warn,"))
            .unwrap()
            .split(" = \"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        for filter in [format!("{base}{commit}"), browser.to_owned()] {
            let bytes = Arc::new(Mutex::new(Vec::new()));
            let writer = bytes.clone();
            let subscriber = tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
                .with_writer(move || Capture(writer.clone()))
                .finish();
            tracing::subscriber::with_default(subscriber, || {
                observe(
                    "peer-policy",
                    "stream.subscribe",
                    "store-admit",
                    &Error::new(
                        "authority-busy",
                        "Current authority reader registry is busy",
                    ),
                );
                observe(
                    "native-stream-worker",
                    "stream.subscribe",
                    "accept",
                    &Error::new("authority-busy", "private-unrecognized-never-emitted"),
                );
                observe(
                    "must-not-emit",
                    "stream.subscribe",
                    "accept",
                    &Error::forbidden(),
                );
            });
            let text = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
            assert!(
                text.contains("peer-policy") && text.contains("authority-reader-registry"),
                "{text}"
            );
            assert!(
                text.contains("native-stream-worker")
                    && text.contains("unclassified-private-message"),
                "{text}"
            );
            assert!(
                !text.contains("private-unrecognized-never-emitted")
                    && !text.contains("must-not-emit")
            );
        }
    }
}
