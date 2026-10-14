//! Exercise the upstream transport fixtures through the exact Connect Cargo.lock.
//! The sustained encoded-rate fixture is a transport guard, not decoded AV qualification.
// Generated upstream sources are formatted before their patch digest is recorded.
#[allow(clippy::unwrap_used)] // Imported fixture assertions deliberately panic.
#[rustfmt::skip]
#[path = "../../../.deps/rvoip/crates/uctp/rvoip-quic/tests/video_budget.rs"]
mod budget;
// Generated upstream sources are formatted before their patch digest is recorded.
#[allow(clippy::unwrap_used)] // Imported fixture assertions deliberately panic.
#[rustfmt::skip]
#[path = "../../../.deps/rvoip/crates/uctp/rvoip-quic/tests/video_sustained.rs"]
mod sustained;
