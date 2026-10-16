//! Local server composition and command-line operations.
pub mod cli;
pub mod config;
pub mod installation;
mod media_diagnostics;
mod media_ingress;
mod media_lifecycle;
pub mod media_profile;
mod media_transports;
mod registration_monitor;
pub mod runtime;
mod sip_outbound;
mod sip_registration;
pub mod tls;
mod transports;
/// Bounded loopback HTTP facade over the shared application service.
pub use transports::http::Listener as HttpListener;
pub mod vapi_bootstrap;
mod vapi_media;
mod vapi_observer;
pub mod vapi_task_setup;

pub mod providers;
pub mod secrets;
pub mod setup_vault;

pub mod voice;

mod sms_authority;
pub use sms_authority::{
    synchronize as synchronize_sms_policy, Authority as HttpSmsPolicy, SmsPolicy,
};
mod startup;
mod worker;
