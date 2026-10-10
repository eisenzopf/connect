//! Versioned application task contract. This is Connect policy, not a new UCTP noun.
//!
//! The server authenticates invocation grants outside model-controlled command
//! arguments. A task profile is explicitly required; older control clients keep
//! their existing communication behavior without receiving task authority.
mod lifecycle;
mod models;
mod request;
mod validation;
pub use lifecycle::{EffectState, TaskState};
pub use models::*;
pub use request::{TaskCommand, TaskRequest};

/// Required task extension profile inside the existing control application.
pub const TASK_PROFILE: &str = "connect-task/1";
/// Largest provider tool wait; the actual result may remain pending afterwards.
pub const MAX_TOOL_WAIT_MS: u32 = 10_000;
/// Hard task projection and paged recall budget, separate from retained history.
pub const TASK_CONTEXT_BYTES: usize = 32 * 1024;
/// Maximum delegated operations or recipients in one task scope.
pub const MAX_SCOPE_ITEMS: usize = 32;
/// Bound an opaque identifier to the existing control logical-ID envelope.
pub const MAX_TASK_ID_BYTES: usize = 128;
/// Maximum task lifetime; task expiry cannot expand the backing credential.
pub const MAX_TASK_TTL_SECONDS: i64 = 86_400;
/// Maximum lifetime of one narrow invocation grant.
pub const MAX_GRANT_TTL_SECONDS: i64 = 300;
/// Admission ceiling for nonterminal tasks in one installation.
pub const MAX_ACTIVE_TASKS: u32 = 1_024;
/// Resource ceilings for retained task metadata and pending inbox admission.
pub const MAX_RETAINED_TASKS: u32 = 10_000;
/// Per-task unconsumed reply events. Capacity rejection does not acknowledge delivery.
pub const MAX_TASK_INBOX_EVENTS: u32 = 1_000;
/// Maximum concurrent reference-runner claims in the qualified local profile.
pub const MAX_RUNNING_TASKS: u32 = 16;
/// A claim expires without granting permission to retry a dispatched effect.
pub const TASK_CLAIM_SECONDS: i64 = 30;
