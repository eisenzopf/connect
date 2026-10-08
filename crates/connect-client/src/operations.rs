//! Typed application conveniences. No hidden retries or automatic logical IDs.
use crate::{ClientError, ConnectClient, Conversation, Event, Message, Operation, Outcome, Result};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendReceipt {
    pub request_id: String,
    pub message_id: String,
    pub conversation_id: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryPage {
    pub messages: Vec<Message>,
    pub next: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InboxPage {
    pub events: Vec<Event>,
    pub cursor: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestState {
    Accepted(Box<Outcome>),
    Rejected,
    NotFound,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerHealth {
    pub installation_id: String,
    pub schema: u32,
    pub profile: String,
}

impl ConnectClient {
    /// Persist request_id and inputs in the caller's state before this call.
    pub async fn create(
        &mut self,
        request_id: &str,
        participants: Vec<String>,
    ) -> Result<Conversation> {
        match self
            .request(Operation::Create {
                request_id: request_id.into(),
                participants,
            })
            .await?
        {
            Outcome::Conversation(c) => Ok(c),
            _ => Err(ClientError::Protocol),
        }
    }
    /// Success proves durable acceptance, not recipient application processing.
    pub async fn send(
        &mut self,
        request_id: &str,
        conversation_id: &str,
        body: &str,
    ) -> Result<SendReceipt> {
        match self
            .request(Operation::Send {
                request_id: request_id.into(),
                conversation_id: conversation_id.into(),
                body: body.into(),
            })
            .await?
        {
            Outcome::Accepted {
                request_id,
                message_id,
                conversation_id,
            } => Ok(SendReceipt {
                request_id,
                message_id,
                conversation_id,
            }),
            _ => Err(ClientError::Protocol),
        }
    }
    /// Page count is an upper bound; large entries may yield a shorter page.
    pub async fn history(
        &mut self,
        conversation_id: &str,
        after: Option<String>,
        limit: u32,
    ) -> Result<HistoryPage> {
        match self
            .request(Operation::History {
                conversation_id: conversation_id.into(),
                after,
                limit,
            })
            .await?
        {
            Outcome::History { messages, next } => Ok(HistoryPage { messages, next }),
            _ => Err(ClientError::Protocol),
        }
    }
    /// Persist the cursor only after durably handling its returned events.
    pub async fn inbox(
        &mut self,
        cursor: Option<String>,
        limit: u32,
        wait_ms: u32,
    ) -> Result<InboxPage> {
        match self
            .request(Operation::Inbox {
                cursor,
                limit,
                wait_ms,
            })
            .await?
        {
            Outcome::Inbox { events, cursor } => Ok(InboxPage { events, cursor }),
            _ => Err(ClientError::Protocol),
        }
    }
    /// NotFound does not imply an uncertain in-flight request cannot commit.
    pub async fn request_status(&mut self, request_id: &str) -> Result<RequestState> {
        match self
            .request(Operation::RequestStatus {
                request_id: request_id.into(),
            })
            .await?
        {
            Outcome::RequestStatus { state, outcome } => match (state.as_str(), outcome) {
                ("accepted", Some(saved)) => Ok(RequestState::Accepted(saved)),
                ("rejected", None) => Ok(RequestState::Rejected),
                ("not_found", None) => Ok(RequestState::NotFound),
                _ => Err(ClientError::Protocol),
            },
            _ => Err(ClientError::Protocol),
        }
    }
    /// Closing blocks new messages while retaining history and prior receipts.
    pub async fn close_conversation(
        &mut self,
        request_id: &str,
        conversation_id: &str,
    ) -> Result<Conversation> {
        match self
            .request(Operation::Close {
                request_id: request_id.into(),
                conversation_id: conversation_id.into(),
            })
            .await?
        {
            Outcome::Conversation(c) => Ok(c),
            _ => Err(ClientError::Protocol),
        }
    }
    pub async fn health(&mut self) -> Result<ServerHealth> {
        match self.request(Operation::Health {}).await? {
            Outcome::Health {
                installation_id,
                schema,
                profile,
            } => Ok(ServerHealth {
                installation_id,
                schema,
                profile,
            }),
            _ => Err(ClientError::Protocol),
        }
    }
    /// Cancellation stops this wait, not a transaction already accepted by the
    /// server. It closes this physical peer so abandoned waits cannot block later
    /// work. Reopen a client and recover with the original logical ID/cursor.
    pub async fn request_cancelled(
        &mut self,
        operation: Operation,
        cancel: &CancellationToken,
    ) -> Result<Outcome> {
        let result = tokio::select! {biased;_ = cancel.cancelled()=>Err(ClientError::Cancelled),result=self.request(operation)=>result};
        if matches!(result, Err(ClientError::Cancelled)) {
            self.disconnect();
        }
        result
    }
}
