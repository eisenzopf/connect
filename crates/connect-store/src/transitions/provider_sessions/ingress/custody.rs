//! Account-scoped private routing before any common Session admission. Call
//! identity is immutable; discovery state/control metadata are observations.
use super::*;
use crate::incoming_contacts::{self as contacts, Candidate};
use connect_protocol::incoming_contacts::*;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Snapshot {
    pub(super) ingress: ProviderVoiceIngressPolicy,
    pub(super) binding: Binding,
    pub(super) since: i64,
}
struct Selection {
    candidates: Vec<Candidate>,
    historical: usize,
}
fn select(c: &Connection, scan: &ProviderVoiceScan, r: &InboundVoiceReceipt) -> Result<Selection> {
    // Retained withdrawn/dormant routes still make a late call ambiguous. A
    // different owner cannot turn the same actual configured account into a
    // second resource namespace.
    let mut q = c.prepare("SELECT id,conversation_id,policy_json,source_json,since FROM provider_voice_ingress WHERE json_extract(policy_json,'$.provider')=?1 AND json_extract(policy_json,'$.route_reference')=?2 AND json_extract(policy_json,'$.assistant_id')=?3 AND json_extract(policy_json,'$.phone_number_id')=?4 AND json_extract(source_json,'$.endpoint.address')=?5 ORDER BY id LIMIT 65").map_err(|_|Error::storage())?;
    let rows = q
        .query_map(
            params![
                scan.ingress.policy.provider,
                scan.ingress.policy.route_reference,
                r.call.assistant_id,
                r.call.phone_number_id,
                r.call.caller
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            },
        )
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    if rows.len() > 64 {
        return Err(Error::new(
            "capacity-exceeded",
            "Incoming voice candidate budget exhausted",
        ));
    }
    let historical = rows.len();
    let mut candidates = vec![];
    for (id, conversation_id, policy, saved, since) in rows {
        let p = ProviderVoiceIngressPolicy {
            id,
            conversation_id,
            policy: serde_json::from_str(&policy).map_err(|_| Error::storage())?,
        };
        let b: Binding = serde_json::from_str(&saved).map_err(|_| Error::storage())?;
        if b.route_owner_id != scan.binding.route_owner_id {
            return Err(Error::new(
                "provider-binding-conflict",
                "Incoming provider account has conflicting ownership",
            ));
        }
        match current(c, &scan.worker, &p, &b) {
            Ok(()) if since <= r.call.created_at => {}
            Ok(()) => continue,
            Err(e)
                if matches!(
                    e.code.as_str(),
                    "forbidden" | "unauthorized" | "conversation-closed"
                ) =>
            {
                continue
            }
            Err(e) => return Err(e),
        }
        let member = &b.target;
        candidates.push(
            Candidate {
                view: IncomingContactCandidate {
                    id: String::new(),
                    route_id: p.id.clone(),
                    conversation_id: p.conversation_id.clone(),
                    endpoint_id: p.policy.endpoint_id.clone(),
                    participant_id: member.participant_id.clone(),
                    membership_epoch: member.membership_epoch,
                },
                identity_id: member.identity_id.clone(),
                source: serde_json::to_value(Snapshot {
                    ingress: p,
                    binding: b,
                    since,
                })
                .map_err(|_| Error::storage())?,
            }
            .seal()?,
        );
    }
    Ok(Selection {
        candidates,
        historical,
    })
}

pub(super) fn scopes(
    c: &Connection,
    scan: &ProviderVoiceScan,
    receipts: &[InboundVoiceReceipt],
) -> Result<Vec<AuthorityScope>> {
    current(c, &scan.worker, &scan.ingress, &scan.binding)?;
    let mut scopes = scope_set(c, &scan.ingress, &scan.binding, crate::AuthorityMode::Write)?;
    for r in receipts {
        for candidate in select(c, scan, r)?.candidates {
            let s: Snapshot =
                serde_json::from_value(candidate.source).map_err(|_| Error::storage())?;
            scopes.extend(scope_set(
                c,
                &s.ingress,
                &s.binding,
                crate::AuthorityMode::Write,
            )?);
        }
    }
    Ok(scopes)
}

pub(super) struct Admission {
    pub hold: String,
    pub candidate: Candidate,
    pub existing: Option<String>,
}
pub(super) fn admit(
    c: &Connection,
    store: &Store,
    scan: &ProviderVoiceScan,
    r: &InboundVoiceReceipt,
) -> Result<Option<Admission>> {
    let selected = select(c, scan, r)?;
    if selected.candidates.is_empty() {
        return Ok(None);
    }
    let reason = if selected.candidates.len() > 1 {
        IncomingContactReason::AmbiguousRoute
    } else if selected.historical > 1 {
        IncomingContactReason::HistoricalOverlap
    } else {
        IncomingContactReason::Routed
    };
    let payload = json!({"kind":"voice","provider_id":r.call.provider_id,"assistant_id":r.call.assistant_id,"phone_number_id":r.call.phone_number_id,"caller":r.call.caller,"created_at":r.call.created_at});
    let event = contacts::encoded(
        &json!({"resource":payload,"state":r.call.state,"verification":r.verification,"control_url":r.call.control_url,"ended_reason":r.call.ended_reason}),
    )?;
    let hold = contacts::capture(
        c,
        &contacts::Capture {
            source: contacts::Source {
                kind: IncomingContactKind::Voice,
                provider: &scan.ingress.policy.provider,
                account_reference: &scan.ingress.policy.route_reference,
                credential_id: &scan.ingress.policy.route_credential_id,
                binding: json!({"route_owner_id":scan.binding.route_owner_id,"account_reference":scan.ingress.policy.route_reference}),
            },
            provider_id: &r.call.provider_id,
            event_id: &r.event_id,
            event_digest: &hex::encode(Sha256::digest(event.as_bytes())),
            payload,
            candidates: selected.candidates,
            reason,
        },
        store,
    )?;
    let (reason, raw, rejected): (String,String,bool) = c.query_row("SELECT reason,candidates_json,EXISTS(SELECT 1 FROM incoming_contact_rejections WHERE hold_id=h.id) FROM incoming_contact_holds h WHERE id=?1",[&hold],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|Error::storage())?;
    if rejected {
        return Ok(None);
    }
    let delivered: Option<(String,String)> = c.query_row("SELECT candidate_json,provider_call_id FROM incoming_contact_deliveries WHERE hold_id=?1",[&hold],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
    let (candidate, existing) = if let Some((raw, id)) = delivered {
        (
            serde_json::from_str::<Candidate>(&raw).map_err(|_| Error::storage())?,
            Some(id),
        )
    } else {
        // A retry may never narrow an original hold down to one remaining room.
        if reason != "routed" {
            return Ok(None);
        }
        let mut candidates: Vec<Candidate> =
            serde_json::from_str(&raw).map_err(|_| Error::storage())?;
        if candidates.len() != 1 {
            return Err(Error::storage());
        }
        (candidates.remove(0), None)
    };
    let s: Snapshot =
        serde_json::from_value(candidate.source.clone()).map_err(|_| Error::storage())?;
    if s.ingress != scan.ingress {
        return Ok(None);
    }
    current(c, &scan.worker, &s.ingress, &s.binding)?;
    if s.binding != scan.binding || s.since > r.call.created_at {
        return Err(Error::forbidden());
    }
    Ok(Some(Admission {
        hold,
        candidate,
        existing,
    }))
}

pub(super) fn delivered(
    c: &Connection,
    store: &Store,
    scan: &ProviderVoiceScan,
    a: &Admission,
    call: &str,
) -> Result<()> {
    let actor = actor_from_connection(c, &scan.ingress.policy.route_credential_id)?;
    let operation = format!("icvoice_{}", hex::encode(Sha256::digest(a.hold.as_bytes())));
    let intent = contacts::encoded(
        &json!({"contact_id":a.hold,"candidate_id":a.candidate.view.id,"expected_revision":0}),
    )?;
    bind(
        c,
        store,
        a,
        call,
        &actor,
        &operation,
        "incoming-contact.voice-auto",
        &intent,
    )
}
pub(super) fn bind(
    c: &Connection,
    store: &Store,
    a: &Admission,
    call: &str,
    actor: &crate::Actor,
    operation: &str,
    kind: &str,
    intent: &str,
) -> Result<()> {
    let resource: String = c
        .query_row(
            "SELECT resource_id FROM incoming_contact_holds WHERE id=?1",
            [&a.hold],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let outcome = json!({"kind":"session","provider_call_id":call,"conversation_id":a.candidate.view.conversation_id,"remote_identity_verified":false,"local_media":false});
    c.execute("INSERT INTO incoming_contact_deliveries(resource_id,hold_id,candidate_id,candidate_json,provider_call_id,outcome_json,actor_identity_id,actor_credential_id,operation_id,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![resource,a.hold,a.candidate.view.id,contacts::encoded(&a.candidate)?,call,contacts::encoded(&outcome)?,actor.identity_id,actor.credential_id,operation,crate::now()]).map_err(|_|conflict())?;
    c.execute("INSERT INTO incoming_contact_audit(hold_id,event_type,actor_identity_id,actor_credential_id,operation_id,created_at) VALUES(?1,'resolved',?2,?3,?4,?5)",params![a.hold,actor.identity_id,actor.credential_id,operation,crate::now()]).map_err(|_|Error::storage())?;
    contacts::record(c, actor, operation, kind, intent, &a.hold, store)
}
