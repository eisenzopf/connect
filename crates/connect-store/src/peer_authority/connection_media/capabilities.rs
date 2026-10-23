//! Descriptor notifications use original source facts and fresh joint policy.
use super::*;

fn latest(
    store: &Store,
    c: &Connection,
    source: &NativeMediaOrigin,
    record: &str,
    payload: &serde_json::Value,
) -> Result<bool> {
    current_owner(store, source)?;
    super::credential_refresh::require_epoch(store, c, source)?;
    source.attempt.current(c, store)?;
    let raw: Option<String> = c.query_row(
        "SELECT json_extract(c.readiness_json,'$.advertised_capabilities') FROM connections c JOIN operations o ON o.record_id=?2 WHERE c.id=?1 AND c.generation=?3 AND c.state IN ('ready','active') AND c.observed_runtime_id=?4 AND o.connection_id=c.id AND o.conversation_id=?5 AND o.session_id=?6 AND o.command_type='capability.advertise' AND o.state='completed'",
        params![source.connection_id(),record,source.generation(),store.runtime_epoch(),source.conversation_id(),source.session_id()],
        |row| row.get::<_,Option<String>>(0),
    ).optional().map_err(|_|Error::storage())?.flatten();
    let Some(raw) = raw else {
        return Ok(false);
    };
    let value: serde_json::Value = serde_json::from_str(&raw).map_err(|_| Error::storage())?;
    Ok(value["operation_record_id"] == record
        && value["by_participant"] == source.participant_id()
        && value["by_participant"] == payload["by_participant"]
        && value["capabilities"] == payload["capabilities"]
        && value["trigger"] == payload["trigger"])
}

impl Store {
    /// Passive coalescing check only, never a permission grant. Call while the
    /// queue's own metadata lock is held, after the completing SQL transaction
    /// has released its lock. This prevents delayed old enqueue from replacing
    /// a newer descriptor. Actual IO must acquire fresh joint authority below.
    pub fn native_capability_is_latest(
        &self,
        source: &NativeMediaOrigin,
        record: &str,
        payload: &serde_json::Value,
    ) -> Result<bool> {
        let c = self.try_connection()?;
        latest(self, &c, source, record, payload)
    }

    /// Fresh original source and exact current recipient fences are acquired
    /// together, outside engine locks. Keep the returned read through actual
    /// reliable IO. Core additionally enforces original/current private groups.
    pub fn try_native_capability_delivery_read(
        &self,
        target: &NativeMediaOrigin,
        source: &NativeMediaOrigin,
        record: &str,
        payload: &serde_json::Value,
    ) -> Result<NativeMediaRead> {
        let read = self.try_native_media_read(target, Some(source))?;
        let c = self.try_connection()?;
        // The joint media scope includes this exact original credential's
        // current Identity/key/role intervals. No second reader is acquired.
        self.native_media_route_actor(&c, source)?
            .require("capability.advertise")?;
        if !read.current() || !latest(self, &c, source, record, payload)? {
            return Err(Error::forbidden());
        }
        Ok(read)
    }
}
