//! REVIEW DRAFT ONLY. Intended private child of phase_ledger::scoped.
//! Called after plan_facts and complete same-Unit current-authorization checks.
//! It supplies no policy, media permission, lifecycle no-op witness or runtime
//! absence observation. Those remain mandatory in the four fixed entry points.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

fn refuse<T>() -> Result<T> {
    Err(Error::forbidden())
}
fn jtext<'a>(v: &'a serde_json::Value, key: &str) -> Result<&'a str> {
    v.get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(Error::forbidden)
}
fn jint(v: &serde_json::Value, key: &str) -> Result<i64> {
    v.get(key)
        .and_then(serde_json::Value::as_u64)
        .and_then(|n| i64::try_from(n).ok())
        .ok_or_else(Error::forbidden)
}
fn null(row: &Row, column: &str) -> Result<()> {
    if row.get(column)? == &Value::Null {
        Ok(())
    } else {
        refuse()
    }
}
fn equal(row: &Row, column: &str, value: &Value) -> Result<()> {
    if row.get(column)? == value {
        Ok(())
    } else {
        refuse()
    }
}
fn same_row(c: &Connection, a: &Row, b: &Row) -> Result<()> {
    if a.table == b.table && row_tuple(c, a)? == row_tuple(c, b)? {
        Ok(())
    } else {
        refuse()
    }
}
fn exists(c: &Connection, table: Table, key: &str) -> Result<Row> {
    row_snapshot(c, table, &[Value::Text(key.into())])?.ok_or_else(Error::forbidden)
}

/// Generic footprints.json validation does not guarantee this independent
/// typed tuple's column list was updated alongside a future schema migration.
/// Check xinfo, including hidden/generated columns, against the closed tuple.
fn complete_columns(c: &Connection, table: Table) -> Result<()> {
    let actual = c
        .prepare(&format!("PRAGMA table_xinfo(\"{}\")", table.name()))
        .map_err(|_| Error::storage())?
        .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(6)?)))
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    let wanted = table
        .columns()
        .iter()
        .map(|s| s.to_string())
        .collect::<BTreeSet<_>>();
    if actual.len() != wanted.len()
        || actual.iter().any(|(_, hidden)| *hidden != 0)
        || actual
            .into_iter()
            .map(|(name, _)| name)
            .collect::<BTreeSet<_>>()
            != wanted
    {
        return refuse();
    }
    Ok(())
}

/// This witness is PRIVATE and assembled only from already-confirmed immutable
/// phase rows under the original SQL Unit. No Plan-supplied JSON is authority.
struct FinalWitness {
    root: Row,
    source: Row,
    descriptor: serde_json::Value,
    capture: serde_json::Value,
    target: connect_protocol::connection_control::PreparedConnection,
    identity: String,
    runtime: String,
    device: String,
    resources: String,
    source_until: i64,
    retained: String,
}
impl FinalWitness {
    fn load(c: &Connection, p: &Plan<'_>, f: &Facts) -> Result<Self> {
        let candidate = p.candidate.as_deref().ok_or_else(Error::forbidden)?;
        // Deliberately no retired-slot reuse/private/stopped/successor branch.
        if row_snapshot(c, Table::Connections, &[Value::Text(candidate.into())])?.is_some()
            || row_snapshot(c, Table::Offers, &[Value::Text(candidate.into())])?.is_some()
        {
            return refuse();
        }
        let claimed:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_candidates WHERE operation_record_id=?1 OR candidate_connection_id=?2)",rusqlite::params![p.key.record,candidate],|r|r.get(0)).map_err(|_|Error::storage())?;
        if claimed {
            return refuse();
        }
        let raw:Option<(String,String)>=c.query_row(
            "SELECT s.observation_json,se.observation_json FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=s.operation_record_id JOIN native_replacement_steps start ON start.operation_record_id=s.operation_record_id AND start.phase='start' JOIN operation_effects se ON se.id=start.effect_id AND se.operation_record_id=start.operation_record_id WHERE s.operation_record_id=?1 AND s.phase='commit' AND s.attempt_id=?2 AND s.effect_id=?3 AND s.started=1 AND s.observation_json IS NOT NULL AND e.provider='native-replacement-commit' AND e.correlation_key=e.id AND e.idempotency_mode='none' AND e.state='confirmed' AND e.attempt=1 AND e.submission_generation=1 AND e.observation_json=?4 AND e.provider_ref IS NULL AND e.compensates_effect_id IS NULL AND start.started=1 AND start.observation_json IS NOT NULL AND se.provider='native-replacement-start' AND se.correlation_key=se.id AND se.idempotency_mode='none' AND se.state='confirmed' AND se.attempt=1 AND se.submission_generation=1 AND se.provider_ref IS NULL AND se.compensates_effect_id IS NULL",
            rusqlite::params![p.key.record,p.key.attempt,p.key.effect,p.key.expected],|r|Ok((r.get(0)?,r.get(1)?)))
            .optional().map_err(|_|Error::storage())?;
        let (capture, descriptor) = raw.ok_or_else(Error::forbidden)?;
        let capture: serde_json::Value =
            serde_json::from_str(&capture).map_err(|_| Error::forbidden())?;
        let descriptor: serde_json::Value =
            serde_json::from_str(&descriptor).map_err(|_| Error::forbidden())?;
        let expected: serde_json::Value =
            serde_json::from_str(&p.key.expected).map_err(|_| Error::forbidden())?;
        if capture["movement"] != expected || !capture["projection"]["private_successor"].is_null()
        {
            return refuse();
        }
        let projection = &capture["projection"];
        let target: connect_protocol::connection_control::PreparedConnection =
            serde_json::from_value(projection["target"]["connection"].clone())
                .map_err(|_| Error::forbidden())?;
        let planned_generation: i64 = c
            .query_row(
                "SELECT generation FROM native_replacements WHERE operation_record_id=?1",
                [&p.key.record],
                |r| r.get(0),
            )
            .map_err(|_| Error::forbidden())?;
        if i64::try_from(target.generation).ok() != Some(planned_generation)
            || source_generation(c, p)?.checked_add(1) != Some(planned_generation)
        {
            return refuse();
        }
        let source = exists(c, Table::Connections, &p.source)?;
        let root = exists(c, Table::Operations, &p.key.record)?;
        let identity = jtext(&projection["target"], "identity_id")?.to_owned();
        let runtime = jtext(&projection["target"], "runtime_connection_id")?.to_owned();
        let device = jtext(&projection["target"], "device_id")?.to_owned();
        let mut request: serde_json::Value =
            serde_json::from_str(text(&root, "intent_json")?).map_err(|_| Error::forbidden())?;
        request
            .as_object_mut()
            .ok_or_else(Error::forbidden)?
            .insert(
                "operation_id".into(),
                serde_json::json!(text(&root, "operation_id")?),
            );
        let request: connect_protocol::transitions::TransitionRequest =
            serde_json::from_value(request).map_err(|_| Error::forbidden())?;
        let retained = request.intent.retained().to_owned();
        let intent: connect_protocol::connection_control::OfferIntent =
            serde_json::from_value(descriptor["intent"].clone()).map_err(|_| Error::forbidden())?;
        if target.connection_id != candidate
            || target.conversation_id != f.conversation
            || target.session_id != f.session
            || target.participant_id != text(&source, "participant_id")?
            || i64::try_from(target.membership_epoch).ok()
                != Some(integer(&source, "membership_epoch")?)
            || identity != text(&source, "identity_id")?
            || target.endpoint_id != jtext(&descriptor, "endpoint_id")?
            || device != jtext(&descriptor, "device_id")?
            || intent.connection_id != candidate
            || intent.conversation_id != f.conversation
            || intent.session_id != f.session
        {
            return refuse();
        }
        // Some Store-only controlled captures predate no wire boundary because
        // they have no native writer. Preserve exactly NULL in that case; never
        // invent a cut. If an actual boundary is stored it must remain exact.
        if !capture["native_activation"].is_null() {
            let boundary: connect_protocol::replacement_activation::NativeActivationBoundary =
                serde_json::from_value(capture["native_activation"].clone())
                    .map_err(|_| Error::forbidden())?;
            boundary.validate()?;
            if boundary.operation_record_id != p.key.record
                || boundary.operation_id != text(&root, "operation_id")?
                || boundary.source_connection_id != p.source
                || boundary.target_connection_id != candidate
                || boundary.target_generation != target.generation
                || i64::try_from(boundary.stream_revision).ok()
                    != Some(jint(&capture["readiness"], "stream_revision")?)
            {
                return refuse();
            }
            use connect_protocol::replacement_activation::NativeActivationAudience as A;
            match (&boundary.audience, intent.offer["substrate"].as_str()) {
                (A::Public { .. }, Some("quic" | "webtransport")) => {}
                (A::Unsupported { reason }, Some("websocket+webrtc"))
                    if reason == "non-native-adapter" => {}
                _ => return refuse(),
            }
        }
        let resources = jtext(projection, "resources")?.to_owned();
        let source_until = jint(projection, "source_expires_at")?;
        Ok(Self {
            root,
            source,
            descriptor,
            capture,
            target,
            identity,
            runtime,
            device,
            resources,
            source_until,
            retained,
        })
    }
    fn validate_row(
        &self,
        c: &Connection,
        p: &Plan<'_>,
        t: &Transition,
        topology: i64,
        at: i64,
    ) -> Result<()> {
        let r = &t.new;
        match t.table {
            Table::Connections if text(r, "id")? == self.target.connection_id => {
                if t.old.is_some() {
                    return refuse();
                }
                let expected = Row::new(
                    Table::Connections,
                    vec![
                        Value::Text(self.target.connection_id.clone()),
                        Value::Text(self.target.session_id.clone()),
                        Value::Text(self.target.participant_id.clone()),
                        Value::Integer(
                            i64::try_from(self.target.membership_epoch)
                                .map_err(|_| Error::forbidden())?,
                        ),
                        Value::Text(self.identity.clone()),
                        self.source.get("tenant_id")?.clone(),
                        Value::Text(self.target.endpoint_id.clone()),
                        Value::Integer(
                            i64::try_from(self.target.generation)
                                .map_err(|_| Error::forbidden())?,
                        ),
                        Value::Text(p.source.clone()),
                        Value::Text("ready".into()),
                        Value::Integer(1),
                        Value::Text(p.key.runtime.clone()),
                        Value::Integer(at),
                        Value::Text(self.capture["readiness"].to_string()),
                    ],
                )?;
                same_row(c, r, &expected)?;
            }
            Table::Offers => {
                if t.old.is_some() {
                    return refuse();
                }
                let mut offer = self.descriptor["intent"]["offer"].clone();
                offer
                    .as_object_mut()
                    .ok_or_else(Error::forbidden)?
                    .remove("replacement_preparation");
                let expected = Row::new(
                    Table::Offers,
                    vec![
                        Value::Text(self.target.connection_id.clone()),
                        Value::Text(self.runtime.clone()),
                        Value::Text(jtext(&self.descriptor["intent"], "operation_id")?.into()),
                        Value::Text(jtext(&self.descriptor, "credential_id")?.into()),
                        Value::Text(offer.to_string()),
                        Value::Integer(at),
                    ],
                )?;
                same_row(c, r, &expected)?;
            }
            Table::NativeReplacementLifetimes => {
                if t.old.is_some() {
                    return refuse();
                }
                let expected = Row::new(
                    Table::NativeReplacementLifetimes,
                    vec![
                        Value::Text(p.key.record.clone()),
                        Value::Text(jtext(&self.descriptor, "credential_id")?.into()),
                        Value::Text(jtext(&self.descriptor, "key_id")?.into()),
                        Value::Text(self.device.clone()),
                        Value::Text(self.target.endpoint_id.clone()),
                        Value::Text(self.target.participant_id.clone()),
                        Value::Integer(
                            i64::try_from(self.target.membership_epoch)
                                .map_err(|_| Error::forbidden())?,
                        ),
                        Value::Integer(
                            i64::try_from(self.target.generation)
                                .map_err(|_| Error::forbidden())?,
                        ),
                        Value::Text(self.resources.clone()),
                        Value::Integer(self.source_until),
                        Value::Text(p.key.runtime.clone()),
                        Value::Text(p.key.attempt.clone()),
                        Value::Text(p.key.effect.clone()),
                    ],
                )?;
                same_row(c, r, &expected)?;
            }
            Table::NativeConnectionLifetimes if t.old.is_none() => {
                let expected = Row::new(
                    Table::NativeConnectionLifetimes,
                    vec![
                        Value::Text(p.key.record.clone()),
                        Value::Text(self.target.connection_id.clone()),
                        Value::Text(p.key.runtime.clone()),
                        Value::Integer(
                            i64::try_from(self.target.generation)
                                .map_err(|_| Error::forbidden())?,
                        ),
                        Value::Integer(0),
                    ],
                )?;
                same_row(c, r, &expected)?;
            }
            Table::Operations
                if text(r, "state")? == "completed"
                    && t.old
                        .as_ref()
                        .is_some_and(|old| text(old, "state").is_ok_and(|s| s == "committing")) =>
            {
                let expected = serde_json::json!({"activation_state":"ready","retained_connection_ids":[self.retained],"admitted_connection_ids":[self.target.connection_id],"retired_connection_ids":[p.source],"conversation_revision":null,"connection_id":self.target.connection_id,"replaces":p.source,"generation":self.target.generation,"participant_id":self.target.participant_id,"membership_epoch":self.target.membership_epoch,"endpoint_id":self.target.endpoint_id,"device_id":self.device,"readiness":self.capture["readiness"],"physical_readiness":self.capture["readiness"],"native_activation":self.capture["native_activation"],"session_revision":topology});
                equal(r, "result_json", &Value::Text(expected.to_string()))?;
            }
            _ => {}
        }
        Ok(())
    }
}

fn source_generation(c: &Connection, p: &Plan<'_>) -> Result<i64> {
    let sql = if p.purpose.cleanup() {
        "SELECT generation FROM native_admission_sources WHERE operation_record_id=?1"
    } else {
        "SELECT json_extract(plan_json,'$.source_generation') FROM native_replacements WHERE operation_record_id=?1"
    };
    c.query_row(sql, [&p.key.record], |r| r.get(0))
        .map_err(|_| Error::forbidden())
}
fn expected_worker_invalidation(old: &Row) -> Result<Row> {
    let mut n = old.clone();
    for k in ["lease_owner", "lease_expires_at", "worker_runtime"] {
        n.set(k, Value::Null)?;
    }
    n.set(
        "worker_generation",
        Value::Integer(
            integer(old, "worker_generation")?
                .saturating_add(1)
                .min(9_007_199_254_740_991),
        ),
    )?;
    Ok(n)
}
fn expected_touch(old: &Row, at: i64) -> Result<Row> {
    let mut n = old.clone();
    let revision = integer(old, "revision")?
        .checked_add(1)
        .filter(|v| *v < 9_007_199_254_740_991)
        .ok_or_else(Error::forbidden)?;
    n.set("revision", Value::Integer(revision))?;
    n.set("updated_at", Value::Integer(at))?;
    Ok(n)
}
fn root_sequence(c: &Connection, p: &Plan<'_>, at: i64) -> Result<()> {
    let rows = p
        .transitions
        .iter()
        .filter(|t| t.table == Table::Operations)
        .collect::<Vec<_>>();
    let expected_count = match p.purpose {
        Purpose::CommitBegin | Purpose::OriginalRetire => 1,
        Purpose::ReplacementRetire => 0,
        Purpose::FinalReady => 4,
    };
    if rows.len() != expected_count {
        return refuse();
    }
    for (i, t) in rows.into_iter().enumerate() {
        let old = t.old.as_ref().ok_or_else(Error::forbidden)?;
        if text(&t.new, "record_id")? != p.key.record {
            return refuse();
        }
        let expected = match p.purpose {
            Purpose::CommitBegin => expected_touch(old, at)?,
            Purpose::OriginalRetire => expected_worker_invalidation(old)?,
            Purpose::ReplacementRetire => return refuse(),
            Purpose::FinalReady => match i {
                0 => {
                    if text(old, "state")? != "preparing" {
                        return refuse();
                    }
                    let mut n = old.clone();
                    n.set("state", Value::Text("committing".into()))?;
                    n
                }
                1 => {
                    if text(old, "state")? != "committing" {
                        return refuse();
                    }
                    let mut n = old.clone();
                    n.set("state", Value::Text("completed".into()))?;
                    n.set("result_json", t.new.get("result_json")?.clone())?;
                    n
                } // exact result checked separately from persisted witness
                2 => {
                    if text(old, "state")? != "completed" {
                        return refuse();
                    }
                    expected_worker_invalidation(old)?
                }
                3 => expected_touch(old, at)?,
                _ => return refuse(),
            },
        };
        same_row(c, &t.new, &expected)?;
    }
    Ok(())
}
fn exact_retirement(c: &Connection, p: &Plan<'_>, t: &Transition) -> Result<()> {
    let old = t.old.as_ref().ok_or_else(Error::forbidden)?;
    if text(old, "id")? != p.source || !["ready", "active", "held"].contains(&text(old, "state")?) {
        return refuse();
    }
    let generation = source_generation(c, p)?;
    if integer(old, "generation")? != generation {
        return refuse();
    }
    let mut n = old.clone();
    n.set("state", Value::Text("retired".into()))?;
    n.set(
        "generation",
        Value::Integer(generation.checked_add(1).ok_or_else(Error::forbidden)?),
    )?;
    n.set(
        "revision",
        Value::Integer(
            integer(old, "revision")?
                .checked_add(1)
                .ok_or_else(Error::forbidden)?,
        ),
    )?;
    for key in ["observed_runtime_id", "observed_at", "readiness_json"] {
        n.set(key, Value::Null)?;
    }
    same_row(c, &t.new, &n)
}

/// Additional closed-plan validation. Root's native_scoped_mutation_ledger must
/// still compare initial OLD against CURRENT actual snapshots and every adjacent
/// repeated-PK transition against its predecessor, then pin full actual scopes.
///
/// Required callsite: strict_validation::validate(c,&plan,&facts)? before permit
/// installation. Root additionally derives the CommitBegin expected movement
/// from the checked opaque media read / immutable saved plan (not key.expected).
pub(super) fn validate(c: &Connection, p: &Plan<'_>, f: &Facts) -> Result<()> {
    for table in [
        Table::Operations,
        Table::Effects,
        Table::ReplacementSteps,
        Table::Connections,
        Table::Offers,
        Table::Sessions,
        Table::NativeConnectionLifetimes,
        Table::NativeReplacementLifetimes,
    ] {
        complete_columns(c, table)?;
    }
    let root = exists(c, Table::Operations, &p.key.record)?;
    let source = exists(c, Table::Connections, &p.source)?;
    let at = if p.purpose.cleanup() {
        crate::now()
    } else {
        let touches = p
            .transitions
            .iter()
            .filter(|t| {
                t.table == Table::Operations
                    && t.old
                        .as_ref()
                        .is_some_and(|o| o.get("revision").ok() != t.new.get("revision").ok())
            })
            .collect::<Vec<_>>();
        if touches.len() != 1 {
            return refuse();
        }
        let at = integer(&touches[0].new, "updated_at")?;
        // No fresh duration or authority. Same recorded timestamp across exact
        // public rows; cannot be before the saved root observation or future.
        if at < integer(&root, "updated_at")? || at > crate::now() {
            return refuse();
        }
        at
    };
    let final_witness = if p.purpose == Purpose::FinalReady {
        Some(FinalWitness::load(c, p, f)?)
    } else {
        None
    };
    let topology = if p.purpose == Purpose::FinalReady {
        integer(
            &exists(c, Table::Sessions, &f.session)?,
            "topology_revision",
        )?
        .checked_add(1)
        .filter(|v| *v < 9_007_199_254_740_991)
        .ok_or_else(Error::forbidden)?
    } else {
        0
    };
    root_sequence(c, p, at)?;
    let life_keys=c.prepare("SELECT operation_record_id FROM native_connection_lifetimes WHERE connection_id=?1 AND retired=0 ORDER BY operation_record_id").map_err(|_|Error::storage())?
        .query_map([&p.source],|r|r.get::<_,String>(0)).map_err(|_|Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>().map_err(|_|Error::storage())?;
    let source_update = usize::from(text(&source, "state")? != "retired");
    let mut counts = BTreeMap::<Table, usize>::new();
    let mut seen_lives = BTreeSet::new();
    for t in &p.transitions {
        *counts.entry(t.table).or_default() += 1;
        match t.table {
            Table::Effects => {
                if p.purpose != Purpose::CommitBegin {
                    return refuse();
                }
                let n = &t.new;
                for k in ["provider_ref", "compensates_effect_id"] {
                    null(n, k)?;
                }
                if let Some(old) = &t.old {
                    if text(old, "state")? != "prepared"
                        || integer(old, "attempt")? != 0
                        || integer(old, "submission_generation")? != 0
                    {
                        return refuse();
                    }
                    let mut expected = old.clone();
                    expected.set("state", Value::Text("submitting".into()))?;
                    expected.set("attempt", Value::Integer(1))?;
                    expected.set("submission_generation", Value::Integer(1))?;
                    same_row(c, n, &expected)?;
                } else if text(n, "state")? != "prepared"
                    || integer(n, "attempt")? != 0
                    || integer(n, "submission_generation")? != 0
                {
                    return refuse();
                }
            }
            Table::ReplacementSteps => {
                if p.purpose != Purpose::CommitBegin {
                    return refuse();
                }
                if let Some(old) = &t.old {
                    if integer(old, "started")? != 0 {
                        return refuse();
                    }
                    let mut expected = old.clone();
                    expected.set("started", Value::Integer(1))?;
                    same_row(c, &t.new, &expected)?;
                } else if integer(&t.new, "started")? != 0 {
                    return refuse();
                }
            }
            Table::Connections => {
                if text(&t.new, "id")? == p.source {
                    exact_retirement(c, p, t)?;
                } else if final_witness.is_none() {
                    return refuse();
                }
            }
            Table::Sessions => {
                let old = t.old.as_ref().ok_or_else(Error::forbidden)?;
                let mut expected = old.clone();
                expected.set("topology_revision", Value::Integer(topology))?;
                expected.set("updated_at", Value::Integer(at))?;
                same_row(c, &t.new, &expected)?;
            }
            Table::NativeConnectionLifetimes if t.old.is_some() => {
                let old = t.old.as_ref().ok_or_else(Error::forbidden)?;
                let record = text(old, "operation_record_id")?;
                if text(old, "connection_id")? != p.source
                    || text(old, "runtime_epoch")? != p.key.runtime
                    || integer(old, "generation")? != source_generation(c, p)?
                    || integer(old, "retired")? != 0
                    || !life_keys.iter().any(|k| k == record)
                    || !seen_lives.insert(record.to_owned())
                {
                    return refuse();
                }
                let mut expected = old.clone();
                expected.set("retired", Value::Integer(1))?;
                same_row(c, &t.new, &expected)?;
            }
            _ => {}
        }
        if let Some(witness) = &final_witness {
            witness.validate_row(c, p, t, topology, at)?;
        }
    }
    let count = |table| counts.get(&table).copied().unwrap_or(0);
    let good = match p.purpose {
        Purpose::CommitBegin => {
            count(Table::Operations) == 1
                && count(Table::Effects) == 2
                && count(Table::ReplacementSteps) == 2
                && p.transitions.len() == 5
        }
        Purpose::FinalReady => {
            count(Table::Operations) == 4
                && count(Table::Connections) == 1 + source_update
                && count(Table::Offers) == 1
                && count(Table::Sessions) == 1
                && count(Table::NativeReplacementLifetimes) == 1
                && count(Table::NativeConnectionLifetimes) == 1 + life_keys.len()
                && p.transitions.len() == 9 + source_update + life_keys.len()
                && seen_lives.len() == life_keys.len()
        }
        Purpose::OriginalRetire => {
            count(Table::Operations) == 1
                && count(Table::Connections) == source_update
                && count(Table::NativeConnectionLifetimes) == life_keys.len()
                && p.transitions.len() == 1 + source_update + life_keys.len()
                && seen_lives.len() == life_keys.len()
        }
        Purpose::ReplacementRetire => {
            count(Table::Connections) == source_update
                && count(Table::NativeConnectionLifetimes) == life_keys.len()
                && p.transitions.len() == source_update + life_keys.len()
                && seen_lives.len() == life_keys.len()
        }
    };
    if good {
        Ok(())
    } else {
        refuse()
    }
}
