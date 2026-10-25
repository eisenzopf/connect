//! Validate local canonical receipts before exposing a typed observable Operation.
use super::*;
use crate::canonical::Payload;
use connect_protocol::{membership::MembershipOutcome, Outcome};
impl Client {
    pub(super) async fn submit_local<T>(
        &self,
        kind: OperationKind<T>,
        scope: OperationScope,
        expected_revision: Option<u64>,
        body: Value,
        options: MutationOptions,
    ) -> Result<Operation<T>> {
        let key = OperationKey {
            operation_id: options.operation_id.clone(),
            command: kind.command.into(),
            scope,
        };
        if !(kind.valid_scope)(&key.scope) {
            return Err(Error::request(ErrorKind::Validation, Some(&key), false));
        }
        let mut scope = key.scope.command(Some(&key.operation_id));
        scope.expected_revision = expected_revision;
        let body = conversations::extend(body, options.extensions)?;
        let batch = self
            .request(
                kind.command,
                body.clone(),
                scope,
                Some(&key),
                options.request,
            )
            .await?;
        let fail = || Error::request(ErrorKind::Protocol, Some(&key), true);
        let Some(Payload::OperationUpdated(update)) = batch.frames.first().map(|f| f.payload())
        else {
            return Err(fail());
        };
        if update.operation_id.as_str() != key.operation_id.as_str()
            || update.revision > 9_007_199_254_740_991
            || update.problem.is_some()
            || serde_json::to_value(update.state).map_err(|_| fail())?
                != serde_json::json!("completed")
        {
            return Err(fail());
        }
        let result = update.result.clone().ok_or_else(fail)?;
        let outcome: Outcome = serde_json::from_value(result.clone()).map_err(|_| fail())?;
        // Membership receipt reads project current authorized metadata, so a
        // replay may have a later revision/role than the original input. The
        // host still enforces original intent digest and membership epoch.
        let revision = match &outcome {
            Outcome::Membership(MembershipOutcome::Participant {
                participant,
                revision,
                ..
            }) => {
                if batch.frames.len() != 3
                    || batch.frames[1].kind() != "conversation.participant.changed"
                {
                    return Err(fail());
                }
                let change = &batch.frames[1].original()["payload"];
                let action = match kind.command {
                    "conversation.participant.add" => "added",
                    "conversation.participant.update" => "updated",
                    "conversation.participant.remove" => "removed",
                    _ => return Err(fail()),
                };
                if change["kind"] != action
                    || change["participant_id"] != participant.participant_id
                    || change["membership_epoch"] != participant.membership_epoch
                {
                    return Err(fail());
                }
                if action == "added" {
                    if body["identity_id"] != participant.identity_id
                        || body["kind"]
                            != serde_json::to_value(&participant.kind).map_err(|_| fail())?
                    {
                        return Err(fail());
                    }
                } else if body["participant_id"] != participant.participant_id {
                    return Err(fail());
                }
                if action == "removed" {
                    if participant.active {
                        return Err(fail());
                    }
                } else if !participant.active {
                    return Err(fail());
                }
                *revision
            }
            Outcome::SessionControl(control) => {
                if batch.frames.len() != 2 {
                    return Err(fail());
                }
                let action = if kind.command == "session.update" {
                    body["kind"].as_str().ok_or_else(fail)?
                } else {
                    kind.command
                };
                if control.action != action {
                    return Err(fail());
                }
                control.session.topology_revision
            }
            Outcome::SessionDecision(decision) => {
                if batch.frames.len() != 2
                    || body["reason_code"] != decision.reason_code
                    || body["reason"] != decision.reason
                {
                    return Err(fail());
                }
                decision.revision
            }
            _ => return Err(fail()),
        };
        if revision > 9_007_199_254_740_991
            || expected_revision.is_some_and(|v| match v.checked_add(1) {
                Some(next) if kind.command.starts_with("conversation.participant.") => {
                    revision < next
                }
                Some(next) => revision != next,
                None => true,
            })
        {
            return Err(fail());
        }
        (kind.decode)(result, self, &key)?;
        Ok(Operation::new(
            self.clone(),
            OperationReference { key, kind },
        ))
    }
}
