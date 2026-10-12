//! Two-way connected review application using only the public native SDK.
//!
//! The private app credential creates and controls its own tasks. It is never
//! supplied to a model. Developer policy is configured once; there are no
//! per-action approval prompts. Connect checks current authority on replay.
mod artifacts;
mod bootstrap;
mod decisions;
mod domain;
mod driver;
mod intake;
mod journal;
mod ledger;
mod observations;
pub mod state;
mod workflow;
pub use domain::{Decision, Draft, DraftFile, Policy, Request};
pub use driver::{run, RunOptions};
pub use state::{AppStore, BusinessResult, Statistics};

pub(super) fn fault(point: &str) {
    #[cfg(feature = "test-support")]
    if std::env::var("CONNECT_CONNECTED_APP_FAULT").as_deref() == Ok(point) {
        std::process::exit(86);
    }
    #[cfg(not(feature = "test-support"))]
    let _ = point;
}

#[cfg(test)]
mod tests;
