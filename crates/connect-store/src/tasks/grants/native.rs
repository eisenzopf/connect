//! Native peers receive task authority, never their parent's transport scopes.
use super::{installation_id, revalidate, rows, TaskInvocation};
use crate::Store;
use connect_protocol::{
    tasks::{task_grant_id, task_grant_subject, InvocationKind, TaskPermission},
    Error, Result,
};

/// Non-secret authenticated metadata for trusted transport composition. Every
/// command still revalidates the private invocation inside its own transaction.
#[derive(Clone, Debug)]
pub struct NativeTaskAuthority {
    invocation: TaskInvocation,
    identity_id: String,
    expires_at: i64,
    media_attachment: bool,
}
impl NativeTaskAuthority {
    /// Current task handle; private fields prevent model-selected authority.
    pub fn invocation(&self) -> &TaskInvocation {
        &self.invocation
    }
    /// Backing identity for assurance, without exporting its credential.
    pub fn identity_id(&self) -> &str {
        &self.identity_id
    }
    /// Signed grant expiry, already bounded by current task/parent authority.
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
    /// Whether the signed operation ceiling permits session participation.
    /// This is discovery metadata only; resource use rechecks current authority.
    pub fn permits_media_attachment(&self) -> bool {
        self.media_attachment
    }
    /// Distinct expected transport subject, containing no bearer plaintext.
    pub fn subject(&self) -> Result<String> {
        task_grant_subject(self.invocation.grant_id())
    }
}
impl Store {
    /// Verify a signed registered Native grant. Host/provider grants require
    /// their own adapters and cannot authenticate a generic SDK connection.
    pub fn authenticate_native_task_grant(&self, token: &str) -> Result<NativeTaskAuthority> {
        let invocation = self.authenticate_local_task_grant(token)?;
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        native_authority(&tx, invocation)
    }
    /// Revalidate a transport-authenticated reserved subject. Trusted service
    /// composition must check its issuer/installation first; never call this
    /// with a model-selected subject or use it as bearer authentication.
    pub fn revalidate_native_task_principal(&self, subject: &str) -> Result<NativeTaskAuthority> {
        let id = task_grant_id(subject)?;
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        native_authority(&tx, invocation_for_subject(&tx, id)?)
    }
}
pub(super) fn invocation_for_subject(c: &rusqlite::Connection, id: &str) -> Result<TaskInvocation> {
    let row = rows::get(c, id)?;
    if row.grant.binding.kind != InvocationKind::Native {
        return Err(Error::forbidden());
    }
    Ok(TaskInvocation {
        grant_id: id.into(),
        task_id: row.grant.task_id,
        binding: row.grant.binding,
        installation_id: installation_id(c)?,
    })
}
fn native_authority(
    c: &rusqlite::Connection,
    invocation: TaskInvocation,
) -> Result<NativeTaskAuthority> {
    if invocation.binding.kind != InvocationKind::Native {
        return Err(Error::forbidden());
    }
    let (row, actor, _) = revalidate(c, &invocation, crate::now())?;
    Ok(NativeTaskAuthority {
        invocation,
        identity_id: actor.identity_id,
        expires_at: row.grant.expires_at,
        media_attachment: row.grant.operations.contains(&TaskPermission::SessionGet)
            && (row
                .grant
                .operations
                .contains(&TaskPermission::SessionInvite)
                || row
                    .grant
                    .operations
                    .contains(&TaskPermission::SessionAccept)),
    })
}
