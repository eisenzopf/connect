//! A fresh export of the fixture's existing grant, without model credentials.
#![allow(clippy::unwrap_used)]
use connect_protocol::tasks::*;
use connect_server::cli::{
    init::private_write,
    task_access::{codex, GrantFile},
};
use std::path::PathBuf;

pub fn export(f: &super::fixture::Fixture, issuance_id: &str) -> PathBuf {
    let grant = ProtectedTaskGrant::new(f.issued.grant.clone(), f.issued.expose_secret().into());
    let file = GrantFile {
        profile: TASK_ACCESS_PROFILE.into(),
        request: TaskAccessRequest::new(TaskAccessCommand::Issue {
            request_id: issuance_id.into(),
            definition: GrantDefinition {
                task_id: grant.grant.task_id.clone(),
                binding: grant.grant.binding.clone(),
                operations: grant.grant.operations.clone(),
                expires_at: grant.grant.expires_at,
            },
        }),
        grant: Some(grant),
    };
    let source = f.directory().join("grant.json");
    private_write(&source, &serde_json::to_vec(&file).unwrap()).unwrap();
    let output = f.directory().join("host");
    codex::export(&source, f.server.mcp_address.unwrap(), &output).unwrap();
    std::fs::canonicalize(output).unwrap()
}
