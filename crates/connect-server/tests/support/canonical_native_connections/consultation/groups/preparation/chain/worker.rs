//! Example external worker: only public clients; an observation is never a grant.
use connect_client::{
    application::{ApplicationClient, ApplicationClientConfig, ApplicationOperation},
    canonical::{CanonicalHttpClient, CommandScope},
    ClientError, Result,
};
use connect_protocol::{events::*, tasks::*, transitions::TransitionCommand, Operation, Outcome};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// Trusted application approval persisted BEFORE sending any mutation. Retain
/// this full command (including its original revisions) across worker restarts.
#[derive(Serialize, Deserialize)]
pub(super) struct Decision {
    pub work: ObservationWork,
    pub command: TransitionCommand,
    pub reply_address: String,
}
pub(super) struct Worker {
    pub decision: Decision,
    api: ApplicationClient,
    core: CanonicalHttpClient,
    replies: ApplicationClient,
}
impl Worker {
    pub async fn open(
        path: &Path,
        inspection: ApplicationClientConfig,
        execution: ApplicationClientConfig,
        response: ApplicationClientConfig,
    ) -> Result<Self> {
        let decision: Decision =
            serde_json::from_slice(&std::fs::read(path).map_err(|_| ClientError::Configuration)?)
                .map_err(|_| ClientError::Configuration)?;
        if decision.command.operation_id != format!("agent-answer-{}", decision.work.question_id) {
            return Err(ClientError::Configuration);
        }
        let mut api = ApplicationClient::open(inspection).await?;
        api.negotiate_task(&decision.work.task_id).await?;
        let mut replies = ApplicationClient::open(response).await?;
        replies
            .negotiate_tasks(
                &decision.reply_address,
                Some(&decision.command.conversation_id),
            )
            .await?;
        Ok(Self {
            decision,
            api,
            core: CanonicalHttpClient::open(execution).await?,
            replies,
        })
    }
    async fn task(&mut self, command: TaskCommand) -> Result<TaskOutcome> {
        let op = ApplicationOperation::try_from(Operation::Task {
            request: Box::new(TaskRequest::new(command)),
        })?;
        match self.api.execute(op).await? {
            Outcome::Task(outcome) => Ok(*outcome),
            _ => Err(ClientError::Protocol),
        }
    }
    async fn inspect(&mut self) -> Result<()> {
        let work = self.decision.work.clone();
        let TaskOutcome::Record(task) = self
            .task(TaskCommand::Get {
                task_id: work.task_id.clone(),
            })
            .await?
        else {
            return Err(ClientError::Protocol);
        };
        if task.id != work.task_id
            || task.conversation_id != self.decision.command.conversation_id
            || task.state.is_terminal()
            || task.expires_at <= chrono::Utc::now().timestamp()
        {
            return Err(ClientError::Application(
                connect_protocol::Error::forbidden(),
            ));
        }
        let TaskOutcome::Effect(effect) = self
            .task(TaskCommand::EffectGet {
                task_id: work.task_id,
                effect_id: work.effect_id.clone(),
            })
            .await?
        else {
            return Err(ClientError::Protocol);
        };
        if effect.id != work.effect_id
            || !matches!(effect.outcome.as_deref(),Some(Outcome::Accepted {message_id,conversation_id,..}) if message_id==&work.question_id && conversation_id==&task.conversation_id)
        {
            return Err(ClientError::Protocol);
        }
        Ok(())
    }
    pub async fn observe(&mut self, observation: &SessionObservation) -> Result<bool> {
        if !matches!(&observation.data,ObservationDraft::Agent {observation} if matches!(&observation.content,AgentObservationContent::Action {name:Some(name),phase:AgentActionPhase::Requested,..} if name=="handoff"))
        {
            return Ok(false);
        }
        if observation.work.as_ref() != Some(&self.decision.work)
            || observation.conversation_id != self.decision.command.conversation_id
        {
            return Err(ClientError::Protocol);
        }
        self.inspect().await?;
        // This command and its explicit current Core credential come from the
        // independent application approval. No tool arguments or provider IDs
        // are used to choose its target, logical operation ID or authority.
        self.core.transition(self.decision.command.clone()).await?;
        Ok(true)
    }
    pub async fn completion(&mut self) -> Result<Value> {
        let cmd = &self.decision.command;
        let result = self
            .core
            .operation_get(
                &cmd.operation_id,
                CommandScope {
                    conversation_id: Some(cmd.conversation_id.clone()),
                    session_id: Some(cmd.session_id.clone()),
                    ..Default::default()
                },
            )
            .await?;
        serde_json::to_value(result).map_err(|_| ClientError::Protocol)
    }
    pub async fn reply_completed(&mut self) -> Result<TaskOutcome> {
        let actual = self.completion().await?;
        if actual["state"] != "completed" {
            return Err(ClientError::Application(connect_protocol::Error::new(
                "not-ready",
                "Actual operation has not completed",
            )));
        }
        self.inspect().await?;
        let command = TaskCommand::Reply {
            request_id: format!("{}-reply", self.decision.command.operation_id),
            conversation_id: self.decision.command.conversation_id.clone(),
            reply_to: self.decision.work.question_id.clone(),
            body: format!(
                "Connect operation {} completed",
                self.decision.command.operation_id
            ),
        };
        let op = ApplicationOperation::try_from(Operation::Task {
            request: Box::new(TaskRequest::new(command)),
        })?;
        match self.replies.execute(op).await? {
            Outcome::Task(result) => Ok(*result),
            _ => Err(ClientError::Protocol),
        }
    }
}
