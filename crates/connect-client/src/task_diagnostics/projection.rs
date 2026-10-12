//! Closed projections keep user content and remote error text out of diagnostics.
use super::models::*;
use crate::{ClientError, Result};
use connect_protocol::tasks::*;

pub(super) fn section<T>(
    result: Result<TaskOutcome>,
    project: impl FnOnce(TaskOutcome) -> Result<T>,
) -> Result<DiagnosticSection<T>> {
    match result {
        Ok(value) => Ok(DiagnosticSection::Available(project(value)?)),
        Err(error) => unavailable(error),
    }
}
pub(super) fn unavailable<T>(error: ClientError) -> Result<DiagnosticSection<T>> {
    match error {
        ClientError::Application(error)
            if matches!(
                error.code.as_str(),
                "forbidden" | "unsupported-operation" | "unsupported-profile"
            ) =>
        {
            Ok(DiagnosticSection::Unavailable { code: error.code })
        }
        other => Err(other),
    }
}
pub(super) fn record(outcome: TaskOutcome) -> Result<TaskRecord> {
    match outcome {
        TaskOutcome::Record(record) => Ok(*record),
        _ => Err(ClientError::Protocol),
    }
}
pub(super) fn task(record: TaskRecord) -> DiagnosticTask {
    DiagnosticTask {
        task_id: record.id,
        conversation_id: record.conversation_id,
        state: record.state,
        revision: record.revision,
        expires_at: record.expires_at,
        updated_at: record.updated_at,
    }
}
pub(super) fn event(event: TaskEvent) -> DiagnosticEvent {
    let observation = match event.data {
        TaskEventData::State {
            state, revision, ..
        } => DiagnosticObservation::State { state, revision },
        TaskEventData::Effect(effect) => DiagnosticObservation::Effect {
            effect_id: effect.id,
            step_id: effect.step_id,
            request_id: effect.request_id,
            state: effect.state,
        },
        TaskEventData::Reply(reply) => DiagnosticObservation::Reply(reply),
        TaskEventData::Claim(claim) => DiagnosticObservation::Claim(claim),
    };
    DiagnosticEvent {
        event_id: event.id,
        sequence: event.sequence,
        observed_at: event.created_at,
        observation,
    }
}
pub(super) fn source(source: TaskSource) -> DiagnosticSource {
    DiagnosticSource {
        source_id: source.source_id,
        kind: source.kind,
        actor_id: source.actor_id,
        created_at: source.created_at,
        provenance: source.provenance,
        state: source.state,
    }
}
