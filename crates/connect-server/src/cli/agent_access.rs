//! Install existing agent communication credentials; task grants remain optional.
mod arguments;
mod codex;
pub mod diagnostics;
mod host;
mod identity;
mod preflight;
mod state;

pub use arguments::Command;
pub use codex::export;
pub use codex::remove;
pub use host::run;
pub use identity::create;

#[cfg(test)]
mod tests;
