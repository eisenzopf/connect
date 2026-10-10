//! Local server composition and command-line operations.
pub mod cli;
pub mod config;
pub mod installation;
mod media_ingress;
mod media_lifecycle;
pub mod media_profile;
mod media_transports;
pub mod runtime;
mod sip_outbound;
mod sip_registration;
pub mod tls;
mod transports;
mod vapi_media;
mod vapi_observer;

pub mod providers;

pub mod voice;

mod sms_authority;
mod worker;
