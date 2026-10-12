//! Closed diagnostic stages keep provider payloads and private paths out of logs.
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CallStage {
    IntentRead,
    Origin,
    SessionAuthority,
    Route,
    InvocationAuthority,
    PriorDispatch,
    ModelDeadline,
    ModelLookup,
    Assistant,
    ModelShape,
    CurrentAuthority,
    Options,
    IntentWrite,
}

impl CallStage {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::IntentRead => "intent-read",
            Self::Origin => "origin",
            Self::SessionAuthority => "session-authority",
            Self::Route => "route",
            Self::InvocationAuthority => "invocation-authority",
            Self::PriorDispatch => "prior-dispatch",
            Self::ModelDeadline => "model-deadline",
            Self::ModelLookup => "model-lookup",
            Self::Assistant => "assistant",
            Self::ModelShape => "model-shape",
            Self::CurrentAuthority => "current-authority",
            Self::Options => "options",
            Self::IntentWrite => "intent-write",
        }
    }
}

impl fmt::Display for CallStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

impl std::error::Error for CallStage {}

pub(crate) fn stage(error: &anyhow::Error) -> &'static str {
    error
        .downcast_ref::<CallStage>()
        .map(|stage| stage.label())
        .unwrap_or("setup-closed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_expose_only_the_stage_even_with_a_sensitive_cause() {
        let error = anyhow::anyhow!("Bearer private-secret; provider body; /private/path")
            .context(CallStage::ModelLookup);
        assert_eq!(stage(&error), "model-lookup");
        assert_eq!(error.to_string(), "model-lookup");
        assert_eq!(stage(&anyhow::anyhow!("private-secret")), "setup-closed");
    }
}
