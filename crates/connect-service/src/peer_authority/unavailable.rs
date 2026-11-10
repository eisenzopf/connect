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
    tracing::debug!(target: "connect_service::control_unavailable_origin", boundary, command, stage, code, leaf,
        "Original control unavailable origin");
}
