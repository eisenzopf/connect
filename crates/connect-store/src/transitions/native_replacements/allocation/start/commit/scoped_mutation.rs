//! Exact immutable SQL plans built only after the original phase checks.
use super::*;
use crate::authority_writer_boundary::phase_ledger::scoped::{
    self, Plan, Purpose, Row, Table, Transition, Witness,
};
use crate::authority_writer_boundary::phase_ledger::{Key, Stage};
use rusqlite::types::Value;

fn text(value: impl Into<String>) -> Value {
    Value::Text(value.into())
}
fn number(value: u64) -> Result<Value> {
    i64::try_from(value)
        .map(Value::Integer)
        .map_err(|_| Error::forbidden())
}
fn row(c: &Connection, table: Table, id: &str) -> Result<Row> {
    scoped::row_snapshot(c, table, &[text(id)])?.ok_or_else(Error::forbidden)
}
fn integer(row: &Row, column: &str) -> Result<i64> {
    match row.get(column)? {
        Value::Integer(value) => Ok(*value),
        _ => Err(Error::forbidden()),
    }
}
fn append(c: &Connection, all: &mut Vec<Transition>, old: Option<&Row>, new: &Row) -> Result<()> {
    all.push(Transition::new(c, old, Some(new))?);
    Ok(())
}
pub(super) fn begin_plan<'a>(
    store: &Store,
    c: &Connection,
    origin: &'a NativeReplacementMediaOrigin,
    read: &'a NativeReplacementSceneRead,
    attempt: &str,
    effect: &str,
    expected: &str,
    at: i64,
) -> Result<Plan<'a>> {
    let p = &origin.started.attempt.allocated.attempt.prepared;
    let context = read.media.source.context();
    let mut all = Vec::new();
    let effect_prepared = Row::new(
        Table::Effects,
        vec![
            text(effect),
            text(&p.record),
            text("native-replacement-commit"),
            text(effect),
            text("none"),
            text("prepared"),
            Value::Integer(0),
            Value::Null,
            text(expected),
            Value::Integer(0),
            Value::Null,
        ],
    )?;
    append(c, &mut all, None, &effect_prepared)?;
    let step = Row::new(
        Table::ReplacementSteps,
        vec![
            text(&p.record),
            text("commit"),
            text(attempt),
            text(effect),
            Value::Integer(0),
            Value::Null,
        ],
    )?;
    append(c, &mut all, None, &step)?;
    let mut started = step.clone();
    started.set("started", Value::Integer(1))?;
    append(c, &mut all, Some(&step), &started)?;
    let mut submitting = effect_prepared.clone();
    submitting.set("state", text("submitting"))?;
    submitting.set("attempt", Value::Integer(1))?;
    submitting.set("submission_generation", Value::Integer(1))?;
    append(c, &mut all, Some(&effect_prepared), &submitting)?;
    let old = row(c, Table::Operations, &p.record)?;
    let mut touched = old.clone();
    touched.set(
        "revision",
        Value::Integer(
            integer(&old, "revision")?
                .checked_add(1)
                .ok_or_else(Error::forbidden)?,
        ),
    )?;
    touched.set("updated_at", Value::Integer(at))?;
    append(c, &mut all, Some(&old), &touched)?;
    let until:i64=c.query_row("SELECT min(o.expires_at,n.expires_at) FROM operations o JOIN native_replacements n ON n.operation_record_id=o.record_id WHERE o.record_id=?1",[&p.record],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !read.current() || p.runtime != store.runtime_epoch() {
        return Err(Error::forbidden());
    }
    Ok(Plan {
        key: Key {
            record: p.record.clone(),
            phase: "scoped-commit-begin",
            attempt: attempt.into(),
            effect: effect.into(),
            expected: expected.into(),
            runtime: p.runtime.clone(),
            reservation: p.reservation.clone(),
            digest: p.hash.clone(),
            stage: Stage::Begin,
        },
        witness: Witness::Begin { origin, read },
        purpose: Purpose::CommitBegin,
        source: context.source_connection_id.clone(),
        candidate: Some(read.media.target.connection.connection_id.clone()),
        required: read.scoped_commit_scopes()?,
        until: Some(until.min(read.media.source.expires_at())),
        transitions: all,
    })
}

/// Only current known public Ready with a fresh absent candidate enters this
/// plan. Claimed/private/stopped/successor/Unknown cases keep the original broad
/// metadata Writer; actual Core work is never repeated by either path.
pub(super) fn ready_plan<'a>(
    c: &Connection,
    p: &Projection,
    captured: &'a NativeReplacementCommitted,
    required: Vec<AuthorityScope>,
    until: i64,
    descriptor: &serde_json::Value,
    intent: &connect_protocol::connection_control::OfferIntent,
    readiness: &str,
    result: &serde_json::Value,
    topology: u64,
    at: i64,
) -> Result<Plan<'a>> {
    let root = row(c, Table::Operations, &p.record)?;
    if root.get("state")? != &text("preparing") {
        return Err(Error::forbidden());
    }
    let source = match root.get("connection_id")? {
        Value::Text(value) => value.clone(),
        _ => return Err(Error::forbidden()),
    };
    let target = &p.facts.target;
    let candidate = &target.connection.connection_id;
    if scoped::row_snapshot(c, Table::Connections, &[text(candidate)])?.is_some()
        || scoped::row_snapshot(c, Table::Offers, &[text(candidate)])?.is_some()
    {
        return Err(Error::forbidden());
    }
    let source_before = row(c, Table::Connections, &source)?;
    let tenant = source_before.get("tenant_id")?.clone();
    let mut all = Vec::new();
    let candidate_row = Row::new(
        Table::Connections,
        vec![
            text(candidate),
            text(&target.connection.session_id),
            text(&target.connection.participant_id),
            number(target.connection.membership_epoch)?,
            text(&target.identity_id),
            tenant,
            text(&target.connection.endpoint_id),
            number(target.connection.generation)?,
            text(&source),
            text("ready"),
            Value::Integer(1),
            text(&p.runtime),
            Value::Integer(at),
            text(readiness),
        ],
    )?;
    append(c, &mut all, None, &candidate_row)?;
    let mut offer = intent.offer.clone();
    offer
        .as_object_mut()
        .ok_or_else(Error::forbidden)?
        .remove("replacement_preparation");
    let offer_row = Row::new(
        Table::Offers,
        vec![
            text(candidate),
            text(&target.runtime_connection_id),
            text(&intent.operation_id),
            text(
                descriptor["credential_id"]
                    .as_str()
                    .ok_or_else(Error::forbidden)?,
            ),
            text(offer.to_string()),
            Value::Integer(at),
        ],
    )?;
    append(c, &mut all, None, &offer_row)?;
    let source_record: String = c
        .query_row(
            "SELECT operation_record_id FROM native_connection_lifetimes WHERE connection_id=?1",
            [&source],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let lifetime = row(c, Table::NativeConnectionLifetimes, &source_record)?;
    if integer(&lifetime, "retired")? == 0 {
        let mut retired = lifetime.clone();
        retired.set("retired", Value::Integer(1))?;
        append(c, &mut all, Some(&lifetime), &retired)?;
    }
    if source_before.get("state")? != &text("retired") {
        let mut retired = source_before.clone();
        retired.set("state", text("retired"))?;
        retired.set(
            "generation",
            Value::Integer(
                integer(&source_before, "generation")?
                    .checked_add(1)
                    .ok_or_else(Error::forbidden)?,
            ),
        )?;
        retired.set(
            "revision",
            Value::Integer(
                integer(&source_before, "revision")?
                    .checked_add(1)
                    .ok_or_else(Error::forbidden)?,
            ),
        )?;
        for column in ["observed_runtime_id", "observed_at", "readiness_json"] {
            retired.set(column, Value::Null)?;
        }
        append(c, &mut all, Some(&source_before), &retired)?;
    }
    let session = row(c, Table::Sessions, &target.connection.session_id)?;
    let mut revised = session.clone();
    revised.set("topology_revision", number(topology)?)?;
    revised.set("updated_at", Value::Integer(at))?;
    append(c, &mut all, Some(&session), &revised)?;
    let mut committing = root.clone();
    committing.set("state", text("committing"))?;
    append(c, &mut all, Some(&root), &committing)?;
    let mut completed = committing.clone();
    completed.set("state", text("completed"))?;
    completed.set("result_json", text(result.to_string()))?;
    append(c, &mut all, Some(&committing), &completed)?;
    let replacement_lifetime = Row::new(
        Table::NativeReplacementLifetimes,
        vec![
            text(&p.record),
            text(
                descriptor["credential_id"]
                    .as_str()
                    .ok_or_else(Error::forbidden)?,
            ),
            text(descriptor["key_id"].as_str().ok_or_else(Error::forbidden)?),
            text(&target.device_id),
            text(&target.connection.endpoint_id),
            text(&target.connection.participant_id),
            number(target.connection.membership_epoch)?,
            number(target.connection.generation)?,
            text(&p.facts.resources),
            Value::Integer(p.facts.source_expires_at),
            text(&p.runtime),
            text(&p.attempt),
            text(&p.effect),
        ],
    )?;
    append(c, &mut all, None, &replacement_lifetime)?;
    let native_lifetime = Row::new(
        Table::NativeConnectionLifetimes,
        vec![
            text(&p.record),
            text(candidate),
            text(&p.runtime),
            number(target.connection.generation)?,
            Value::Integer(0),
        ],
    )?;
    append(c, &mut all, None, &native_lifetime)?;
    let mut invalidated = completed.clone();
    for column in ["lease_owner", "lease_expires_at", "worker_runtime"] {
        invalidated.set(column, Value::Null)?;
    }
    invalidated.set(
        "worker_generation",
        Value::Integer(
            integer(&completed, "worker_generation")?
                .saturating_add(1)
                .min(9_007_199_254_740_991),
        ),
    )?;
    append(c, &mut all, Some(&completed), &invalidated)?;
    let mut touched = invalidated.clone();
    touched.set(
        "revision",
        Value::Integer(
            integer(&invalidated, "revision")?
                .checked_add(1)
                .ok_or_else(Error::forbidden)?,
        ),
    )?;
    touched.set("updated_at", Value::Integer(at))?;
    append(c, &mut all, Some(&invalidated), &touched)?;
    Ok(Plan {
        key: Key {
            record: p.record.clone(),
            phase: "scoped-final-ready",
            attempt: p.attempt.clone(),
            effect: p.effect.clone(),
            expected: p.expected.clone(),
            runtime: p.runtime.clone(),
            reservation: p.reservation.clone(),
            digest: p.hash.clone(),
            stage: Stage::Confirm,
        },
        witness: Witness::Ready(captured),
        purpose: Purpose::FinalReady,
        source,
        candidate: Some(candidate.clone()),
        required,
        until: Some(until),
        transitions: all,
    })
}

pub(super) fn publication_result(
    p: &Projection,
    source: &str,
    retained: &str,
    topology: u64,
    stopped: bool,
) -> Result<serde_json::Value> {
    let target = &p.facts.target;
    let ready: serde_json::Value =
        serde_json::from_str(&p.observation).map_err(|_| Error::storage())?;
    let retired = if stopped {
        vec![source.to_owned(), target.connection.connection_id.clone()]
    } else {
        vec![source.to_owned()]
    };
    Ok(
        serde_json::json!({"activation_state":if stopped{"stopped"}else{"ready"},"retained_connection_ids":[retained],"admitted_connection_ids":[target.connection.connection_id],"retired_connection_ids":retired,"conversation_revision":null,"connection_id":target.connection.connection_id,"replaces":source,"generation":target.connection.generation,"participant_id":target.connection.participant_id,"membership_epoch":target.connection.membership_epoch,"endpoint_id":target.connection.endpoint_id,"device_id":target.device_id,"readiness":if stopped {serde_json::Value::Null}else{ready["readiness"].clone()},"physical_readiness":ready["readiness"],"native_activation":ready["native_activation"],"session_revision":topology}),
    )
}
