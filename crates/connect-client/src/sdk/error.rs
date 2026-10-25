//! Safe structured context without private request bodies or credentials.
use super::OperationKey;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Validation,
    Configuration,
    Trust,
    Authentication,
    Protocol,
    Deadline,
    Cancelled,
    TransportUncertain,
    Unsupported,
    Conflict,
    Application,
    ReceiptUnavailable,
    ResyncRequired,
    CleanupIncomplete,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Certainty {
    NotSubmitted,
    Refused,
    Uncertain,
    Recorded,
    ReadOnly,
}
#[derive(Clone, Debug)]
pub struct CanonicalProblem {
    pub code: u64,
    pub category: String,
    pub reason: String,
}
#[derive(Clone, thiserror::Error)]
#[error("Connect SDK {kind:?} ({certainty:?})")]
pub struct Error {
    pub kind: ErrorKind,
    pub certainty: Certainty,
    pub operation: Option<OperationKey>,
    pub problem: Option<CanonicalProblem>,
    pub observation: Option<super::OperationSnapshot>,
}
impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Error")
            .field("kind", &self.kind)
            .field("certainty", &self.certainty)
            .field("operation", &self.operation)
            .finish_non_exhaustive()
    }
}
impl Error {
    pub(super) fn media(kind: ErrorKind, submitted: bool) -> Self {
        Self {
            kind,
            certainty: if submitted {
                Certainty::Uncertain
            } else {
                Certainty::NotSubmitted
            },
            operation: None,
            problem: None,
            observation: None,
        }
    }
    pub(super) fn local(kind: ErrorKind) -> Self {
        Self {
            kind,
            certainty: Certainty::NotSubmitted,
            operation: None,
            problem: None,
            observation: None,
        }
    }
    pub(super) fn request(
        kind: ErrorKind,
        operation: Option<&OperationKey>,
        submitted: bool,
    ) -> Self {
        Self {
            kind,
            certainty: if submitted && operation.is_some() {
                Certainty::Uncertain
            } else if operation.is_none() {
                Certainty::ReadOnly
            } else {
                Certainty::NotSubmitted
            },
            operation: operation.cloned(),
            problem: None,
            observation: None,
        }
    }
    pub(super) fn transport(error: crate::ClientError, operation: Option<&OperationKey>) -> Self {
        use crate::ClientError as E;
        let kind = match error {
            E::Configuration => ErrorKind::Configuration,
            E::Authentication => ErrorKind::Authentication,
            E::Timeout => ErrorKind::Deadline,
            E::Cancelled => ErrorKind::Cancelled,
            E::Protocol => ErrorKind::Protocol,
            E::Transport | E::MediaEnded => ErrorKind::TransportUncertain,
            E::Application(_) => ErrorKind::Application,
        };
        Self::request(kind, operation, true)
    }
}
pub type Result<T> = std::result::Result<T, Error>;
