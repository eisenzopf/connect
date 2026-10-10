//! Existing read operations under a caller-owned transaction.
use super::receipts::{check_receipt_authority, receipt, valid_id};
use crate::conversations::{authorized_conversation, page_limit};
use crate::{Actor, Store};
use connect_protocol::{Error, Operation, Outcome, Result};
use rusqlite::Connection;
pub(super) fn read(
    c: &Connection,
    actor: &Actor,
    operation: &Operation,
    runtime: &crate::DiscoveryRuntime,
    store: &Store,
) -> Result<Option<Outcome>> {
    let outcome = match operation {
        Operation::Capabilities {
            address,
            conversation_id,
        } => Outcome::Capabilities(crate::discovery::capabilities(
            c,
            actor,
            address,
            conversation_id.as_deref(),
            runtime,
            store.limits(),
        )?),
        Operation::BrowserBinding {} => {
            Outcome::BrowserBinding(crate::discovery::browser_binding(c, actor, runtime)?)
        }
        Operation::ContextDeliveries {
            conversation_id,
            before,
            limit,
        } => {
            authorized_conversation(c, actor, conversation_id)?;
            page_limit(*limit, store.limits.page_size)?;
            Outcome::ContextDeliveries(crate::context::deliveries(
                c,
                actor,
                conversation_id,
                before.as_deref(),
                *limit,
            )?)
        }
        Operation::Resolve { address } => crate::conversation_queries::resolve(c, actor, address)?,
        Operation::ProviderStatus { conversation_id } => {
            authorized_conversation(c, actor, conversation_id)?;
            Outcome::ProviderStatus {
                deliveries: crate::providers::status(c, conversation_id, store.limits.page_size)?,
            }
        }
        Operation::CallStatus {
            conversation_id,
            call_id,
        } => {
            authorized_conversation(c, actor, conversation_id)?;
            Outcome::Call(crate::voice::get(c, conversation_id, call_id)?)
        }
        Operation::SessionGet {
            conversation_id,
            session_id,
        } => {
            authorized_conversation(c, actor, conversation_id)?;
            Outcome::Session(crate::sessions::get(c, conversation_id, session_id)?)
        }
        Operation::SipStatus {
            conversation_id,
            sip_leg_id,
        } => {
            authorized_conversation(c, actor, conversation_id)?;
            Outcome::SipLeg(crate::sip_legs::status(c, conversation_id, sip_leg_id)?)
        }
        Operation::AvailabilityGet { identity_id } => {
            if !actor.permits_recipient(identity_id) {
                return Err(Error::forbidden());
            }
            Outcome::Availability(crate::availability::get(c, identity_id)?)
        }
        Operation::ContextView {
            conversation_id,
            limit,
        } => {
            page_limit(*limit, store.limits.page_size)?;
            authorized_conversation(c, actor, conversation_id)?;
            Outcome::ContextView(crate::context::view(c, actor, conversation_id, *limit)?)
        }
        Operation::ArtifactGet {
            conversation_id,
            artifact_id,
            offset,
            limit,
        } => {
            authorized_conversation(c, actor, conversation_id)?;
            Outcome::ArtifactChunk(crate::artifacts::get_chunk(
                c,
                conversation_id,
                artifact_id,
                *offset,
                *limit,
                store,
            )?)
        }
        Operation::Health {} => {
            let id: String = c
                .query_row("SELECT id FROM installation", [], |r| r.get(0))
                .map_err(|_| Error::storage())?;
            Outcome::Health {
                installation_id: id,
                schema: crate::SCHEMA_VERSION,
                readiness: None,
                profile: connect_protocol::PROFILE.into(),
            }
        }
        Operation::History {
            conversation_id,
            after,
            limit,
        } => crate::conversation_queries::history(
            c,
            actor,
            conversation_id,
            after.as_deref(),
            *limit,
            store.limits.page_size,
        )?,
        Operation::Inbox {
            cursor,
            limit,
            wait_ms,
        } => crate::conversation_queries::inbox(
            c,
            actor,
            cursor.as_deref(),
            *limit,
            *wait_ms,
            store.limits.page_size,
            store.limits.wait_ms,
        )?,
        Operation::RequestStatus { request_id } => {
            valid_id(request_id)?;
            let saved = receipt(c, &actor.identity_id, request_id)?;
            if let Some(saved) = saved {
                check_receipt_authority(actor, &saved)?;
                match saved.result {
                    Ok(result) => Outcome::RequestStatus {
                        state: "accepted".into(),
                        outcome: Some(Box::new(result)),
                    },
                    Err(_) => Outcome::RequestStatus {
                        state: "rejected".into(),
                        outcome: None,
                    },
                }
            } else {
                Outcome::RequestStatus {
                    state: "not_found".into(),
                    outcome: None,
                }
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(outcome))
}
