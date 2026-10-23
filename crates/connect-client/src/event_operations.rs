//! Snapshot pagination and leased journal/observation reads share service authority.
use crate::{ClientError, ConnectClient, Operation, Outcome, Result};
use connect_protocol::events::*;
impl ConnectClient {
    pub async fn conversation_snapshot(
        &mut self,
        conversation_id: &str,
        query: SnapshotQuery,
    ) -> Result<SnapshotPage> {
        match self
            .request(Operation::ConversationSnapshot {
                conversation_id: conversation_id.into(),
                query,
            })
            .await?
        {
            Outcome::ConversationSnapshot(page) => Ok(page),
            _ => Err(ClientError::Protocol),
        }
    }
    pub async fn conversation_events(
        &mut self,
        conversation_id: &str,
        query: EventQuery,
    ) -> Result<EventPage> {
        match self
            .request(Operation::ConversationEvents {
                conversation_id: conversation_id.into(),
                query,
            })
            .await?
        {
            Outcome::ConversationEvents(page) => Ok(page),
            _ => Err(ClientError::Protocol),
        }
    }
    pub async fn session_observations(
        &mut self,
        conversation_id: &str,
        session_id: &str,
        query: ObservationQuery,
    ) -> Result<ObservationPage> {
        match self
            .request(Operation::SessionObservations {
                conversation_id: conversation_id.into(),
                session_id: session_id.into(),
                query,
            })
            .await?
        {
            Outcome::SessionObservations(page) => Ok(page),
            _ => Err(ClientError::Protocol),
        }
    }
}
