//! A tool observation can progress while its original effect identity stays fixed.
use anyhow::{ensure, Result};
use connect_client::{tasks::*, ConnectClient, Operation, Outcome};

pub(super) async fn verify(
    client: &mut ConnectClient,
    operation: &Operation,
    prior: &Outcome,
    next: &Outcome,
) -> Result<()> {
    if prior == next {
        return Ok(());
    }
    let Operation::Task { request } = operation else {
        anyhow::bail!("Immutable receipt changed")
    };
    let TaskCommand::Invoke {
        task_id,
        step_id,
        request_id,
        ..
    } = &request.command
    else {
        anyhow::bail!("Immutable task receipt changed")
    };
    let Outcome::Task(prior) = prior else {
        anyhow::bail!("Expected initial tool result")
    };
    let TaskOutcome::Tool(prior) = prior.as_ref() else {
        anyhow::bail!("Expected initial tool observation")
    };
    let (previous_task, effect_id) = match prior {
        TaskToolResult::Pending {
            task_id,
            effect_id: Some(effect_id),
        }
        | TaskToolResult::Uncertain { task_id, effect_id } => (task_id, effect_id),
        _ => anyhow::bail!("Completed receipt cannot be replaced"),
    };
    ensure!(
        previous_task == task_id,
        "Initial effect task differs from intent"
    );
    let TaskOutcome::Effect(effect) = client
        .task(TaskCommand::EffectGet {
            task_id: task_id.clone(),
            effect_id: effect_id.clone(),
        })
        .await?
    else {
        anyhow::bail!("Expected authoritative effect")
    };
    ensure!(
        &effect.id == effect_id
            && &effect.task_id == task_id
            && &effect.step_id == step_id
            && &effect.request_id == request_id,
        "Observation changed logical effect binding"
    );
    let Outcome::Task(next) = next else {
        anyhow::bail!("Expected later tool observation")
    };
    let compatible = match next.as_ref() {
        TaskOutcome::Tool(TaskToolResult::Completed {
            task_id: actual,
            outcome,
        }) => {
            actual == task_id
                && effect.state == EffectState::Succeeded
                && effect.outcome.as_ref() == Some(outcome)
        }
        TaskOutcome::Tool(TaskToolResult::Pending {
            task_id: actual,
            effect_id: Some(actual_effect),
        }) => {
            actual == task_id
                && actual_effect == effect_id
                && matches!(
                    effect.state,
                    EffectState::Prepared | EffectState::Dispatched | EffectState::Succeeded
                )
        }
        TaskOutcome::Tool(TaskToolResult::Uncertain {
            task_id: actual,
            effect_id: actual_effect,
        }) => {
            actual == task_id
                && actual_effect == effect_id
                && effect.state == EffectState::Uncertain
        }
        _ => false,
    };
    ensure!(
        compatible,
        "Later result is not backed by the original effect"
    );
    Ok(())
}
