//! Metadata-only observations; pages and process health are not delivery proof.
use connect_protocol::{tasks::*, Readiness};
use serde::{Deserialize, Serialize};

/// Independent authenticated checkpoints. A task cursor cannot be reused as a
/// conversation-reply or recall cursor. None starts that particular view.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct DiagnosticCursors {
    pub journal: Option<String>,
    pub sources: Option<String>,
    pub unassigned: Option<String>,
}

/// Missing permission or optional support is explicit, never an empty success.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "availability", content = "data", rename_all = "snake_case")]
pub enum DiagnosticSection<T> {
    Available(T),
    /// Stable code only; no arbitrary remote error message is retained.
    Unavailable {
        code: String,
    },
}

/// One bounded page, not an installation-wide count or a claim of exhaustion.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DiagnosticPage<T> {
    pub items: Vec<T>,
    /// The caller's input checkpoint, separate from the newly sampled position.
    pub after: Option<String>,
    pub next: Option<String>,
    pub limit: u32,
    /// Server-reported recall omissions; None means the unread extent is unknown.
    pub omitted: Option<u64>,
}

/// Journal metadata without message bodies, outcomes, or arbitrary reason text.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DiagnosticEvent {
    pub event_id: String,
    pub sequence: u64,
    pub observed_at: i64,
    pub observation: DiagnosticObservation,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
/// A past journal observation. Reconcile current effects before retrying work.
pub enum DiagnosticObservation {
    State {
        state: TaskState,
        revision: u64,
    },
    Effect {
        effect_id: String,
        step_id: String,
        request_id: String,
        state: EffectState,
    },
    Reply(TaskReply),
    /// Lease metadata is an ownership hint, not a process-liveness assertion.
    Claim(TaskStepClaim),
}

/// Authorized source attribution; source content and artifact bytes are omitted.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DiagnosticSource {
    pub source_id: String,
    pub kind: String,
    pub actor_id: String,
    pub created_at: Option<i64>,
    pub provenance: String,
    pub state: String,
}

/// Current task identity and lifecycle, without backing credentials or text.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct DiagnosticTask {
    pub task_id: String,
    pub conversation_id: String,
    pub state: TaskState,
    pub revision: u64,
    pub expires_at: i64,
    pub updated_at: i64,
}

/// Six bounded public reads. This is not one atomic database snapshot. Each
/// section checks current authority; the final task read checks it again. A
/// changed revision signals that the sampled sections can describe older work.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TaskDiagnostics {
    pub task: DiagnosticTask,
    pub changed_during_sample: bool,
    pub journal: DiagnosticSection<DiagnosticPage<DiagnosticEvent>>,
    pub sources: DiagnosticSection<DiagnosticPage<DiagnosticSource>>,
    pub unassigned: DiagnosticSection<DiagnosticPage<TaskReply>>,
    /// Process-local server supervision only. External hosts are not inferred.
    pub server_readiness: DiagnosticSection<Readiness>,
}
