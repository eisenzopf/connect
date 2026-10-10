//! Task-authenticated MCP adapter over the Connect application service.
//!
//! The official MCP SDK owns JSON-RPC and Streamable HTTP. This crate owns
//! task-grant admission and tool projection. It has no direct database access
//! and never accepts an owner or verified invocation binding from model input.
//! Trusted server composition supplies the authenticator and verified binding.
#![doc = include_str!("../README.md")]
mod arguments;
mod auth;
mod budget;
mod catalog;
mod commands;
mod handler;
mod listener;
mod schemas;

pub use auth::{FixedBindingAuth, LocalGrantAuth, McpAuthenticator};
pub use listener::RunningMcp;
