//! Historical audience observations retain no authority reader and never replay IO.
use super::*;
use std::collections::BTreeMap;
#[derive(Clone)]
pub(super) struct Origin {
    connection: String,
    generation: u64,
    revision: u64,
}
pub struct NativeReplacementAudienceCapture {
    record: String,
    runtime: String,
    phase: &'static str,
    values: BTreeMap<String, (String, u64, u64, u64)>,
}
redacted!(NativeReplacementAudienceCapture);
impl NativeReplacementSceneRead {
    pub fn capture_activation_revisions(
        &self,
        observed: BTreeMap<String, u64>,
    ) -> Result<NativeReplacementAudienceCapture> {
        self.capture_revisions(observed, "activate")
    }
    pub fn capture_commit_revisions(
        &self,
        mut observed: BTreeMap<String, u64>,
    ) -> Result<NativeReplacementAudienceCapture> {
        observed.remove(&self.media.source.context().source_runtime_connection_id);
        self.capture_revisions(observed, "commit")
    }
    fn capture_revisions(
        &self,
        observed: BTreeMap<String, u64>,
        phase: &'static str,
    ) -> Result<NativeReplacementAudienceCapture> {
        let mut origins = self.audience_sources.clone();
        if phase == "commit" {
            origins.remove(&self.media.source.context().source_runtime_connection_id);
        }
        if origins.len() != observed.len() {
            return Err(conflict());
        }
        let mut values = BTreeMap::new();
        for (runtime, origin) in origins {
            let after = observed.get(&runtime).copied().ok_or_else(conflict)?;
            if origin.revision.checked_add(1) != Some(after) || after > 9_007_199_254_740_991 {
                return Err(conflict());
            }
            values.insert(
                runtime,
                (origin.connection, origin.generation, origin.revision, after),
            );
        }
        let origin = &self.media.source.selection().origin;
        Ok(NativeReplacementAudienceCapture {
            record: origin.record.clone(),
            runtime: origin.runtime.clone(),
            phase,
            values,
        })
    }
}
impl NativeReplacementAudienceCapture {
    pub(super) fn require(&self, record: &str, runtime: &str, phase: &str) -> Result<()> {
        if self.record != record || self.runtime != runtime || self.phase != phase {
            return Err(Error::forbidden());
        }
        Ok(())
    }
    pub(super) fn after(&self, runtime: &str) -> Option<u64> {
        self.values.get(runtime).map(|v| v.3)
    }
    fn json(&self) -> Result<String> {
        crate::provisioning::canonical_json(&serde_json::json!({"sources":self.values}))
    }
    pub(super) fn require_recorded(&self, c: &Connection, effect: &str) -> Result<()> {
        let saved:Option<(String,String,String)>=c.query_row("SELECT runtime_epoch,effect_id,capture_json FROM native_replacement_audiences WHERE operation_record_id=?1 AND phase=?2",params![self.record,self.phase],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
        if saved != Some((self.runtime.clone(), effect.into(), self.json()?)) {
            return Err(conflict());
        }
        Ok(())
    }
    pub(super) fn record(&self, c: &Connection, store: &Store, effect: &str) -> Result<()> {
        if self.runtime != store.runtime_epoch() {
            return Err(Error::forbidden());
        }
        let body = self.json()?;
        if body.len() > 262144 {
            return Err(Error::new(
                "capacity-exceeded",
                "Audience revision capture exceeds its bound",
            ));
        }
        c.execute("INSERT INTO native_replacement_audiences(operation_record_id,phase,effect_id,runtime_epoch,capture_json) VALUES(?1,?2,?3,?4,?5)",params![self.record,self.phase,effect,self.runtime,body]).map_err(|_|Error::storage())?;
        for (connection, generation, before, after) in self.values.values() {
            // No historical metadata can revive a retired incarnation or
            // overwrite a later independently recorded native observation.
            c.execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.stream_revision',?4) WHERE id=?1 AND generation=?2 AND observed_runtime_id=?3 AND state IN ('ready','active') AND json_extract(readiness_json,'$.stream_revision')=?5 AND (EXISTS(SELECT 1 FROM native_connection_lifetimes l WHERE l.connection_id=connections.id AND l.generation=connections.generation AND l.runtime_epoch=?3 AND l.retired=0) OR EXISTS(SELECT 1 FROM telephone_connection_lifetimes l WHERE l.connection_id=connections.id AND l.generation=connections.generation AND l.runtime_epoch=?3 AND l.retired=0))",params![connection,generation,self.runtime,after,before]).map_err(|_|Error::storage())?;
        }
        Ok(())
    }
}
pub(super) fn telephone_origin(
    c: &Connection,
    phone: &crate::TelephoneMediaRead,
    values: &mut BTreeMap<String, Origin>,
) -> Result<()> {
    if !phone.current() {
        return Err(Error::forbidden());
    }
    let s = phone.snapshot();
    let revision:u64=c.query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1 AND generation=?2",params![s.connection_id,s.generation],|r|r.get(0)).map_err(|_|Error::storage())?;
    if values
        .insert(
            s.runtime_connection_id.clone(),
            Origin {
                connection: s.connection_id.clone(),
                generation: s.generation,
                revision,
            },
        )
        .is_some()
    {
        return Err(Error::forbidden());
    }
    Ok(())
}
pub(super) fn origins(
    c: &Connection,
    sources: &[Arc<crate::NativeMediaOrigin>],
    target: &PeerConnectionProjection,
    record: &str,
) -> Result<BTreeMap<String, Origin>> {
    let mut values = BTreeMap::new();
    for source in sources {
        let revision:u64=c.query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1 AND generation=?2",params![source.connection_id(),source.generation()],|r|r.get(0)).map_err(|_|Error::storage())?;
        values.insert(
            source.runtime_connection_id().into(),
            Origin {
                connection: source.connection_id().into(),
                generation: source.generation(),
                revision,
            },
        );
    }
    let captured:Option<String>=c.query_row("SELECT capture_json FROM native_replacement_audiences WHERE operation_record_id=?1 AND phase='activate'",[record],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
    let revision = if let Some(captured) = captured {
        serde_json::from_str::<serde_json::Value>(&captured).map_err(|_| Error::storage())?
            ["sources"][&target.runtime_connection_id][3]
            .as_u64()
            .ok_or_else(Error::storage)?
    } else {
        2
    };
    values.insert(
        target.runtime_connection_id.clone(),
        Origin {
            connection: target.connection.connection_id.clone(),
            generation: target.connection.generation,
            revision,
        },
    );
    Ok(values)
}
