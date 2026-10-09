//! Local server composition and command-line operations.
pub mod cli;
pub mod config;
pub mod installation;
mod media_ingress;
mod media_lifecycle;
mod media_transports;
pub mod runtime;
mod sip_outbound;
pub mod tls;
mod transports;
mod vapi_media;

pub mod providers;

pub mod voice;

mod worker;
