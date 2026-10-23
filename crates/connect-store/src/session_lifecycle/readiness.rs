//! Current public readiness from actual origins and committed flow facts.
//! Cached origins carry metadata only, never an IO grant or retained reader.
use super::*;
use std::sync::Arc;

impl Store {
    pub(crate) fn remember_session_ready_origin(
        &self,
        c: &Connection,
        origin: &NativeMediaOrigin,
    ) -> Result<()> {
        let mut origins = self
            .session_ready_origins
            .lock()
            .map_err(|_| Error::storage())?;
        if let Some(previous) = origins.get(origin.connection_id()) {
            if previous.generation() > origin.generation()
                || (previous.generation() == origin.generation()
                    && previous.credential_epoch() >= origin.credential_epoch())
            {
                return Ok(());
            }
        } else if origins.len() >= self.limits.connections {
            // Retired Connections do not consume the configured live bound.
            // Never evict a current origin merely to guess another readiness.
            let mut q = c.prepare("SELECT id FROM connections WHERE observed_runtime_id=?1 AND state IN ('ready','active')").map_err(|_| Error::storage())?;
            let live = q
                .query_map([self.runtime_epoch()], |r| r.get::<_, String>(0))
                .map_err(|_| Error::storage())?
                .collect::<std::result::Result<std::collections::BTreeSet<_>, _>>()
                .map_err(|_| Error::storage())?;
            origins.retain(|id, _| live.contains(id));
            if origins.len() >= self.limits.connections {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Session readiness origin capacity exhausted",
                ));
            }
        }
        origins.insert(origin.connection_id().into(), Arc::new(origin.clone()));
        Ok(())
    }
}

pub(super) fn native(
    c: &Connection,
    store: &Store,
    sid: &str,
) -> Result<Vec<(String, u64, SessionActiveConnection)>> {
    let origins = store
        .session_ready_origins
        .lock()
        .map_err(|_| Error::storage())?
        .values()
        .filter(|origin| origin.session_id() == sid)
        .cloned()
        .collect::<Vec<_>>();
    let mut ready = Vec::new();
    for origin in origins {
        let observed: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM native_connection_flow_facts f JOIN connections n ON n.id=f.connection_id AND n.generation=f.generation AND n.observed_runtime_id=f.runtime_epoch WHERE f.connection_id=?1 AND f.generation=?2 AND f.runtime_epoch=?3 AND f.runtime_connection_id=?4 AND f.participant_id=?5 AND f.membership_epoch=?6 AND n.state IN ('ready','active') AND n.readiness_json IS NOT NULL)", params![origin.connection_id(),origin.generation(),store.runtime_epoch(),origin.runtime_connection_id(),origin.participant_id(),origin.membership_epoch()], |r|r.get(0)).map_err(|_| Error::storage())?;
        if !observed {
            continue;
        }
        let view = match store.native_session_readiness_projection(c, &origin) {
            Ok(view) => view,
            Err(error) if error.code != "storage-unavailable" => continue,
            Err(error) => return Err(error),
        };
        if view.private_consultation.is_some()
            || view.handoff_preparation
            || view.publication_suspended
            || crate::connection_control::private_consultation::private_partner(
                c,
                origin.connection_id(),
            )?
            .is_some()
        {
            continue;
        }
        ready.push((
            origin.participant_id().into(),
            origin.membership_epoch(),
            SessionActiveConnection {
                connid: origin.connection_id().into(),
                participant_id: origin.participant_id().into(),
                transport: "quic".into(),
            },
        ));
    }
    Ok(ready)
}
