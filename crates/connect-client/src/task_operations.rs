//! Task policy uses the same authenticated native UCTP/QUIC control connection.
use crate::{tasks as contract, ClientError, ConnectClient, Result};
use connect_protocol::{Operation, Outcome};

impl ConnectClient {
    /// Discover task support through an authorized participant's capabilities.
    /// This negotiates the profile only; each later command still checks current
    /// credential, task scope, membership, expiry and receipt authority.
    /// An old server that omits the extension is explicitly unsupported.
    pub async fn negotiate_tasks(
        &mut self,
        address: &str,
        conversation_id: Option<&str>,
    ) -> Result<()> {
        let capabilities = self.capabilities(address, conversation_id).await?;
        if !capabilities
            .extensions
            .iter()
            .any(|p| p == contract::TASK_PROFILE)
        {
            return Err(ClientError::Application(connect_protocol::Error::new(
                "unsupported-profile",
                "Server does not advertise connect-task/1",
            )));
        }
        Ok(())
    }

    /// Execute a typed command under this connection's backing credential.
    /// Call [`Self::negotiate_tasks`] or inspect capabilities first. Mutation IDs
    /// remain caller-owned across retry/reconnect; pending/uncertain is not success.
    /// Narrow model invocation grants use the separately authenticated adapter
    /// boundary, never an owner or binding argument on this method.
    ///
    /// ```no_run
    /// # async fn inspect(client: &mut connect_client::ConnectClient, cid: &str, task_id: &str) -> connect_client::Result<()> {
    /// use connect_client::tasks::{TaskCommand, TaskOutcome};
    /// client.negotiate_tasks("connect:agent@local", Some(cid)).await?;
    /// let outcome = client.task(TaskCommand::Get { task_id: task_id.into() }).await?;
    /// if let TaskOutcome::Record(record) = outcome {
    ///     assert_eq!(record.conversation_id, cid);
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn task(&mut self, command: contract::TaskCommand) -> Result<contract::TaskOutcome> {
        match self
            .request(Operation::Task {
                request: Box::new(contract::TaskRequest::new(command)),
            })
            .await?
        {
            Outcome::Task(outcome) => Ok(*outcome),
            _ => Err(ClientError::Protocol),
        }
    }
}
