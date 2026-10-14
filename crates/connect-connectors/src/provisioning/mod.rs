//! Explicit resource selection and recoverable provisioning through injected adapters.
//! Qualification uses local fixtures; this API itself performs no provider I/O.
mod catalog;
mod discovery;
mod execution;
mod readiness;
pub use catalog::*;
pub use discovery::{discover_selected_clients, discover_with_clients};
pub use execution::*;
pub use readiness::*;
