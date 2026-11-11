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
    candidate_only: Option<String>,
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
    /// Exact Core observation for a muted candidate-only private cut. Every
    /// outside controller keeps its original revision; only this target advances.
    pub fn capture_candidate_only_activation_revisions(
        &self,
        observed: BTreeMap<String, (u64, u64)>,
    ) -> Result<NativeReplacementAudienceCapture> {
        if observed.len() != self.audience_sources.len() {
            return Err(conflict());
        }
        let target = self.media.target.runtime_connection_id.as_str();
        let mut values = BTreeMap::new();
        for (runtime, origin) in &self.audience_sources {
            let (before, after) = observed.get(runtime).copied().ok_or_else(conflict)?;
            let expected = if runtime == target {
                before.checked_add(1)
            } else {
                Some(before)
            };
            if before != origin.revision || expected != Some(after) || after > 9_007_199_254_740_991
            {
                return Err(conflict());
            }
            values.insert(
                runtime.clone(),
                (origin.connection.clone(), origin.generation, before, after),
            );
        }
        if !values.contains_key(target) {
            return Err(conflict());
        }
        let origin = &self.media.source.selection().origin;
        Ok(NativeReplacementAudienceCapture {
            record: origin.record.clone(),
            runtime: origin.runtime.clone(),
            phase: "activate",
            candidate_only: Some(target.to_owned()),
            values,
        })
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
            candidate_only: None,
            values,
        })
    }
}
impl NativeReplacementAudienceCapture {
    /// Exact current live-original deltas for a sealed historical Commit. The
    /// fresh target is still absent and is not an original Connection UPDATE.
    /// Existing/claimed/retired slots keep Writer custody; a descriptor alone
    /// cannot prove a subsequently appearing candidate row is the same slot.
    pub(crate) fn positive_commit_json(
        &self,
        store: &Store,
        c: &Connection,
        record: &str,
        runtime: &str,
        candidate_connection: &str,
        candidate_runtime: &str,
        candidate_generation: u64,
    ) -> Result<Option<(String, Vec<String>)>> {
        self.require(record, runtime, "commit")?;
        if self.candidate_only.is_some() || runtime != store.runtime_epoch() {
            return Ok(None);
        }
        let Some((id, generation, before, after)) = self.values.get(candidate_runtime) else {
            return Ok(None);
        };
        let dormant:bool=c.query_row("SELECT NOT EXISTS(SELECT 1 FROM connections WHERE id=?1) AND NOT EXISTS(SELECT 1 FROM native_replacement_candidates WHERE operation_record_id=?2 OR candidate_connection_id=?1) AND NOT EXISTS(SELECT 1 FROM native_connection_lifetimes WHERE connection_id=?1 AND retired=0)",params![candidate_connection,record],|r|r.get(0)).map_err(|_|Error::storage())?;
        if id != candidate_connection
            || *generation != candidate_generation
            || before.checked_add(1) != Some(*after)
            || !dormant
        {
            return Ok(None);
        }
        let mut connections = Vec::new();
        let mut unique = std::collections::BTreeSet::new();
        for (source, (id, generation, before, after)) in &self.values {
            if !unique.insert(id) || before.checked_add(1) != Some(*after) {
                return Ok(None);
            }
            if source == candidate_runtime {
                continue;
            }
            let row:Option<String>=c.query_row("SELECT readiness_json FROM connections WHERE id=?1 AND generation=?2 AND observed_runtime_id=?3 AND state IN ('ready','active') AND json_extract(readiness_json,'$.stream_revision')=?4 AND (EXISTS(SELECT 1 FROM native_connection_lifetimes l WHERE l.connection_id=connections.id AND l.generation=connections.generation AND l.runtime_epoch=?3 AND l.retired=0) OR EXISTS(SELECT 1 FROM telephone_connection_lifetimes l WHERE l.connection_id=connections.id AND l.generation=connections.generation AND l.runtime_epoch=?3 AND l.retired=0))",params![id,generation,runtime,before],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
            let Some(old) = row else {
                return Ok(None);
            };
            let next: String = c
                .query_row(
                    "SELECT json_set(?1,'$.stream_revision',?2)",
                    params![old, after],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            let exact: String = c
                .query_row(
                    "SELECT json_array(?1,?2,?3,?4,?5)",
                    params![id, generation, runtime, old, next],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            connections.push(exact);
        }
        Ok(Some((self.json()?, connections)))
    }
    pub(super) fn require(&self, record: &str, runtime: &str, phase: &str) -> Result<()> {
        if self.record != record || self.runtime != runtime || self.phase != phase {
            return Err(Error::forbidden());
        }
        Ok(())
    }
    pub(super) fn after(&self, runtime: &str) -> Option<u64> {
        self.values.get(runtime).map(|v| v.3)
    }
    pub(crate) fn is_candidate_only(&self) -> bool {
        self.phase == "activate" && self.candidate_only.is_some()
    }
    pub(crate) fn candidate_activation_json(
        &self,
        record: &str,
        runtime: &str,
        candidate_runtime: &str,
        candidate_connection: &str,
        generation: u64,
    ) -> Result<String> {
        self.require(record, runtime, "activate")?;
        if self.candidate_only.as_deref() != Some(candidate_runtime) {
            return Err(Error::forbidden());
        }
        let mut changed = 0;
        for (source, (connection, original_generation, before, after)) in &self.values {
            if source == candidate_runtime {
                if connection != candidate_connection
                    || *original_generation != generation
                    || before.checked_add(1) != Some(*after)
                {
                    return Err(Error::forbidden());
                }
                changed += 1;
            } else if before != after {
                return Err(Error::forbidden());
            }
        }
        if changed != 1 {
            return Err(Error::forbidden());
        }
        self.json()
    }
    fn json(&self) -> Result<String> {
        let mut value = serde_json::json!({"sources":self.values});
        if self.is_candidate_only() {
            value["candidate_only"] = serde_json::json!(true);
        }
        crate::provisioning::canonical_json(&value)
    }
    pub(super) fn require_recorded(&self, c: &Connection, effect: &str) -> Result<()> {
        let saved:Option<(String,String,String)>=c.query_row("SELECT runtime_epoch,effect_id,capture_json FROM native_replacement_audiences WHERE operation_record_id=?1 AND phase=?2",params![self.record,self.phase],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
        if saved != Some((self.runtime.clone(), effect.into(), self.json()?)) {
            return Err(conflict());
        }
        Ok(())
    }
    pub(super) fn record(&self, c: &Connection, store: &Store, effect: &str) -> Result<()> {
        self.record_in(c, store, effect, None)
    }
    pub(super) fn record_positive_commit(
        &self,
        c: &Connection,
        store: &Store,
        effect: &str,
    ) -> Result<()> {
        self.require(&self.record, store.runtime_epoch(), "commit")?;
        let candidate:String=c.query_row("SELECT candidate_connection_id FROM native_replacements WHERE operation_record_id=?1",[&self.record],|r|r.get(0)).map_err(|_|Error::storage())?;
        self.record_in(c, store, effect, Some(&candidate))
    }
    fn record_in(
        &self,
        c: &Connection,
        store: &Store,
        effect: &str,
        exact_candidate: Option<&str>,
    ) -> Result<()> {
        if self.runtime != store.runtime_epoch() {
            return Err(Error::forbidden());
        }
        let body = if let Some(candidate) = &self.candidate_only {
            let (connection, generation, _, _) =
                self.values.get(candidate).ok_or_else(Error::forbidden)?;
            let public: bool = c.query_row(
                "SELECT EXISTS(SELECT 1 FROM connections WHERE id=?1 AND state IN ('ready','active','held'))",
                [connection], |row| row.get(0),
            ).map_err(|_| Error::storage())?;
            if public {
                return Err(Error::forbidden());
            }
            self.candidate_activation_json(
                &self.record,
                &self.runtime,
                candidate,
                connection,
                *generation,
            )?
        } else {
            self.json()?
        };
        if body.len() > 262144 {
            return Err(Error::new(
                "capacity-exceeded",
                "Audience revision capture exceeds its bound",
            ));
        }
        c.execute("INSERT INTO native_replacement_audiences(operation_record_id,phase,effect_id,runtime_epoch,capture_json) VALUES(?1,?2,?3,?4,?5)",params![self.record,self.phase,effect,self.runtime,body]).map_err(|_|Error::storage())?;
        if self.is_candidate_only() {
            // The only changed target is still dormant. This receipt changes no
            // published original Connection, readiness, permission or epoch.
            return Ok(());
        }
        for (connection, generation, before, after) in self.values.values() {
            if exact_candidate == Some(connection.as_str()) {
                continue;
            }
            if before == after {
                continue;
            }
            // No historical metadata can revive a retired incarnation or
            // overwrite a later independently recorded native observation.
            let changed=c.execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.stream_revision',?4) WHERE id=?1 AND generation=?2 AND observed_runtime_id=?3 AND state IN ('ready','active') AND json_extract(readiness_json,'$.stream_revision')=?5 AND (EXISTS(SELECT 1 FROM native_connection_lifetimes l WHERE l.connection_id=connections.id AND l.generation=connections.generation AND l.runtime_epoch=?3 AND l.retired=0) OR EXISTS(SELECT 1 FROM telephone_connection_lifetimes l WHERE l.connection_id=connections.id AND l.generation=connections.generation AND l.runtime_epoch=?3 AND l.retired=0))",params![connection,generation,self.runtime,after,before]).map_err(|_|Error::storage())?;
            if exact_candidate.is_some() && changed != 1 {
                return Err(conflict());
            }
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
