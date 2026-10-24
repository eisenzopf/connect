//! Current public readiness from actual origins and committed observations.
//! Cached origins carry metadata only, never an IO grant or retained reader.
use super::*;
use crate::{NativeConnectionFlowFact, NativeMediaOrigin};
use std::sync::Arc;

#[derive(Clone)]
pub(crate) enum ReadyOrigin {
    Native(Arc<NativeMediaOrigin>),
    InitialTelephone(Arc<crate::InitialTelephoneMediaOrigin>),
    Telephone(Arc<crate::TelephoneMediaOrigin>),
}
impl ReadyOrigin {
    fn id(&self) -> &str {
        match self {
            Self::Native(o) => o.connection_id(),
            Self::InitialTelephone(o) => o.connection_id(),
            Self::Telephone(o) => &o.snapshot().connection_id,
        }
    }
    fn session(&self) -> &str {
        match self {
            Self::Native(o) => o.session_id(),
            Self::InitialTelephone(o) => &o.source().session_id,
            Self::Telephone(o) => &o.snapshot().session_id,
        }
    }
    fn version(&self) -> (u64, u64) {
        match self {
            Self::Native(o) => (o.generation(), o.credential_epoch()),
            Self::InitialTelephone(_) => (1, 0),
            Self::Telephone(o) => (o.snapshot().generation, 0),
        }
    }
}
/// Validated metadata only. It cannot acquire a reader, admit media or call IO.
pub(crate) struct ReadyConnection {
    pub connection: SessionActiveConnection,
    pub epoch: u64,
    pub observed_at_millis: i64,
    pub capabilities: serde_json::Value,
    pub expires_at: i64,
}
impl Store {
    pub(crate) fn remember_session_ready_origin(
        &self,
        c: &Connection,
        origin: &NativeMediaOrigin,
    ) -> Result<()> {
        if self
            .session_ready_origins
            .lock()
            .map_err(|_| Error::storage())?
            .get(origin.connection_id())
            .is_some_and(|previous| {
                previous.version() >= (origin.generation(), origin.credential_epoch())
            })
        {
            return Ok(());
        }
        self.remember_session_ready(c, ReadyOrigin::Native(Arc::new(origin.clone())))
    }
    pub(crate) fn remember_session_ready(&self, c: &Connection, origin: ReadyOrigin) -> Result<()> {
        let mut origins = self
            .session_ready_origins
            .lock()
            .map_err(|_| Error::storage())?;
        if let Some(previous) = origins.get(origin.id()) {
            if previous.version() >= origin.version() {
                return Ok(());
            }
        } else if origins.len() >= self.limits.connections {
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
        origins.insert(origin.id().into(), origin);
        Ok(())
    }
}
fn native(
    c: &Connection,
    store: &Store,
    origin: &NativeMediaOrigin,
) -> Result<Option<ReadyConnection>> {
    let row: Option<(String,Option<String>)> = c.query_row("SELECT f.fact_json,json_extract(n.readiness_json,'$.codec_bindings') FROM native_connection_flow_facts f JOIN connections n ON n.id=f.connection_id AND n.generation=f.generation AND n.observed_runtime_id=f.runtime_epoch WHERE f.connection_id=?1 AND f.generation=?2 AND f.runtime_epoch=?3 AND f.runtime_connection_id=?4 AND f.participant_id=?5 AND f.membership_epoch=?6 AND n.state IN ('ready','active') AND n.readiness_json IS NOT NULL", params![origin.connection_id(),origin.generation(),store.runtime_epoch(),origin.runtime_connection_id(),origin.participant_id(),origin.membership_epoch()], |r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_| Error::storage())?;
    let Some((raw, bindings)) = row else {
        return Ok(None);
    };
    let flow: NativeConnectionFlowFact =
        serde_json::from_str(&raw).map_err(|_| Error::storage())?;
    let capabilities = if let Some(raw) = bindings {
        let bindings: Vec<connect_protocol::native_streams::CodecBinding> =
            serde_json::from_str(&raw).map_err(|_| Error::storage())?;
        connect_protocol::native_streams::validate_codec_bindings(&bindings)?;
        if bindings
            .iter()
            .any(|b| b.connection_generation != origin.generation())
        {
            return Err(Error::forbidden());
        }
        let mut out = serde_json::json!({});
        for binding in bindings {
            let key = match binding.medium.as_str() {
                "audio" => "audio_codecs",
                "video" | "screen" => "video_codecs",
                "data" => "data_protocols",
                _ => return Err(Error::forbidden()),
            };
            let mut codec = binding.codec;
            let value = if binding.medium == "data" {
                codec["name"].clone()
            } else {
                if let Some(name) = codec["name"].as_str() {
                    codec["name"] = serde_json::json!(match name {
                        "pcmu" => "g.711-mu",
                        "pcma" => "g.711-a",
                        name => name,
                    });
                }
                let params = codec["params"].as_object_mut().ok_or_else(Error::storage)?;
                params.remove("payload_type");
                params.entry("fmtp").or_insert(serde_json::Value::Null);
                codec
            };
            let values = out
                .as_object_mut()
                .ok_or_else(Error::storage)?
                .entry(key)
                .or_insert_with(|| serde_json::json!([]))
                .as_array_mut()
                .ok_or_else(Error::storage)?;
            if !values.contains(&value) {
                values.push(value);
            }
        }
        out
    } else {
        flow.negotiated_capabilities
    };
    let (view, expires_at, public_handoff) =
        store.native_session_readiness_projection(c, origin)?;
    if !public_handoff
        && (view.private_consultation.is_some()
            || view.handoff_preparation
            || view.publication_suspended
            || crate::connection_control::private_consultation::private_partner(
                c,
                origin.connection_id(),
            )?
            .is_some())
    {
        return Ok(None);
    }
    Ok(Some(ReadyConnection {
        connection: SessionActiveConnection {
            connid: origin.connection_id().into(),
            participant_id: origin.participant_id().into(),
            transport: "quic".into(),
        },
        epoch: origin.membership_epoch(),
        observed_at_millis: if public_handoff {
            chrono::Utc::now().timestamp_millis()
        } else {
            flow.observed_at_millis
        },
        capabilities,
        expires_at,
    }))
}
/// Negative metadata preflight only: a positive result still requires every
/// actual origin and current authorization below. Do not serialize packet IO
/// behind expensive authority/capability projection for an impossible start.
pub(super) fn observed_coverage(c: &Connection, store: &Store, sid: &str) -> Result<bool> {
    c.query_row("SELECT NOT EXISTS(SELECT 1 FROM session_participants sp JOIN participants p ON p.participant_id=sp.participant_id AND p.membership_epoch=sp.membership_epoch AND p.active=1 JOIN identities identity ON identity.id=p.identity_id AND identity.revoked=0 WHERE sp.session_id=?1 AND sp.state='present' AND NOT EXISTS(SELECT 1 FROM connections n WHERE n.session_id=sp.session_id AND n.participant_id=sp.participant_id AND n.membership_epoch=sp.membership_epoch AND n.state IN ('ready','active') AND n.observed_runtime_id=?2 AND n.readiness_json IS NOT NULL AND (EXISTS(SELECT 1 FROM native_connection_flow_facts f WHERE f.connection_id=n.id AND f.generation=n.generation AND f.runtime_epoch=?2) OR EXISTS(SELECT 1 FROM telephone_connection_lifetimes l WHERE l.connection_id=n.id AND l.generation=n.generation AND l.runtime_epoch=?2 AND l.retired=0) OR EXISTS(SELECT 1 FROM initial_telephone_operations initial JOIN operations o ON o.record_id=initial.operation_record_id JOIN initial_telephone_answers a ON a.operation_record_id=o.record_id WHERE initial.parent_kind='invitation' AND o.connection_id=n.id AND n.generation=1 AND initial.attempt_runtime=?2 AND a.connected_at IS NOT NULL AND a.retired_at IS NULL))))",params![sid,store.runtime_epoch()],|r|r.get(0)).map_err(|_|Error::storage())
}
pub(super) fn current(c: &Connection, store: &Store, sid: &str) -> Result<Vec<ReadyConnection>> {
    let origins = store
        .session_ready_origins
        .lock()
        .map_err(|_| Error::storage())?
        .values()
        .filter(|o| o.session() == sid)
        .cloned()
        .collect::<Vec<_>>();
    let mut ready = vec![];
    for origin in origins {
        let value = match origin {
            ReadyOrigin::Native(origin) => native(c, store, &origin),
            ReadyOrigin::InitialTelephone(origin) => store
                .initial_telephone_session_readiness(c, &origin)
                .map(Some),
            ReadyOrigin::Telephone(origin) => {
                store.telephone_session_readiness(c, &origin).map(Some)
            }
        };
        match value {
            Ok(Some(value)) => ready.push(value),
            Ok(None) => (),
            Err(error) if error.code != "storage-unavailable" => (),
            Err(error) => return Err(error),
        }
    }
    Ok(ready)
}
pub(super) fn capabilities(ready: &[ReadyConnection]) -> serde_json::Value {
    let Some(first) = ready.first() else {
        return serde_json::json!({});
    };
    if ready.iter().all(|r| r.capabilities == first.capabilities) {
        return first.capabilities.clone();
    }
    // Only actually common selected descriptors are global capabilities.
    // A gateway can transcode distinct formats; preserve their attribution
    // without presenting a union as a common endpoint agreement.
    let mut out = first.capabilities.as_object().cloned().unwrap_or_default();
    out.retain(|key, value| {
        if let Some(values) = value.as_array_mut() {
            values.retain(|value| {
                ready.iter().all(|r| {
                    r.capabilities
                        .get(key)
                        .and_then(|v| v.as_array())
                        .is_some_and(|values| values.contains(value))
                })
            });
            true
        } else {
            ready.iter().all(|r| r.capabilities.get(key) == Some(value))
        }
    });
    out.insert(
        "connections".into(),
        serde_json::json!(ready
            .iter()
            .map(
                |r| serde_json::json!({"connid":r.connection.connid,"capabilities":r.capabilities})
            )
            .collect::<Vec<_>>()),
    );
    serde_json::Value::Object(out)
}
