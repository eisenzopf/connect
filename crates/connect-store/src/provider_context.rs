//! Durable, source-attributed observations from an authenticated provider leg.

use crate::Store;
use connect_protocol::{Error, Message, Result};
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const MAX_PROVIDER_REFERENCE_BYTES: usize = 512;

struct SessionAuthority {
    conversation_id: String,
    initiator_id: String,
    participant_id: String,
}

impl Store {
    /// Record incomplete provider context as a Connect observation, rather
    /// than inventing a provider transcript or successful end report.
    pub fn record_session_context_gap(
        &self,
        session_id: &str,
        provider_ref: &str,
        missing: &str,
    ) -> Result<Message> {
        if !matches!(missing, "end_report" | "transcript_events") {
            return Err(invalid("Invalid provider context gap"));
        }
        let body = serde_json::to_string(&json!({
            "provider": "vapi", "state": "incomplete", "missing": [missing],
            "source": "connect-bridge",
        }))
        .map_err(|_| Error::storage())?;
        self.append_session_observation(
            session_id,
            &format!("{provider_ref}:missing-{missing}"),
            "assistant",
            "voice_context_status",
            "connect-observation",
            &body,
        )
    }

    /// Append one final provider transcript to the durable conversation.
    ///
    /// The provider stream is an internal authenticated adapter boundary. The
    /// durable Session chooses the conversation and actor; provider content can
    /// never name either. A deterministic source ID makes repeated delivery of
    /// the same final transcript idempotent.
    pub fn record_session_transcript(
        &self,
        session_id: &str,
        provider_ref: &str,
        role: &str,
        transcript_type: &str,
        body: &str,
    ) -> Result<Option<Message>> {
        if !transcript_type.eq_ignore_ascii_case("final") {
            return Ok(None);
        }
        if !matches!(role, "user" | "assistant")
            || body.is_empty()
            || body.len() > self.limits.message_bytes
        {
            return Err(invalid("Invalid final provider transcript"));
        }
        self.append_session_observation(
            session_id,
            provider_ref,
            role,
            "voice_transcript",
            transcript_type,
            body,
        )
        .map(Some)
    }

    /// Append the bounded, non-sensitive part of a provider end report.
    /// Unknown report fields are deliberately excluded from the conversation.
    pub fn record_session_outcome(
        &self,
        session_id: &str,
        provider_ref: &str,
        report: &Value,
    ) -> Result<Message> {
        let ended_reason = bounded_string(report.get("endedReason"), 256);
        let summary = bounded_string(report.pointer("/analysis/summary"), 4_096)
            .or_else(|| bounded_string(report.get("summary"), 4_096));
        let success = report
            .pointer("/analysis/successEvaluation")
            .and_then(Value::as_str)
            .filter(|value| value.len() <= 128);
        let body = serde_json::to_string(&json!({
            "provider": "vapi",
            "status": "ended",
            "ended_reason": ended_reason,
            "summary": summary,
            "success_evaluation": success,
        }))
        .map_err(|_| Error::storage())?;
        if body.len() > self.limits.message_bytes {
            return Err(invalid("Provider outcome exceeds configured bounds"));
        }
        self.append_session_observation(
            session_id,
            provider_ref,
            "assistant",
            "voice_outcome",
            "end-of-call-report",
            &body,
        )
    }

    fn append_session_observation(
        &self,
        session_id: &str,
        provider_ref: &str,
        role: &str,
        kind: &str,
        source_type: &str,
        body: &str,
    ) -> Result<Message> {
        validate_provider_ref(provider_ref)?;
        let mut connection = self.connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let authority = session_authority(&tx, session_id)?;
        let sender = if role == "user" {
            authority.initiator_id.clone()
        } else {
            authority.participant_id.clone()
        };
        let digest = hex::encode(Sha256::digest(
            format!("{session_id}\0{provider_ref}\0{kind}\0{source_type}").as_bytes(),
        ));
        let canonical_id = format!("voice:{session_id}:{}", &digest[..32]);
        let (id, event_kind, stored_body) = match existing_message(&tx, &canonical_id)? {
            None => (canonical_id, kind.to_owned(), body.to_owned()),
            Some(existing) if existing.body == body => {
                tx.commit().map_err(|_| Error::storage())?;
                return Ok(existing);
            }
            Some(existing) => {
                let correction_body = serde_json::to_string(&json!({
                    "corrects_source_id": existing.id,
                    "body": body,
                }))
                .map_err(|_| Error::storage())?;
                if correction_body.len() > self.limits.message_bytes {
                    return Err(invalid("Provider correction exceeds configured bounds"));
                }
                let correction_digest = hex::encode(Sha256::digest(
                    format!("{}\0{correction_body}", existing.id).as_bytes(),
                ));
                let correction_id =
                    format!("voice-correction:{session_id}:{}", &correction_digest[..32]);
                if let Some(correction) = existing_message(&tx, &correction_id)? {
                    tx.commit().map_err(|_| Error::storage())?;
                    return Ok(correction);
                }
                (correction_id, format!("{kind}_correction"), correction_body)
            }
        };
        let recipients = recipients(&tx, &authority.conversation_id)?;
        if !recipients.iter().any(|identity| identity == &sender) {
            return Err(Error::forbidden());
        }
        if !crate::conversations::capacity(
            &tx,
            recipients.len() as u64,
            stored_body.len() as u64,
            self,
        )? {
            return Err(Error::new(
                "capacity-exceeded",
                "Durable provider observation capacity exceeded",
            ));
        }
        tx.execute(
            "INSERT INTO messages(id,conversation_id,sender,body) VALUES(?1,?2,?3,?4)",
            params![id, authority.conversation_id, sender, stored_body],
        )
        .map_err(|_| Error::storage())?;
        for recipient in recipients {
            crate::conversations::insert_event(
                &tx,
                &recipient,
                &authority.conversation_id,
                &event_kind,
                Some(&id),
            )?;
        }
        let message = Message {
            id,
            conversation_id: authority.conversation_id,
            sender,
            body: stored_body,
        };
        tx.commit().map_err(|_| Error::storage())?;
        Ok(message)
    }
}

fn session_authority(tx: &Transaction<'_>, session_id: &str) -> Result<SessionAuthority> {
    let (conversation_id, initiator_id, participant_id, state): (String, String, String, String) =
        tx.query_row(
            "SELECT conversation_id,initiator_id,participant_id,state FROM sessions WHERE id=?1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|_| Error::storage())?
        .ok_or_else(Error::forbidden)?;
    if !matches!(state.as_str(), "accepted" | "ready" | "ended") {
        return Err(Error::forbidden());
    }
    Ok(SessionAuthority {
        conversation_id,
        initiator_id,
        participant_id,
    })
}

fn recipients(tx: &Transaction<'_>, conversation_id: &str) -> Result<Vec<String>> {
    let mut statement = tx
        .prepare(
            "SELECT identity_id FROM participants WHERE conversation_id=?1 ORDER BY identity_id",
        )
        .map_err(|_| Error::storage())?;
    let values = statement
        .query_map([conversation_id], |row| row.get::<_, String>(0))
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    Ok(values)
}

fn existing_message(tx: &Transaction<'_>, id: &str) -> Result<Option<Message>> {
    tx.query_row(
        "SELECT id,conversation_id,sender,body FROM messages WHERE id=?1",
        [id],
        |row| {
            Ok(Message {
                id: row.get(0)?,
                conversation_id: row.get(1)?,
                sender: row.get(2)?,
                body: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(|_| Error::storage())
}

fn validate_provider_ref(provider_ref: &str) -> Result<()> {
    if provider_ref.is_empty() || provider_ref.len() > MAX_PROVIDER_REFERENCE_BYTES {
        return Err(invalid("Invalid provider reference"));
    }
    Ok(())
}

fn bounded_string(value: Option<&Value>, limit: usize) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .filter(|value| value.len() <= limit)
}

fn invalid(message: &str) -> Error {
    Error::new("invalid-request", message)
}
