//! Source planning only: no selector resolution, route mutation or media grant.
use super::*;
use connect_protocol::{connection_control::OfferIntent, native_streams::Intent};

/// Backend facts under this exact command's retained original source readers.
/// An existing native route and its real physical owner must still be checked
/// by the installed service/engine before any control effect is attempted.
pub struct NativeStreamProjection {
    pub parent_record_id: String,
    pub parent_operation_id: String,
    pub stream_revision: u64,
    pub connection: PeerConnectionProjection,
}
pub(super) fn projection(
    store: &Store,
    c: &Connection,
    actor: &crate::Actor,
    device: &str,
    key: &str,
    intent: &Intent,
) -> Result<(NativeStreamProjection, Vec<String>)> {
    intent.validate()?;
    actor.require(intent.command.required_scope())?;
    let signal = intent.command == connect_protocol::native_streams::Command::Dtmf;
    if !signal {
        actor.require("connection.offer")?;
    }
    let member = crate::audience::member(c, &actor.identity_id, &intent.conversation_id, true)?;
    if intent
        .by_participant
        .as_ref()
        .is_some_and(|pid| pid != &member.participant_id)
    {
        return Err(Error::forbidden());
    }
    struct Parent {
        record: String,
        operation: String,
        offer: String,
        expected: Option<u64>,
        endpoint: String,
        resources: String,
        revision: u64,
    }
    let row:Option<Parent>=c.query_row("SELECT o.record_id,o.operation_id,m.offer_json,o.expected_revision,n.endpoint_id,n.resources_json,json_extract(v.readiness_json,'$.stream_revision') FROM native_admission_sources n JOIN native_admission_sources o ON o.record_id=n.operation_record_id JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=o.record_id JOIN connections v ON v.id=o.connection_id JOIN connection_offers m ON m.connection_id=v.id AND m.operation_id=o.operation_id JOIN native_connection_lifetimes life ON life.operation_record_id=o.record_id AND life.connection_id=v.id WHERE o.owner_id=?1 AND o.conversation_id=?2 AND o.session_id=?3 AND o.connection_id=?4 AND o.state='completed' AND n.effect_started=1 AND e.state='confirmed' AND n.credential_id=?5 AND n.key_id=?6 AND n.device_id=?7 AND n.participant_id=?8 AND n.membership_epoch=?9 AND n.attempt_runtime=?10 AND n.source_expires_at>?11 AND v.generation=n.generation AND v.state IN ('ready','active') AND v.observed_runtime_id=?10 AND life.runtime_epoch=?10 AND life.generation=v.generation AND life.retired=0",params![actor.identity_id,intent.conversation_id,intent.session_id,intent.connection_id,actor.credential_id,key,device,member.participant_id,member.membership_epoch,store.runtime_epoch(),crate::now()],|r|Ok(Parent{record:r.get(0)?,operation:r.get(1)?,offer:r.get(2)?,expected:r.get(3)?,endpoint:r.get(4)?,resources:r.get(5)?,revision:r.get(6)?})).optional().map_err(|_|Error::storage())?;
    let row = row.ok_or_else(Error::forbidden)?;
    let payload: serde_json::Value =
        serde_json::from_str(&row.offer).map_err(|_| Error::storage())?;
    let mut offer = OfferIntent {
        operation_id: row.operation.clone(),
        conversation_id: intent.conversation_id.clone(),
        session_id: intent.session_id.clone(),
        connection_id: intent.connection_id.clone(),
        expected_revision: row.expected,
        by_participant: payload["by_participant"].as_str().map(str::to_owned),
        offer: payload,
    };
    offer.normalize()?;
    let resources: Vec<String> =
        serde_json::from_str(&row.resources).map_err(|_| Error::storage())?;
    if row.revision == 0
        || row.revision > 9_007_199_254_740_991
        || (!signal && resources.iter().any(|id| !actor.permits_recipient(id)))
    {
        return Err(Error::forbidden());
    }
    let connection = super::connection_projection::current_projection(
        c,
        actor,
        &offer,
        &row.endpoint,
        device,
        store.runtime_epoch(),
        false,
    )?;
    if connection.connection.connection_id != intent.connection_id
        || connection.connection.session_id != intent.session_id
        || connection.connection.conversation_id != intent.conversation_id
    {
        return Err(Error::forbidden());
    }
    Ok((
        NativeStreamProjection {
            parent_record_id: row.record,
            parent_operation_id: row.operation,
            stream_revision: row.revision,
            connection,
        },
        resources,
    ))
}
pub(super) fn scopes(
    store: &Store,
    c: &Connection,
    actor: &crate::Actor,
    device: &str,
    key: &str,
    intent: &Intent,
) -> Result<Vec<AuthorityScope>> {
    if intent.command == connect_protocol::native_streams::Command::Reconcile {
        return Err(Error::forbidden());
    }
    if let Some(mut retained) = crate::transitions::retained_native_stream_scopes(c, actor, intent)?
    {
        if intent.command == connect_protocol::native_streams::Command::Dtmf {
            retained.extend(dtmf_scene_scopes(store, c, intent)?);
        }
        if intent.quality_interval_ms().is_none()
            && matches!(
                intent.command,
                connect_protocol::native_streams::Command::Subscribe
                    | connect_protocol::native_streams::Command::Unsubscribe
                    | connect_protocol::native_streams::Command::Update
            )
        {
            retained.extend(store.telephone_scene_scopes(
                c,
                &intent.conversation_id,
                &intent.session_id,
            )?);
        }
        return AuthorityScope::merged(&retained);
    }
    let (source, resources) = projection(store, c, actor, device, key, intent)?;
    let data_recipients = if intent.command == connect_protocol::native_streams::Command::Data {
        super::native_data::recipients(c, actor, intent, &source)?
            .into_iter()
            .map(|p| p.identity_id)
            .collect()
    } else {
        Vec::new()
    };
    let mut scopes = crate::authority_admission::conversation_scopes(
        c,
        &intent.conversation_id,
        AuthorityMode::Read,
    )?;
    if intent.command == connect_protocol::native_streams::Command::Dtmf {
        scopes.extend(dtmf_scene_scopes(store, c, intent)?);
    }
    scopes.push(AuthorityScope::identity(
        &actor.identity_id,
        AuthorityMode::Read,
    )?);
    for recipient in resources.into_iter().chain(data_recipients) {
        scopes.push(AuthorityScope::identity(&recipient, AuthorityMode::Read)?);
    }
    if intent.quality_interval_ms().is_none()
        && matches!(
            intent.command,
            connect_protocol::native_streams::Command::Subscribe
                | connect_protocol::native_streams::Command::Unsubscribe
                | connect_protocol::native_streams::Command::Update
        )
    {
        scopes.extend(store.telephone_scene_scopes(
            c,
            &intent.conversation_id,
            &intent.session_id,
        )?);
    }
    AuthorityScope::merged(&scopes)
}
fn dtmf_scene_scopes(
    store: &Store,
    c: &Connection,
    intent: &Intent,
) -> Result<Vec<AuthorityScope>> {
    let mut q=c.prepare("SELECT DISTINCT p.identity_id FROM participants p JOIN session_participants sp ON sp.participant_id=p.participant_id AND sp.membership_epoch=p.membership_epoch WHERE p.conversation_id=?1 AND p.active=1 AND sp.session_id=?2 AND sp.state='present' ORDER BY p.identity_id LIMIT 257").map_err(|_|Error::storage())?;
    let ids = q
        .query_map(params![intent.conversation_id, intent.session_id], |r| {
            r.get::<_, String>(0)
        })
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    if ids.len() > 256 {
        return Err(Error::new(
            "capacity-exceeded",
            "DTMF scene exceeds its bounded profile",
        ));
    }
    let mut scopes = ids
        .iter()
        .map(|id| AuthorityScope::identity(id, AuthorityMode::Read))
        .collect::<Result<Vec<_>>>()?;
    scopes.extend(store.telephone_scene_scopes(c, &intent.conversation_id, &intent.session_id)?);
    AuthorityScope::merged(&scopes)
}
impl Store {
    /// Early subscription diagnostics under the original command's complete
    /// readers. Receiver kinds are engine planning metadata, never an IO grant.
    /// Current protected media policy still resolves every actual source.
    pub fn validate_native_subscription_sources_with_peer(
        &self,
        credential: &str,
        proof: &PeerAuthorityGuard,
        receiver_kinds: &[String],
    ) -> Result<()> {
        let PeerCommand::NativeStream(wire) = &proof.request.command else {
            return Err(Error::forbidden());
        };
        if wire.command != connect_protocol::native_streams::Command::Subscribe {
            return Err(Error::forbidden());
        }
        let c = self.connection()?;
        self.require_peer_source(&c, proof, credential)?;
        let actor = crate::identity::actor_from_connection(&c, credential)?;
        let target = self.native_stream_projection_at(&c, &actor, proof)?;
        // Private compositions keep their established protected source policy.
        // This is an early ordinary-Session diagnostic, not a replacement grant.
        if target.connection.private_consultation.is_some() || target.connection.handoff_preparation
        {
            return Ok(());
        }
        for selector in wire.payload["subscriptions"]
            .as_array()
            .ok_or_else(Error::forbidden)?
        {
            let Some(participant) = selector["from_participant"].as_str() else {
                continue;
            };
            let source: Option<(String, Option<String>, String, Option<String>)> = c.query_row(
                "SELECT p.identity_id,pc.capabilities_json,s.media,sc.capabilities_json FROM session_participants sp JOIN participants p ON p.participant_id=sp.participant_id AND p.conversation_id=sp.conversation_id AND p.membership_epoch=sp.membership_epoch AND p.active=1 JOIN sessions s ON s.id=sp.session_id AND s.conversation_id=sp.conversation_id LEFT JOIN session_presence_capabilities pc ON pc.session_id=sp.session_id AND pc.participant_id=sp.participant_id AND pc.membership_epoch=sp.membership_epoch LEFT JOIN session_control sc ON sc.session_id=s.id AND sc.conversation_id=s.conversation_id WHERE sp.conversation_id=?1 AND sp.session_id=?2 AND sp.participant_id=?3 AND sp.state='present' AND s.state IN ('accepted','active')",
                params![wire.conversation_id, wire.session_id, participant], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
            ).optional().map_err(|_| Error::storage())?;
            let (identity, answer, media, offer) = source
                .ok_or_else(|| Error::new("not-found", "Subscription source is unavailable"))?;
            if !actor.permits_recipient(&identity)
                || !self.authority_covers(
                    &proof._scopes,
                    &[AuthorityScope::identity(&identity, AuthorityMode::Read)?],
                )
            {
                return Err(Error::forbidden());
            }
            let (Some(answer), Some(offer)) = (answer, offer) else {
                continue;
            };
            let answer: serde_json::Value =
                serde_json::from_str(&answer).map_err(|_| Error::storage())?;
            let offer: serde_json::Value =
                serde_json::from_str(&offer).map_err(|_| Error::storage())?;
            let media: Vec<String> = serde_json::from_str(&media).map_err(|_| Error::storage())?;
            let allows = |kind: &String| {
                media.contains(kind)
                    && crate::session_control::supports(&offer, &answer, std::slice::from_ref(kind))
            };
            let compatible = match selector["kinds"].as_array() {
                Some(kinds) => kinds
                    .iter()
                    .all(|k| k.as_str().is_some_and(|k| allows(&k.to_owned()))),
                None => receiver_kinds.iter().any(allows),
            };
            if !compatible {
                return Err(Error::new(
                    "incompatible-capabilities",
                    "Subscription capabilities have no usable intersection",
                ));
            }
        }
        self.require_peer_source(&c, proof, credential)?;
        Ok(())
    }
    pub fn project_native_stream_with_peer(
        &self,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<NativeStreamProjection> {
        let PeerCommand::NativeStream(intent) = &proof.request.command else {
            return Err(Error::forbidden());
        };
        let c = self.connection()?;
        self.require_peer_source(&c, proof, credential)?;
        let actor = crate::identity::actor_from_connection(&c, credential)?;
        let (view, _) = projection(
            self,
            &c,
            &actor,
            &proof.request.device.id,
            &proof.request.key_id,
            intent,
        )?;
        let sources = scopes(
            self,
            &c,
            &actor,
            &proof.request.device.id,
            &proof.request.key_id,
            intent,
        )?;
        if !self.authority_covers(&proof._scopes, &sources) {
            return Err(Error::forbidden());
        }
        self.require_peer_source(&c, proof, credential)?;
        Ok(view)
    }
}

impl Store {
    pub(crate) fn native_stream_projection_at(
        &self,
        c: &Connection,
        actor: &crate::Actor,
        proof: &PeerAuthorityGuard,
    ) -> Result<NativeStreamProjection> {
        let PeerCommand::NativeStream(intent) = &proof.request.command else {
            return Err(Error::forbidden());
        };
        let (view, _) = projection(
            self,
            c,
            actor,
            &proof.request.device.id,
            &proof.request.key_id,
            intent,
        )?;
        let scopes = scopes(
            self,
            c,
            actor,
            &proof.request.device.id,
            &proof.request.key_id,
            intent,
        )?;
        if !self.authority_covers(&proof._scopes, &scopes) {
            return Err(Error::forbidden());
        }
        Ok(view)
    }
}
