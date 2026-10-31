//! Read-only administrator diagnostics. Never project operation IDs or content.
use connect_protocol::{
    Error, OperationCounts, OperationalHealth, OperationalLimits, Result, MAX_OPERATIONAL_COUNTER,
};
use rusqlite::Connection;
pub(crate) fn snapshot(
    c: &Connection,
    limits: &connect_protocol::Limits,
) -> Result<OperationalHealth> {
    let mut counts = OperationCounts::default();
    let mut query = c
        .prepare("SELECT state,COUNT(*) FROM operations GROUP BY state")
        .map_err(|_| Error::storage())?;
    let rows = query
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })
        .map_err(|_| Error::storage())?;
    for row in rows {
        let (state, count) = row.map_err(|_| Error::storage())?;
        let count = count.min(MAX_OPERATIONAL_COUNTER);
        counts.retained = counts
            .retained
            .saturating_add(count)
            .min(MAX_OPERATIONAL_COUNTER);
        match state.as_str() {
            "accepted" => counts.accepted = count,
            "preparing" => counts.preparing = count,
            "ready" => counts.ready = count,
            "committing" => counts.committing = count,
            "interrupted" => counts.interrupted = count,
            "unknown" => counts.unknown = count,
            "completed" | "failed" | "cancelled" => (),
            _ => return Err(Error::storage()),
        }
    }
    Ok(OperationalHealth {
        observed_at: crate::now(),
        limits: OperationalLimits::from(limits),
        operations: counts,
        runtime: None,
        resource_pools: None,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{Scope, Store};
    use connect_protocol::{Limits, Operation, Outcome};
    #[test]
    fn unsupported_limit_refuses_before_creating_state_or_taking_ownership() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.sqlite3");
        let limits = Limits {
            operation_receipts: MAX_OPERATIONAL_COUNTER + 1,
            ..Default::default()
        };
        assert_eq!(
            Store::initialize(&path, limits.clone()).err().unwrap().code,
            "invalid-config"
        );
        assert!(!path.exists());
        assert_eq!(
            Store::open(&path, limits).err().unwrap().code,
            "invalid-config"
        );
        assert!(!path.exists());
    }
    #[test]
    fn health_counts_only_metadata_and_requires_current_admin_authority() {
        let temp = tempfile::tempdir().unwrap();
        let store =
            Store::initialize(&temp.path().join("state.sqlite3"), Limits::default()).unwrap();
        let identity = store.create_identity("diagnostic-owner").unwrap();
        let admin = store
            .issue(&identity.id, Scope::operator(), crate::now() + 60)
            .unwrap();
        let reader = store
            .issue(&identity.id, Scope::messaging(vec![]), crate::now() + 60)
            .unwrap();
        let limited_admin = store
            .issue(
                &identity.id,
                Scope {
                    operations: vec!["health".into(), "admin".into()],
                    recipients: vec![identity.id.clone()],
                },
                crate::now() + 60,
            )
            .unwrap();
        let secret = "private-diagnostic-intent-must-never-appear";
        {
            let c = store.connection().unwrap();
            let tenant: String = c
                .query_row(
                    "SELECT tenant_id FROM identities WHERE id=?1",
                    [&identity.id],
                    |r| r.get(0),
                )
                .unwrap();
            for state in [
                "accepted",
                "preparing",
                "ready",
                "committing",
                "interrupted",
                "unknown",
                "completed",
            ] {
                c.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,state,intent_json,created_at,updated_at) VALUES(?1,?2,?3,?1,?4,'session.handoff',?5,?6,?7,?7)",
                    rusqlite::params![format!("diagnostic-{state}"), tenant, identity.id, "0".repeat(64), state, serde_json::json!({"secret":secret}).to_string(), crate::now()]).unwrap();
            }
        }
        let before: u64 = store
            .connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM journal", [], |r| r.get(0))
            .unwrap();
        let observed = store.execute(&admin.id, &Operation::Health {}).unwrap();
        let encoded = serde_json::to_string(&observed).unwrap();
        assert!(!encoded.contains(secret));
        assert!(!encoded.contains("diagnostic-owner"));
        let Outcome::Health {
            operational: Some(snapshot),
            ..
        } = observed
        else {
            panic!("administrator snapshot")
        };
        assert_eq!(snapshot.operations.retained, 7);
        assert_eq!(snapshot.operations.accepted, 1);
        assert_eq!(snapshot.operations.preparing, 1);
        assert_eq!(snapshot.operations.ready, 1);
        assert_eq!(snapshot.operations.committing, 1);
        assert_eq!(snapshot.operations.interrupted, 1);
        assert_eq!(snapshot.operations.unknown, 1);
        assert_eq!(
            snapshot.limits.operation_receipts,
            store.limits().operation_receipts
        );
        assert!(matches!(
            store.execute(&reader.id, &Operation::Health {}).unwrap(),
            Outcome::Health {
                operational: None,
                ..
            }
        ));
        assert!(matches!(
            store
                .execute(&limited_admin.id, &Operation::Health {})
                .unwrap(),
            Outcome::Health {
                operational: None,
                ..
            }
        ));
        let after: u64 = store
            .connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM journal", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, after);
        let actor = store.authenticate(&admin.token).unwrap();
        store.revoke(&actor, &admin.id).unwrap();
        assert_eq!(
            store
                .execute(&admin.id, &Operation::Health {})
                .unwrap_err()
                .code,
            "unauthorized"
        );
    }
}
