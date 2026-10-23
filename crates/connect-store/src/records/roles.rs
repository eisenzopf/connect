//! Current explicit tenant-role grants. Only protected provisioning may write;
//! its target Identity writer fences record deliveries through actual IO.
use super::*;
use connect_protocol::provisioning::ProvisioningOutcome;
pub(crate) fn outcome(c: &Connection, identity: &str) -> Result<ProvisioningOutcome> {
    c.query_row("SELECT tenant_id,role,private_sources,active,revision FROM tenant_record_readers WHERE identity_id=?1",[identity],|r|Ok(ProvisioningOutcome::RecordReadRole{identity_id:identity.into(),tenant_id:r.get(0)?,role:r.get(1)?,private_sources:r.get(2)?,active:r.get(3)?,revision:r.get(4)?})).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)
}
pub(crate) fn configure(
    c: &Connection,
    identity: &str,
    tenant: &str,
    role: &str,
    private_sources: bool,
    active: bool,
    expected: Option<u64>,
) -> Result<ProvisioningOutcome> {
    let prior: Option<u64> = c
        .query_row(
            "SELECT revision FROM tenant_record_readers WHERE identity_id=?1 AND tenant_id=?2",
            params![identity, tenant],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    if prior != expected || prior.is_some_and(|r| r >= MAX_REVISION) {
        return Err(conflict());
    }
    match prior {
        None => {
            c.execute(
                "INSERT INTO tenant_record_readers VALUES(?1,?2,?3,?4,?5,1)",
                params![identity, tenant, role, private_sources, active],
            )
            .map_err(|_| Error::storage())?;
        }
        Some(revision) => {
            c.execute("UPDATE tenant_record_readers SET role=?2,private_sources=?3,active=?4,revision=?5 WHERE identity_id=?1",params![identity,role,private_sources,active,revision+1]).map_err(|_|Error::storage())?;
        }
    }
    outcome(c, identity)
}
