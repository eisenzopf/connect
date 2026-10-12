//! Identity communication and optional task-authenticated MCP over Connect.
//!
//! The official MCP SDK owns JSON-RPC and Streamable HTTP. This crate owns
//! current credential/task-grant admission and tool projection. It has no direct database access
//! and never accepts an owner or verified invocation binding from model input.
//! Trusted server composition supplies the authenticator and verified binding.
#![doc = include_str!("../README.md")]
mod arguments;
mod auth;
mod budget;
mod capacity;
mod catalog;
mod commands;
mod execution;
mod handler;
mod identity_auth;
mod identity_catalog;
mod identity_commands;
mod identity_schemas;
mod listener;
mod schemas;
mod supervision;
mod vapi_auth;

pub use auth::{FixedBindingAuth, LocalGrantAuth, McpAccess, McpAuthenticator};
pub use capacity::{McpCapacity, MAX_MCP_LISTENERS, MAX_MCP_REQUESTS};
pub use identity_auth::LocalAuth;
pub use listener::RunningMcp;
pub use vapi_auth::{VapiBindingAuth, VapiOrigin};
