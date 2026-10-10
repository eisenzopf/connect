//! Durable task example: private application state and only the public SDK.
//!
//! The protected controller coordinates claims and consumes replies. Model
//! communication uses a separate Native invocation grant, never that controller.
//! A persisted consume receipt gates each local action; action, follow-up intent,
//! handled events and checkpoint then commit together in the application store.
mod commands;
mod driver;
mod journal;
mod reconciliation;
mod state;
#[cfg(test)]
mod tests;
mod workflow;

pub use driver::{run, RunOptions};
pub use state::{Binding, Statistics, TaskStore};

/// Process exits are opt-in and compiled only into explicit recovery-test builds.
#[cfg(feature = "test-support")]
fn fault(stage: &str) {
    if std::env::var("CONNECT_TASK_RUNNER_FAULT").as_deref() == Ok(stage) {
        std::process::exit(86);
    }
}
