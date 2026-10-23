//! One explicit draft-response workflow; model text never selects a task.
use super::{
    commands::{logical_id, Lane, Prepared},
    driver::Clients,
    journal::Page,
    reconciliation::{consume_conflict, consumed},
    state::TaskStore,
};
use anyhow::{Context, Result};
use connect_client::{
    tasks::{TaskCommand, TaskEventData, TaskOutcome, TaskStepClaim},
    ClientError, Operation,
};
use tokio_util::sync::CancellationToken;

pub(super) async fn drive(
    clients: &mut Clients,
    state: &mut TaskStore,
    stop: &CancellationToken,
) -> Result<()> {
    question(clients, state).await?;
    loop {
        if stop.is_cancelled() {
            return Ok(());
        }
        clients.current(&state.binding).await?;
        for command in state.pending_invocations()? {
            dispatch(clients, state, command).await?;
        }
        let page = if let Some(page) = state.page()? {
            page
        } else {
            let outcome = clients
                .invocation
                .task(TaskCommand::Wait {
                    task_id: state.binding.task_id.clone(),
                    after: state.metadata("cursor")?,
                    limit: 100,
                    wait_ms: 100,
                })
                .await?;
            let TaskOutcome::Inbox { events, next } = outcome else {
                anyhow::bail!("Unexpected task inbox outcome");
            };
            let page = Page { events, next };
            state.stage_page(&page)?;
            page
        };
        for event in &page.events {
            if state.processed(event)? {
                continue;
            }
            if let TaskEventData::Reply(reply) = &event.data {
                if reply.consumed {
                    continue;
                }
                // The server's CAS consume owns the reply before a local action.
                // A crash afterwards replays this exact saved command/receipt.
                consume(clients, state, &reply.event_id).await?;
            }
        }
        // Current delegated authority is checked even when a page/receipt was
        // saved on an earlier connection. It cannot bypass revoke/cancel/expiry.
        clients.current(&state.binding).await?;
        state.commit_page(&page)?;
    }
}

async fn consume(clients: &mut Clients, state: &mut TaskStore, event: &str) -> Result<()> {
    for attempt in 0..3 {
        let record = clients
            .controller
            .negotiate_task(&state.binding.task_id)
            .await?;
        let command = state.consumption(event, record.revision)?;
        match dispatch(clients, state, command).await {
            Ok(outcome) => return consumed(&outcome),
            Err(error) if attempt < 2 && consume_conflict(&error) => {
                println!("reconcile control=consume known-rejection=true");
            }
            Err(error) => return Err(error),
        }
    }
    anyhow::bail!("Consume reconciliation budget exhausted")
}

async fn question(clients: &mut Clients, state: &mut TaskStore) -> Result<()> {
    if let Some(command) = state.command("question")? {
        dispatch(clients, state, command).await?;
        return Ok(());
    }
    let claim = match clients
        .controller
        .task(TaskCommand::ClaimGet {
            task_id: state.binding.task_id.clone(),
            step_id: "draft-question".into(),
        })
        .await?
    {
        TaskOutcome::Claim(claim) => claim,
        _ => anyhow::bail!("Unexpected claim lookup outcome"),
    };
    let prepared = state.prepare(
        "question-claim",
        Lane::Controller,
        TaskCommand::Claim {
            request_id: logical_id(),
            task_id: state.binding.task_id.clone(),
            step_id: "draft-question".into(),
            worker_id: state.worker.clone(),
            expected_revision: claim.as_ref().map(|c| c.revision),
        },
    )?;
    let outcome = dispatch(clients, state, prepared).await?;
    let TaskOutcome::Claim(Some(TaskStepClaim { worker_id, .. })) = outcome else {
        anyhow::bail!("Expected bounded step lease");
    };
    anyhow::ensure!(worker_id == state.worker, "Claim belongs to another worker");
    // Explicit example intent shared by workers of this task. Lease expiry or
    // a lost receipt cannot produce a second independently named question.
    let request_id = format!("draft-question-{}", state.binding.task_id);
    let command = state.prepare(
        "question",
        Lane::Invocation,
        TaskCommand::Invoke {
            request_id: request_id.clone(),
            task_id: state.binding.task_id.clone(),
            step_id: "draft-question".into(),
            operation: Box::new(Operation::Send {
                request_id,
                conversation_id: state.binding.conversation_id.clone(),
                message: connect_client::messages::MessageDraft::text_all(
                    "Please review draft-002 and reply to this question.",
                ),
            }),
            reply_to: None,
            wait_for_reply: true,
        },
    )?;
    dispatch(clients, state, command).await?;
    Ok(())
}

async fn dispatch(
    clients: &mut Clients,
    state: &mut TaskStore,
    prepared: Prepared,
) -> Result<TaskOutcome> {
    if prepared.lane == Lane::Invocation {
        if let Some(outcome) = prepared.outcome {
            return Ok(outcome);
        }
    }
    #[cfg(feature = "test-support")]
    super::fault("before-dispatch");
    #[cfg(feature = "test-support")]
    let consume = prepared.key.starts_with("consume-");
    #[cfg(feature = "test-support")]
    if consume {
        super::fault("before-consume-dispatch");
    }
    let result = match prepared.lane {
        Lane::Controller => clients.controller.task(prepared.command).await,
        Lane::Invocation => clients.invocation.task(prepared.command).await,
    };
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(ClientError::Application(error)) => {
            state.rejected(&prepared.key, &error.code)?;
            return Err(ClientError::Application(error).into());
        }
        Err(error) => return Err(error.into()),
    };
    #[cfg(feature = "test-support")]
    {
        super::fault("after-ack");
        if prepared.lane == Lane::Invocation {
            super::fault("after-invocation-ack");
        }
        if consume {
            super::fault("after-consume-ack");
        }
    }
    state
        .accepted(&prepared.key, &outcome)
        .context("Retain command state to reconcile server acceptance")?;
    #[cfg(feature = "test-support")]
    if prepared.key.starts_with("consume-") {
        super::fault("after-consume");
    }
    Ok(outcome)
}
