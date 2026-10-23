//! Durable admission and status. Acceptance never implies a live route commit.
use crate::{ClientError, ConnectClient, Operation, Outcome, Result};
use connect_protocol::transitions::*;
impl ConnectClient {
    pub async fn admit_transition(
        &mut self,
        request: TransitionRequest,
    ) -> Result<OperationRecord> {
        match self
            .request(Operation::Transition {
                request: Box::new(request),
            })
            .await?
        {
            Outcome::DurableOperation(record) => Ok(record),
            _ => Err(ClientError::Protocol),
        }
    }
    pub async fn operation_action(&mut self, request: OperationAction) -> Result<OperationRecord> {
        match self
            .request(Operation::DurableOperation { request })
            .await?
        {
            Outcome::DurableOperation(record) => Ok(record),
            _ => Err(ClientError::Protocol),
        }
    }
}
