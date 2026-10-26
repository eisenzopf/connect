//! Durable task and effect states. Media-session state is deliberately separate.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};

/// A task remains attached to its conversation through waiting and restart.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Ready,
    Running,
    Waiting,
    Completed,
    Cancelled,
    Failed,
    Expired,
}
impl TaskState {
    /// Terminal tasks retain their history but cannot accept another effect.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Cancelled | Self::Failed | Self::Expired
        )
    }
    /// Validate a transition; the store must also compare the current revision.
    ///
    /// Repeating a terminal transition is only legal through a matching receipt,
    /// not a new request that attempts to reopen completed work.
    pub fn transition(self, next: Self) -> Result<()> {
        let allowed = !self.is_terminal()
            && (matches!(next, Self::Cancelled | Self::Expired | Self::Failed)
                || matches!(
                    (self, next),
                    (Self::Ready, Self::Running)
                        | (Self::Running, Self::Waiting | Self::Completed)
                        | (Self::Waiting, Self::Ready)
                ));
        if allowed {
            Ok(())
        } else {
            Err(Error::new(
                "task-state-conflict",
                "Task transition is not permitted",
            ))
        }
    }
}

/// Dispatch uncertainty is persisted independently of the task lifecycle.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EffectState {
    Prepared,
    Dispatched,
    Succeeded,
    Failed,
    Uncertain,
    Cancelled,
}
impl EffectState {
    /// Only undispatched prepared work may be safely claimed for dispatch.
    pub fn may_dispatch(self) -> bool {
        self == Self::Prepared
    }
    /// Reconciliation records an observation; it never re-prepares dispatch.
    pub fn transition(self, next: Self) -> Result<()> {
        if matches!(
            (self, next),
            (Self::Prepared, Self::Dispatched | Self::Cancelled)
                | (
                    Self::Dispatched,
                    Self::Succeeded | Self::Failed | Self::Uncertain
                )
                | (Self::Uncertain, Self::Succeeded | Self::Failed)
        ) {
            Ok(())
        } else {
            Err(Error::new(
                "effect-state-conflict",
                "Effect transition is not permitted",
            ))
        }
    }
}
