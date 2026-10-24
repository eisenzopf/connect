//! A trusted coordinator reserves the private physical context before allocation.
use super::*;
use crate::{AuthorityMode, AuthorityScope, NativeConsultationOrigin};
use connect_protocol::transitions::TransitionIntent;
pub(crate) mod active;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Purpose {
    Media,
    Signal,
    Presence,
}
impl Purpose {
    pub(crate) fn media(self) -> bool {
        self == Self::Media
    }
}

pub(crate) struct Context {
    pub record: String,
    pub audience: Vec<(String, String, u64)>, // Identity, Participant, original epoch
    pub scopes: Vec<AuthorityScope>,
    pub expires_at: i64,
}

fn marker(intent: &OfferIntent) -> Result<Option<&str>> {
    let Some(value) = intent.offer.get("private_consultation") else {
        return Ok(None);
    };
    let fields = value.as_object().ok_or_else(Error::forbidden)?;
    if fields.len() != 2 || value["profile"] != "connect-private-audio-v1" {
        return Err(Error::new(
            "unsupported-media-profile",
            "Unknown private native profile",
        ));
    }
    let record = value["operation_record_id"]
        .as_str()
        .ok_or_else(Error::forbidden)?;
    connect_protocol::transitions::id(record)?;
    Ok(Some(record))
}

fn audio_only(intent: &OfferIntent) -> Result<()> {
    let streams = intent.offer["streams_offered"]
        .as_array()
        .ok_or_else(Error::forbidden)?;
    if streams.len() != 1 || streams[0]["kind"] != "audio" || streams[0]["direction"] != "sendrecv"
    {
        return Err(Error::new(
            "unsupported-media-profile",
            "Private native preparation requires one duplex audio stream",
        ));
    }
    Ok(())
}

/// This performs no allocation and grants no work. The full invitation must
/// later arrive from its own currently authorized signed physical peer.
impl Store {
    pub fn register_native_consultation_leg(
        &self,
        origin: &NativeConsultationOrigin,
        mut intent: OfferIntent,
        endpoint: &str,
    ) -> Result<()> {
        intent.normalize()?;
        audio_only(&intent)?;
        if marker(&intent)? != Some(origin.record_id()) {
            return Err(Error::forbidden());
        }
        let primary = origin.request().connection_id.as_ref().ok_or_else(|| {
            Error::new(
                "unsupported-media-profile",
                "Private preparation requires an explicit primary Connection",
            )
        })?;
        if intent.conversation_id != origin.request().conversation_id
            || intent.session_id != origin.request().session_id
        {
            return Err(Error::forbidden());
        }
        let _held = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let (actor, mut scopes) = origin.require(self, &c)?;
                scopes.push(AuthorityScope::conversation(
                    &intent.conversation_id,
                    AuthorityMode::Write,
                )?);
                scopes.push(AuthorityScope::identity(
                    &actor.identity_id,
                    AuthorityMode::Write,
                )?);
                AuthorityScope::merged(&scopes)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        origin.require(self, &tx)?;
        let TransitionIntent::Consult {
            private_participants,
            ..
        } = &origin.request().intent
        else {
            return Err(Error::forbidden());
        };
        if private_participants.len() != 2 {
            return Err(Error::new(
                "unsupported-media-profile",
                "Native private preparation currently requires two private Participants",
            ));
        }
        let (owner,endpoint_revision,device):(String,u64,String) = tx.query_row("SELECT e.identity_id,e.revision,e.device_id FROM endpoints e JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.revoked=0 WHERE e.id=?1 AND e.transport='quic' AND e.verification_state='verified'",[endpoint],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
        let target = crate::audience::member(&tx, &owner, &intent.conversation_id, true)?;
        let primary_pid: String = tx
            .query_row(
                "SELECT participant_id FROM connections WHERE id=?1",
                [primary],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if target.participant_id == primary_pid
            || !private_participants.contains(&target.participant_id)
            || intent
                .by_participant
                .as_ref()
                .is_some_and(|p| p != &target.participant_id)
        {
            return Err(Error::forbidden());
        }
        let mut audience = Vec::new();
        for pid in private_participants {
            let row: (String,String,u64) = tx.query_row("SELECT identity_id,participant_id,membership_epoch FROM operation_members WHERE operation_record_id=?1 AND participant_id=?2",params![origin.record_id(),pid],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|Error::storage())?;
            audience.push(row);
        }
        audience.sort();
        let offer = crate::provisioning::canonical_json(
            &serde_json::to_value(&intent).map_err(|_| Error::storage())?,
        )?;
        let audience = serde_json::to_string(&audience).map_err(|_| Error::storage())?;
        let old: Option<(String,String,String,String,String,String,String,String)> = tx.query_row("SELECT operation_record_id,credential_id,original_digest,runtime_epoch,endpoint_id,offer_json,audience_json,primary_connection_id FROM native_consultation_legs WHERE connection_id=?1",[&intent.connection_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional().map_err(|_|Error::storage())?;
        let expected = (
            origin.record.clone(),
            origin.credential.clone(),
            origin.digest.clone(),
            origin.runtime.clone(),
            endpoint.to_owned(),
            offer,
            audience,
            primary.clone(),
        );
        if let Some(old) = old {
            context(&tx, &intent, Some(endpoint))?;
            if old != expected {
                return Err(conflict());
            }
        } else {
            let unclaimed:bool=tx.query_row("SELECT state='accepted' AND lease_owner IS NULL AND NOT EXISTS(SELECT 1 FROM operation_effects WHERE operation_record_id=?1) FROM operations WHERE record_id=?1",[origin.record_id()],|r|r.get(0)).map_err(|_|Error::storage())?;
            if !unclaimed {
                return Err(conflict());
            }
            if private_partner(&tx, primary)?.is_some() {
                return Err(conflict());
            }
            let exists: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM connections WHERE id=?1) OR EXISTS(SELECT 1 FROM consultation_candidate_bindings WHERE operation_record_id=?2)",
                    params![intent.connection_id,origin.record_id()],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            if exists {
                return Err(conflict());
            }
            tx.execute(
                "INSERT INTO native_consultation_legs VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![
                    intent.connection_id,
                    expected.0,
                    expected.1,
                    expected.2,
                    expected.3,
                    expected.4,
                    expected.5,
                    expected.6,
                    expected.7,
                    endpoint_revision,
                    device
                ],
            )
            .map_err(|_| Error::storage())?;
        }
        tx.commit().map_err(|_| Error::storage())
    }
}

pub(crate) fn context(
    c: &Connection,
    intent: &OfferIntent,
    endpoint: Option<&str>,
) -> Result<Option<Context>> {
    let stored: Option<(String,String,String,String,String,String,String,String)> = c.query_row("SELECT operation_record_id,credential_id,original_digest,runtime_epoch,endpoint_id,offer_json,audience_json,primary_connection_id FROM native_consultation_legs WHERE connection_id=?1",[&intent.connection_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional().map_err(|_|Error::storage())?;
    let Some((record, credential, digest, runtime, saved_endpoint, offer, audience, _primary)) =
        stored
    else {
        if marker(intent)?.is_some() {
            return Err(Error::new(
                "not-ready",
                "Private native invitation is not registered",
            ));
        }
        return Ok(None);
    };
    if marker(intent)? != Some(record.as_str())
        || endpoint.is_some_and(|e| e != saved_endpoint)
        || serde_json::from_str::<OfferIntent>(&offer).map_err(|_| Error::storage())? != *intent
    {
        return Err(Error::forbidden());
    }
    let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_consultation_legs n JOIN endpoints e ON e.id=n.endpoint_id AND e.revision=n.endpoint_revision AND e.device_id=n.device_id AND e.verification_state='verified' JOIN devices d ON d.id=n.device_id AND d.identity_id=e.identity_id AND d.revoked=0 WHERE n.connection_id=?1)",[&intent.connection_id],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !exact {
        return Err(Error::forbidden());
    }
    // Constructing the current origin needs the caller's Store/runtime below.
    let _ = (credential, digest, runtime);
    Ok(Some(Context {
        record,
        audience: serde_json::from_str(&audience).map_err(|_| Error::storage())?,
        scopes: Vec::new(),
        expires_at: i64::MAX,
    }))
}

pub(crate) fn current(
    store: &Store,
    c: &Connection,
    intent: &OfferIntent,
    endpoint: Option<&str>,
) -> Result<Option<Context>> {
    current_for(store, c, intent, endpoint, Purpose::Media)
}
pub(crate) fn current_for(
    store: &Store,
    c: &Connection,
    intent: &OfferIntent,
    endpoint: Option<&str>,
    purpose: Purpose,
) -> Result<Option<Context>> {
    let Some(original) = context(c, intent, endpoint)? else {
        return Ok(None);
    };
    let current = registered_current(store, c, &original.record, purpose)?;
    if current.audience != original.audience {
        return Err(Error::forbidden());
    }
    Ok(Some(current))
}
/// A public native primary can carry a private floor without claiming its
/// partner's native invitation. Resolve its own registered root and full scopes.
/// A primary in an existing native/native consultation keeps its ordinary
/// physical metadata view; its private candidate supplies the pair context.
pub(crate) fn for_connection(
    store: &Store,
    c: &Connection,
    connection: &str,
    purpose: Purpose,
) -> Result<Option<Context>> {
    let record: Option<String> = c
        .query_row(
            "SELECT f.operation_record_id FROM native_private_content_floors f JOIN consultation_candidate_bindings b ON b.operation_record_id=f.operation_record_id WHERE f.connection_id=?1 AND (b.candidate_kind='sip' OR b.connection_id=f.connection_id)",
            [connection],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    record
        .map(|record| registered_current(store, c, &record, purpose))
        .transpose()
}
pub(crate) fn registered_current(
    store: &Store,
    c: &Connection,
    record: &str,
    purpose: Purpose,
) -> Result<Context> {
    let handed_off:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_handoffs n JOIN operations o ON o.record_id=n.operation_record_id JOIN operation_effects e ON e.id=n.effect_id AND e.state='confirmed' WHERE n.applied_json IS NOT NULL AND json_extract(o.intent_json,'$.payload_extensions.consultation_record_id')=?1)",[record],|r|r.get(0)).map_err(|_|Error::storage())?;
    let restored:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_consultation_returns r JOIN operation_effects e ON e.id=r.effect_id AND e.state='confirmed' WHERE r.consultation_record_id=?1 AND r.applied_json IS NOT NULL)",[record],|r|r.get(0)).map_err(|_|Error::storage())?;
    let unavailable:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM private_provider_contexts p WHERE p.operation_record_id=?1 AND (p.closed_at IS NOT NULL OR ((p.uncertain=1 OR p.export_uncertain=1) AND p.delivered_at IS NULL)))",[record],|r|r.get(0)).map_err(|_|Error::storage())?;
    if handed_off || restored || unavailable {
        return Err(Error::forbidden());
    }
    let (candidate,kind,credential,digest,runtime,audience):(String,String,String,String,String,String)=c.query_row("SELECT connection_id,candidate_kind,credential_id,original_digest,runtime_epoch,audience_json FROM consultation_candidate_bindings WHERE operation_record_id=?1",[record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
    let audience: Vec<(String, String, u64)> =
        serde_json::from_str(&audience).map_err(|_| Error::storage())?;
    if kind == "native" {
        // Generic context lookup must retain the original native invitation's
        // Endpoint/Device binding, including before a physical floor exists.
        let (offer, endpoint): (String, String) = c.query_row(
            "SELECT offer_json,endpoint_id FROM native_consultation_legs WHERE connection_id=?1 AND operation_record_id=?2",
            params![candidate,record], |r|Ok((r.get(0)?,r.get(1)?)),
        ).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
        let offer: OfferIntent = serde_json::from_str(&offer).map_err(|_| Error::storage())?;
        let original = context(c, &offer, Some(&endpoint))?.ok_or_else(Error::forbidden)?;
        if original.record != record || original.audience != audience {
            return Err(Error::forbidden());
        }
    }
    let established = if purpose.media() {
        active::current(store, c, record)?
    } else {
        active::current_for(store, c, record, purpose)?
    };
    if let Some(active) = established {
        return Ok(Context {
            record: record.into(),
            audience,
            scopes: active.scopes,
            expires_at: active.expires_at,
        });
    }
    let origin = NativeConsultationOrigin::from_private_leg(
        store,
        c,
        record.into(),
        credential,
        digest,
        runtime,
    )?;
    let (actor, mut scopes) = origin.require(store, c)?;
    let mut expires_at = actor.expires_at.min(origin.request().expires_at);
    // A private signal needs actual captured application, not preparation or a
    // future route. Before sealing it still needs current original execution.
    let selected:Option<String>=c.query_row("SELECT n.selection_json FROM native_consultation_attempts n JOIN operation_effects e ON e.id=n.effect_id AND e.state='confirmed' WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.started=1 AND n.applied_json IS NOT NULL",params![record,store.runtime_epoch()],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
    if purpose == Purpose::Signal && selected.is_none() {
        return Err(Error::forbidden());
    }
    if let Some(selected) = selected {
        let physical = active::validate_for(
            store,
            c,
            record,
            &selected,
            &serde_json::to_string(&audience).map_err(|_| Error::storage())?,
            purpose,
        )?;
        scopes.extend(physical.scopes);
        expires_at = expires_at.min(physical.expires_at);
    } else if kind == "sip" {
        let original = store
            .private_initial_telephone_origins
            .lock()
            .map_err(|_| Error::storage())?
            .get(&candidate)
            .cloned()
            .ok_or_else(Error::forbidden)?;
        if original.source().parent.record_id() != record {
            return Err(Error::forbidden());
        }
        let (physical, _, expiry) =
            store.initial_telephone_audience_facts_for(c, &original, purpose)?;
        scopes.extend(physical);
        expires_at = expires_at.min(expiry);
    }
    Ok(Context {
        record: record.into(),
        audience,
        scopes: AuthorityScope::merged(&scopes)?,
        expires_at,
    })
}

/// Historical private metadata never becomes visible through a later epoch.
pub(crate) fn visible(c: &Connection, actor: &Actor, connection: &str) -> Result<bool> {
    if !super::handoff_preparation::visible(c, actor, connection)? {
        return Ok(false);
    }
    let audience: Option<String> = c
        .query_row(
            "SELECT audience_json FROM consultation_candidate_bindings WHERE connection_id=?1",
            [connection],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    let Some(audience) = audience else {
        return Ok(true);
    };
    let audience: Vec<(String, String, u64)> =
        serde_json::from_str(&audience).map_err(|_| Error::storage())?;
    for (identity, pid, epoch) in audience {
        if identity != actor.identity_id {
            continue;
        }
        let current: bool=c.query_row("SELECT EXISTS(SELECT 1 FROM participants WHERE participant_id=?1 AND membership_epoch=?2 AND identity_id=?3 AND active=1)",params![pid,epoch,identity],|r|r.get(0)).map_err(|_|Error::storage())?;
        return Ok(current);
    }
    Ok(false)
}

pub(crate) fn observation_allowed(
    c: &Connection,
    connection: &str,
    selected: &[(String, u64)],
) -> Result<bool> {
    if !super::handoff_preparation::observation_allowed(c, connection, selected)? {
        return Ok(false);
    }
    let audience: Option<String> = c
        .query_row(
            "SELECT audience_json FROM native_private_content_floors WHERE connection_id=?1",
            [connection],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?;
    let Some(audience) = audience else {
        return Ok(true);
    };
    let audience: Vec<(String, String, u64)> =
        serde_json::from_str(&audience).map_err(|_| Error::storage())?;
    Ok(selected
        .iter()
        .all(|(pid, epoch)| audience.iter().any(|(_, p, e)| p == pid && e == epoch)))
}

pub(crate) fn observation_source_current(
    store: &Store,
    c: &Connection,
    connection: &str,
) -> Result<bool> {
    match for_connection(store, c, connection, Purpose::Presence) {
        Ok(Some(_)) | Ok(None) => Ok(true),
        Err(e)
            if matches!(
                e.code.as_str(),
                "unauthorized" | "forbidden" | "conflict" | "gone" | "not-ready"
            ) =>
        {
            Ok(false)
        }
        Err(e) => Err(e),
    }
}

pub(crate) fn private_partner(c: &Connection, connection: &str) -> Result<Option<String>> {
    c.query_row(
        "SELECT partner_connection_id FROM native_private_content_floors WHERE connection_id=?1",
        [connection],
        |r| r.get(0),
    )
    .optional()
    .map_err(|_| Error::storage())
}

pub(crate) fn require_connection_context(c: &Connection, session: &str) -> Result<()> {
    let private:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM consultation_candidate_bindings l JOIN connections v ON v.id=l.connection_id WHERE v.session_id=?1)",[session],|r|r.get(0)).map_err(|_|Error::storage())?;
    if private {
        return Err(Error::new(
            "private-context-required",
            "Private native media requires its Connection-bound source and context",
        ));
    }
    Ok(())
}

impl Store {
    /// Match only the requested current operation's registered native candidate.
    /// An unrelated private context never lends or revalidates its execution
    /// credential. This is a planning fact, not a physical media grant.
    pub fn native_consultation_candidate_matches(
        &self,
        parent: &NativeConsultationOrigin,
        candidate: &crate::NativeMediaOrigin,
    ) -> Result<bool> {
        candidate.require_runtime(self)?;
        let c = self.try_connection()?;
        parent.require(self, &c)?;
        let registered: bool = c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM native_consultation_legs
             WHERE connection_id=?1 AND operation_record_id=?2 AND credential_id=?3
               AND original_digest=?4 AND runtime_epoch=?5)",
                params![
                    candidate.connection_id(),
                    parent.record,
                    parent.credential,
                    parent.digest,
                    parent.runtime
                ],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if !registered {
            return Ok(false);
        }
        let valid:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM connections v JOIN native_connection_lifetimes l ON l.connection_id=v.id AND l.generation=v.generation AND l.runtime_epoch=v.observed_runtime_id AND l.retired=0 JOIN connection_offers m ON m.connection_id=v.id WHERE v.id=?1 AND v.generation=?2 AND m.runtime_connection_id=?3 AND v.observed_runtime_id=?4)",params![candidate.connection_id(),candidate.generation(),candidate.runtime_connection_id(),self.runtime_epoch()],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !valid {
            return Err(Error::forbidden());
        }
        Ok(true)
    }
}
