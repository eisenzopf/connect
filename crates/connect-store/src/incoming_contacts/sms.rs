//! Verified SMS admission uses configured account/Endpoint policy, never caller
//! numbers as authentication. All message/journal/custody effects are atomic.
use super::*;
use crate::{
    conversations::authorized_conversation, providers::route, ProviderMessageEndpoint, Route,
};
use connect_protocol::{
    membership::MemberView,
    messages::{MessageAudience, MessageDraft},
};

/// Protected adapter input after exact raw-body/timestamp verification. Resource
/// identity is separate from callback event identity and retry metadata.
pub struct SmsObservation<'a> {
    pub event_id: &'a str,
    pub event_digest: &'a str,
    pub provider_id: &'a str,
    pub from: &'a str,
    pub to: &'a str,
    pub body: &'a str,
    pub occurred_at: Option<i64>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    route: Route,
    policy: ProviderMessageEndpoint,
    binding: Value,
    audience: Vec<MemberView>,
    consent: Option<String>,
}
struct Selection {
    candidates: Vec<Candidate>,
    source: Snapshot,
    historical: usize,
}
const MATCHES: &str = "SELECT id FROM connector_routes WHERE connector='telnyx' AND target=?1 AND source=?2 AND (?3 IS NULL OR id IN (SELECT value FROM json_each(?3))) ORDER BY id";
fn snapshot(c: &Connection, r: Route) -> Result<Snapshot> {
    let p = crate::provider_message_endpoints::current(c, &r)?.ok_or_else(Error::forbidden)?;
    let actor = actor_from_connection(c, &r.credential_id)?;
    actor.require("send")?;
    let account = actor_from_connection(c, &p.account_credential_id)?;
    account.require("admin")?;
    let audience = crate::audience::select_members(c, &actor, &r.conversation_id, &r.incoming_to)?;
    if audience
        .iter()
        .any(|m| !account.permits_recipient(&m.identity_id))
    {
        return Err(Error::forbidden());
    }
    let binding: String = c
        .query_row(
            "SELECT source_json FROM provider_message_endpoints WHERE route_id=?1",
            [&r.id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let consent = crate::sms_eligibility::eligible(
        c,
        &r.target,
        r.source.as_deref().unwrap_or_default(),
        &r.conversation_id,
    )?
    .ok();
    Ok(Snapshot {
        route: r,
        policy: p,
        binding: serde_json::from_str(&binding).map_err(|_| Error::storage())?,
        audience,
        consent,
    })
}
fn candidate(s: Snapshot) -> Result<Candidate> {
    let member = s
        .audience
        .iter()
        .find(|m| m.identity_id == s.route.identity_id)
        .ok_or_else(Error::forbidden)?;
    Candidate {
        view: IncomingContactCandidate {
            id: String::new(),
            route_id: s.route.id.clone(),
            conversation_id: s.route.conversation_id.clone(),
            endpoint_id: s.policy.endpoint_id.clone(),
            participant_id: member.participant_id.clone(),
            membership_epoch: member.membership_epoch,
        },
        identity_id: s.route.identity_id.clone(),
        source: serde_json::to_value(s).map_err(|_| Error::storage())?,
    }
    .seal()
}
fn select(c: &Connection, input: &SmsObservation<'_>, scope: &Option<String>) -> Result<Selection> {
    let mut q = c.prepare(MATCHES).map_err(|_| Error::storage())?;
    let ids = q
        .query_map(params![input.from, input.to, scope], |r| {
            r.get::<_, String>(0)
        })
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    if ids.len() > 64 {
        return Err(Error::new(
            "capacity-exceeded",
            "Incoming SMS candidate budget exhausted",
        ));
    }
    let mut snapshots = vec![];
    for id in ids {
        match snapshot(c, route(c, &id)?) {
            Ok(s) => snapshots.push(s),
            Err(e)
                if matches!(
                    e.code.as_str(),
                    "forbidden" | "unauthorized" | "conversation-closed"
                ) =>
            {
                ()
            }
            Err(e) => return Err(e),
        }
    }
    let source = snapshots.first().cloned().ok_or_else(Error::forbidden)?;
    let source_owner = actor_from_connection(c, &source.policy.account_credential_id)?;
    for s in &snapshots {
        let owner = actor_from_connection(c, &s.policy.account_credential_id)?;
        if owner.identity_id != source_owner.identity_id
            || s.policy.secret_reference != source.policy.secret_reference
        {
            return Err(conflict());
        }
    }
    // Disabled/unmounted/closed original policies still make a late reply
    // ambiguous. Count only this configured account's retained routing facts.
    let historical:u64=c.query_row("SELECT count(*) FROM connector_routes r JOIN provider_message_endpoints p ON p.route_id=r.id WHERE r.connector='telnyx' AND r.target=?1 AND r.source=?2 AND json_extract(p.policy_json,'$.secret_reference')=?3 AND json_extract(p.source_json,'$.account_owner')=?4",params![input.from,input.to,source.policy.secret_reference,source_owner.identity_id],|r|r.get(0)).map_err(|_|Error::storage())?;
    let mut candidates = vec![];
    for mut s in snapshots {
        if let Some(at) = input.occurred_at {
            let created: i64 = c
                .query_row(
                    "SELECT created_at FROM conversations WHERE id=?1",
                    [&s.route.conversation_id],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            if at < created {
                s.consent = None;
            }
        }
        candidates.push(candidate(s)?);
    }
    Ok(Selection {
        source,
        candidates,
        historical: historical as usize,
    })
}
fn scopes(
    c: &Connection,
    input: &SmsObservation<'_>,
    scope: &Option<String>,
) -> Result<Vec<AuthorityScope>> {
    let selected = select(c, input, scope)?;
    let mut scopes = vec![];
    for candidate in &selected.candidates {
        scopes.extend(crate::authority_admission::conversation_scopes(
            c,
            &candidate.view.conversation_id,
            AuthorityMode::Write,
        )?);
    }
    scopes.extend(crate::provider_message_endpoints::account(c, &selected.source.policy)?.1);
    Ok(scopes)
}
fn checked(c: &Connection, candidate: &Candidate, consent: bool) -> Result<Snapshot> {
    let saved: Snapshot =
        serde_json::from_value(candidate.source.clone()).map_err(|_| invalid())?;
    let r = route(c, &saved.route.id)?;
    let p = crate::provider_message_endpoints::current(c, &r)?.ok_or_else(Error::forbidden)?;
    let actual: String = c
        .query_row(
            "SELECT source_json FROM provider_message_endpoints WHERE route_id=?1",
            [&r.id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if r != saved.route
        || p != saved.policy
        || serde_json::from_str::<Value>(&actual).map_err(|_| Error::storage())? != saved.binding
    {
        return Err(Error::forbidden());
    }
    let actor = actor_from_connection(c, &r.credential_id)?;
    actor.require("send")?;
    let account = actor_from_connection(c, &p.account_credential_id)?;
    if authorized_conversation(c, &actor, &r.conversation_id)?.closed {
        return Err(Error::forbidden());
    }
    for m in &saved.audience {
        if crate::audience::member(c, &m.identity_id, &r.conversation_id, true)? != *m
            || !actor.permits_recipient(&m.identity_id)
            || !account.permits_recipient(&m.identity_id)
        {
            return Err(Error::forbidden());
        }
    }
    if consent {
        let current = crate::sms_eligibility::eligible(
            c,
            &r.target,
            r.source.as_deref().unwrap_or_default(),
            &r.conversation_id,
        )?
        .ok();
        if saved.consent.is_none() || current != saved.consent {
            return Err(Error::forbidden());
        }
    }
    Ok(saved)
}
fn deliver(
    c: &Connection,
    hold: &str,
    candidate: &Candidate,
    actor: &Actor,
    operation: &str,
    store: &Store,
) -> Result<()> {
    let saved = checked(c, candidate, true)?;
    if saved
        .audience
        .iter()
        .any(|m| !actor.permits_recipient(&m.identity_id))
    {
        return Err(Error::forbidden());
    }
    let (resource, payload): (String, String) = c
        .query_row(
            "SELECT resource_id,payload_json FROM incoming_contact_holds WHERE id=?1",
            [hold],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| Error::forbidden())?;
    let payload: Value = serde_json::from_str(&payload).map_err(|_| Error::storage())?;
    let body = payload["body"].as_str().ok_or_else(invalid)?;
    let sender = actor_from_connection(c, &saved.route.credential_id)?;
    let mut draft = MessageDraft::text(
        MessageAudience::Participants(
            saved
                .audience
                .iter()
                .map(|m| m.participant_id.clone())
                .collect(),
        ),
        body,
    );
    draft.msg_id = Some(format!("icmsg_{}", digest(resource.as_bytes())));
    let message = crate::messages::write(c, &sender, &saved.route.conversation_id, draft, store)?;
    let outcome = serde_json::json!({"kind":"message","message_id":message,"conversation_id":saved.route.conversation_id,"remote_identity_verified":false});
    c.execute("INSERT INTO incoming_contact_deliveries(resource_id,hold_id,candidate_id,candidate_json,message_id,outcome_json,actor_identity_id,actor_credential_id,operation_id,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![resource,hold,candidate.view.id,encoded(candidate)?,message,encoded(&outcome)?,actor.identity_id,actor.credential_id,operation,crate::now()]).map_err(|_|conflict())?;
    c.execute("INSERT INTO incoming_contact_audit(hold_id,event_type,actor_identity_id,actor_credential_id,operation_id,created_at) VALUES(?1,'resolved',?2,?3,?4,?5)",params![hold,actor.identity_id,actor.credential_id,operation,crate::now()]).map_err(|_|Error::storage())?;
    Ok(())
}
pub(crate) fn receive(
    store: &Store,
    input: &SmsObservation<'_>,
    routes: Option<&[String]>,
) -> Result<bool> {
    if !bounded(input.provider_id, 256)
        || !bounded(input.event_id, 256)
        || !bounded(input.event_digest, 128)
        || input.body.is_empty()
        || input.body.len() > store.limits().message_bytes
        || crate::sms_eligibility::control_kind(input.body).is_some()
        || input
            .occurred_at
            .is_some_and(|at| at <= 0 || at > crate::now() + 60)
    {
        return Err(invalid());
    }
    store.require_runtime_owner()?;
    let scope = routes
        .map(serde_json::to_string)
        .transpose()
        .map_err(|_| Error::storage())?;
    let held = store.stable_authority(
        &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
        || scopes(&*store.connection()?, input, &scope),
    )?;
    let mut c = store.connection()?;
    let tx = c
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Error::storage())?;
    if !store.authority_covers(&held, &scopes(&tx, input, &scope)?) {
        return Err(Error::forbidden());
    }
    let selection = select(&tx, input, &scope)?;
    let actor = actor_from_connection(&tx, &selection.source.policy.account_credential_id)?;
    let eligible: Vec<_> = selection
        .candidates
        .iter()
        .filter(|candidate| candidate.source["consent"].is_string())
        .cloned()
        .collect();
    let (reason, candidates) = if eligible.is_empty() {
        (
            IncomingContactReason::Ineligible,
            selection.candidates.clone(),
        )
    } else if eligible.len() > 1 {
        (IncomingContactReason::AmbiguousRoute, eligible)
    } else if selection.historical > 1 {
        (IncomingContactReason::HistoricalOverlap, eligible)
    } else {
        (IncomingContactReason::Routed, eligible)
    };
    let payload = serde_json::json!({"kind":"sms","provider_id":input.provider_id,"from":input.from,"to":input.to,"body":input.body,"occurred_at":input.occurred_at});
    let id = capture(
        &tx,
        &Capture {
            source: Source {
                kind: IncomingContactKind::Sms,
                provider: "telnyx",
                account_reference: &selection.source.policy.secret_reference,
                credential_id: &selection.source.policy.account_credential_id,
                binding: selection.source.binding.clone(),
            },
            provider_id: input.provider_id,
            event_id: input.event_id,
            event_digest: input.event_digest,
            payload,
            candidates,
            reason,
        },
        store,
    )?;
    let tenant = tenant(&tx, &actor.identity_id)?;
    let saved = view(&tx, &actor, &tenant, &id)?;
    if saved.state == IncomingContactState::Resolved {
        let raw: String = tx
            .query_row(
                "SELECT candidate_json FROM incoming_contact_deliveries WHERE hold_id=?1",
                [&id],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        checked(
            &tx,
            &serde_json::from_str(&raw).map_err(|_| Error::storage())?,
            false,
        )?;
        tx.commit().map_err(|_| Error::storage())?;
        return Ok(false);
    }
    let mut delivered = false;
    if saved.state == IncomingContactState::Held {
        if saved.reason == IncomingContactReason::Routed {
            let raw: String = tx
                .query_row(
                    "SELECT candidates_json FROM incoming_contact_holds WHERE id=?1",
                    [&id],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            let original: Vec<Candidate> =
                serde_json::from_str(&raw).map_err(|_| Error::storage())?;
            if original.len() != 1 {
                return Err(conflict());
            }
            let operation = format!("icauto_{}", digest(id.as_bytes()));
            deliver(&tx, &id, &original[0], &actor, &operation, store)?;
            record(
                &tx,
                &actor,
                &operation,
                "incoming-contact.route",
                &encoded(&(&id, &original[0].view.id))?,
                &id,
                store,
            )?;
            delivered = true;
        } else if saved.reason == IncomingContactReason::Ineligible {
            let operation = format!("icignore_{}", digest(id.as_bytes()));
            tx.execute("INSERT INTO incoming_contact_rejections VALUES(?1,?2,?3,?4,'No original current enrollment',?5)",params![id,actor.identity_id,actor.credential_id,operation,crate::now()]).map_err(|_|conflict())?;
            tx.execute("INSERT INTO incoming_contact_audit(hold_id,event_type,actor_identity_id,actor_credential_id,operation_id,created_at) VALUES(?1,'rejected',?2,?3,?4,?5)",params![id,actor.identity_id,actor.credential_id,operation,crate::now()]).map_err(|_|Error::storage())?;
            record(
                &tx,
                &actor,
                &operation,
                "incoming-contact.ignore",
                &encoded(&id)?,
                &id,
                store,
            )?;
        }
    }
    if actor.expires_at <= crate::now() {
        return Err(Error::unauthorized());
    }
    tx.commit().map_err(|_| Error::storage())?;
    Ok(delivered)
}
impl Store {
    /// Resolve only an original SMS candidate. Voice uses its own authenticated
    /// observation/owned Session import and cannot be coerced into this path.
    pub fn resolve_incoming_sms_contact(
        &self,
        credential: &str,
        operation: &str,
        id: &str,
        expected_revision: u64,
        candidate_id: &str,
    ) -> Result<IncomingContact> {
        for value in [operation, id, candidate_id] {
            connect_protocol::transitions::id(value)?;
        }
        let derive = |c: &Connection| -> Result<Vec<AuthorityScope>> {
            let (actor, tenant) = admin(c, credential)?;
            let original = visible(c, &actor, &tenant, id)?;
            let chosen = original
                .iter()
                .find(|p| p.view.id == candidate_id)
                .ok_or_else(Error::forbidden)?;
            let saved = checked(c, chosen, true)?;
            let mut scopes = crate::authority_admission::conversation_scopes(
                c,
                &chosen.view.conversation_id,
                AuthorityMode::Write,
            )?;
            scopes.extend(crate::provider_message_endpoints::account(c, &saved.policy)?.1);
            scopes.push(AuthorityScope::identity(
                &actor.identity_id,
                AuthorityMode::Read,
            )?);
            Ok(scopes)
        };
        let held = self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || derive(&*self.connection()?),
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        if !self.authority_covers(&held, &derive(&tx)?) {
            return Err(Error::forbidden());
        }
        let (actor, tenant) = admin(&tx, credential)?;
        let current = view(&tx, &actor, &tenant, id)?;
        if current.kind != IncomingContactKind::Sms
            || current.reason == IncomingContactReason::ResourceConflict
            || current.reason == IncomingContactReason::Ineligible
        {
            return Err(conflict());
        }
        let intent = encoded(&(id, expected_revision, candidate_id))?;
        let intent_digest = digest(intent.as_bytes());
        crate::operations::reject_control_id(&tx, &actor, operation)?;
        let old:Option<(String,String,bool)>=tx.query_row("SELECT digest,command_type,tombstoned FROM operations WHERE tenant_id=?1 AND owner_id=?2 AND operation_id=?3",params![tenant,actor.identity_id,operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
        if let Some((old, kind, tombstoned)) = old {
            if old != intent_digest || kind != "incoming-contact.resolve-sms" {
                return Err(conflict());
            }
            if tombstoned {
                return Err(Error::new("gone", "Incoming decision receipt has expired"));
            }
            return Ok(current);
        }
        if current.state != IncomingContactState::Held || current.revision != expected_revision {
            return Err(conflict());
        }
        let original = visible(&tx, &actor, &tenant, id)?;
        let chosen = original
            .iter()
            .find(|p| p.view.id == candidate_id)
            .ok_or_else(Error::forbidden)?;
        deliver(&tx, id, chosen, &actor, operation, self)?;
        record(
            &tx,
            &actor,
            operation,
            "incoming-contact.resolve-sms",
            &intent,
            id,
            self,
        )?;
        if actor.expires_at <= crate::now() {
            return Err(Error::unauthorized());
        }
        let result = view(&tx, &actor, &tenant, id)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
