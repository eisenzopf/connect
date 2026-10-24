//! Current physical metadata for the original private SIP call. This deliberately
//! does not inspect controller execution authority or issue a media/signal read:
//! pending selection and established private lifetimes validate their own rights.
use super::*;

impl Store {
    /// Only a fresh return over this exact captured private candidate can grant
    /// its closure control view. It grants no public/private media receive edge.
    pub(crate) fn initial_telephone_return_in(
        &self,
        c: &Connection,
        origin: Arc<InitialTelephoneMediaOrigin>,
        returning: &crate::NativeConsultationReturnOrigin,
        interval: Arc<AuthorityGuardSet>,
        until: i64,
    ) -> Result<InitialTelephoneMediaRead> {
        let actor = returning.require(self, c)?;
        let (mut required, binding, expiry) = facts(self, c, &origin)?;
        let context: String = c.query_row(
            "SELECT r.context_selection_json FROM native_consultation_returns r
             JOIN consultation_candidate_bindings b ON b.operation_record_id=r.consultation_record_id
             WHERE r.operation_record_id=?1 AND b.candidate_kind='sip' AND b.connection_id=?2",
            params![returning.record_id(),origin.connection_id()], |r|r.get(0),
        ).optional().map_err(|_| Error::storage())?.ok_or_else(Error::forbidden)?;
        let context: Value = serde_json::from_str(&context).map_err(|_| Error::storage())?;
        if origin.source().parent.record_id() != returning.consultation_record_id()
            || origin.source().conversation_id != returning.request().conversation_id
            || origin.source().session_id != returning.request().session_id
            || !context["private"]
                .as_array()
                .is_some_and(|p| p.contains(&binding))
        {
            return Err(Error::forbidden());
        }
        required.push(AuthorityScope::identity(
            &actor.identity_id,
            AuthorityMode::Read,
        )?);
        if !self.authority_covers(&interval, &required) {
            return Err(Error::forbidden());
        }
        self.initial_telephone_read_with_purpose(
            c,
            origin,
            interval,
            until
                .min(expiry)
                .min(actor.expires_at)
                .min(returning.request().expires_at),
            ProjectionPurpose::PrivateReturn,
        )
    }
    /// Only the complete consultation planner can supply this owned interval.
    /// The root is revalidated here; physical metadata alone cannot issue it.
    pub(crate) fn initial_telephone_consultation_in(
        &self,
        c: &Connection,
        origin: Arc<InitialTelephoneMediaOrigin>,
        root: &crate::NativeConsultationOrigin,
        interval: Arc<AuthorityGuardSet>,
        until: i64,
    ) -> Result<InitialTelephoneMediaRead> {
        if !origin.source().is_private_candidate()
            || origin.source().parent.record_id() != root.record_id()
            || origin.source().conversation_id != root.request().conversation_id
            || origin.source().session_id != root.request().session_id
        {
            return Err(Error::forbidden());
        }
        let (actor, mut required) = root.require(self, c)?;
        required.extend(facts(self, c, &origin)?.0);
        if !self.authority_covers(&interval, &required) {
            return Err(Error::forbidden());
        }
        self.initial_telephone_read_with_purpose(
            c,
            origin,
            interval,
            until.min(actor.expires_at).min(root.request().expires_at),
            ProjectionPurpose::PrivateSelection,
        )
    }
}

pub(super) fn selection_projection(
    store: &Store,
    c: &Connection,
    origin: &InitialTelephoneMediaOrigin,
    until: i64,
) -> Result<InitialTelephoneProjection> {
    let (_, _, physical_expiry) = facts(store, c, origin)?;
    projection(c, origin, until.min(physical_expiry))
}

/// Signal context is distinct from admission and media selection. It requires
/// captured application or a sealed lifetime, never mere private readiness.
pub(super) fn signal_authority(
    store: &Store,
    c: &Connection,
    origin: &InitialTelephoneMediaOrigin,
) -> Result<(Vec<AuthorityScope>, i64)> {
    use crate::connection_control::private_consultation::{registered_current, Purpose};
    let (mut scopes, _, expires) = facts_for(store, c, origin, Purpose::Signal)?;
    let context = registered_current(
        store,
        c,
        origin.source().parent.record_id(),
        Purpose::Signal,
    )?;
    scopes.extend(context.scopes);
    Ok((
        AuthorityScope::merged(&scopes)?,
        expires.min(context.expires_at),
    ))
}

pub(super) fn signal_projection(
    store: &Store,
    c: &Connection,
    origin: &InitialTelephoneMediaOrigin,
    until: i64,
) -> Result<InitialTelephoneProjection> {
    let (_, expires) = signal_authority(store, c, origin)?;
    projection(c, origin, until.min(expires))
}

fn projection(
    c: &Connection,
    origin: &InitialTelephoneMediaOrigin,
    expires: i64,
) -> Result<InitialTelephoneProjection> {
    let original = origin.signal.origin();
    let s = origin.source();
    let (role, revision): (String, u64) = c.query_row(
        "SELECT p.role,v.revision FROM session_participants p JOIN conversations v ON v.id=p.conversation_id WHERE p.session_id=?1 AND p.participant_id=?2 AND p.membership_epoch=?3 AND p.state='present' AND v.closed=0",
        params![s.session_id,s.target.participant_id,s.target.membership_epoch],
        |r| Ok((r.get(0)?,r.get(1)?)),
    ).map_err(|_| Error::forbidden())?;
    if expires <= crate::now() {
        return Err(Error::forbidden());
    }
    Ok(InitialTelephoneProjection {
        endpoint_actor: actor_from_connection(c, &original.policy.endpoint_credential_id)?,
        route_actor: actor_from_connection(c, &original.policy.route_credential_id)?,
        member: s.target.clone(),
        session: crate::session_control::view(c, &s.conversation_id, &s.session_id)?,
        role,
        accepted: json!({"audio":[origin.codec]}),
        conversation_revision: revision,
        expires,
        connected: true,
    })
}

pub(super) fn facts(
    store: &Store,
    c: &Connection,
    origin: &InitialTelephoneMediaOrigin,
) -> Result<(Vec<AuthorityScope>, Value, i64)> {
    facts_for(
        store,
        c,
        origin,
        crate::connection_control::private_consultation::Purpose::Media,
    )
}
pub(super) fn facts_for(
    store: &Store,
    c: &Connection,
    origin: &InitialTelephoneMediaOrigin,
    purpose: crate::connection_control::private_consultation::Purpose,
) -> Result<(Vec<AuthorityScope>, Value, i64)> {
    historical_lifetime(store, c, &origin.signal)?;
    let original = origin.signal.origin();
    let s = origin.source();
    let InitialTelephoneParent::Consultation {
        record_id,
        connection_id,
    } = &s.parent
    else {
        return Err(Error::forbidden());
    };
    if connection_id != origin.connection_id() {
        return Err(Error::forbidden());
    }
    let endpoint = actor_from_connection(c, &original.policy.endpoint_credential_id)?;
    let route = actor_from_connection(c, &original.policy.route_credential_id)?;
    if purpose.media() {
        endpoint.require("connection.offer")?;
    }
    route.require("sip_dial")?;
    if endpoint.identity_id != s.target.identity_id || route.identity_id != s.route_owner_id {
        return Err(Error::forbidden());
    }
    let valid: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM initial_telephone_operations n
         JOIN operations child ON child.record_id=n.operation_record_id AND child.state='completed' AND child.cancel_requested=0
         JOIN operations root ON root.record_id=n.parent_record_id AND root.command_type='session.consult'
         JOIN sip_consultation_reservations reserved ON reserved.operation_record_id=root.record_id AND reserved.connection_id=child.connection_id
         JOIN initial_telephone_answers a ON a.operation_record_id=n.operation_record_id
         JOIN initial_telephone_private_acceptances own ON own.operation_record_id=n.operation_record_id
         JOIN operation_effects effect ON effect.id=n.effect_id AND effect.operation_record_id=child.record_id AND effect.state='confirmed'
         JOIN connections v ON v.id=child.connection_id AND v.session_id=child.session_id AND v.participant_id=own.participant_id AND v.membership_epoch=own.membership_epoch
         JOIN endpoints e ON e.id=v.endpoint_id AND e.identity_id=v.identity_id AND e.revision=own.endpoint_revision
         JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.revoked=0
         WHERE n.operation_record_id=?1 AND n.parent_kind='consultation' AND n.parent_record_id=?2
         AND child.connection_id=?3 AND child.conversation_id=?4 AND child.session_id=?5
         AND root.conversation_id=child.conversation_id AND root.session_id=child.session_id
         AND reserved.runtime_epoch=?6 AND reserved.original_digest=root.digest AND reserved.primary_connection_id=root.connection_id
         AND reserved.endpoint_id=e.id AND reserved.endpoint_revision=e.revision AND reserved.device_id=d.id
         AND reserved.participant_id=own.participant_id AND reserved.membership_epoch=own.membership_epoch
         AND n.attempt_runtime=?6 AND n.runtime_connection_id=?7 AND n.sip_call_id=?8 AND n.signaling_started=1
         AND v.generation=1 AND v.observed_runtime_id=?6 AND v.state IN ('ready','active') AND v.readiness_json IS NOT NULL
         AND e.id=?9 AND e.revision=?10 AND e.device_id=?11 AND e.verification_state='verified'
         AND e.secret_ref=?12 AND e.route_owner_id=?13 AND e.transport IN ('sip','telephone')
         AND own.consultation_record_id=?2 AND own.endpoint_credential_id=?14
         AND own.participant_id=?15 AND own.membership_epoch=?16 AND own.codec=?17 AND own.audience_json=?18
         AND a.observation_json=?19 AND a.codec=own.codec AND a.accepted=1
         AND a.acceptance_kind='private-connection' AND a.acceptance_record_id=n.operation_record_id
         AND a.connected_at IS NOT NULL AND a.retired_at IS NULL
         AND json_extract(a.connected_json,'$.private_audio')=1
         AND json_extract(a.connected_json,'$.private_consultation_record_id')=?2
         AND json_extract(a.connected_json,'$.connection_id')=?3
         AND json_extract(a.connected_json,'$.runtime_connection_id')=?7
         AND json_extract(a.connected_json,'$.call_id')=?8
         AND json_extract(v.readiness_json,'$.private_audio')=1
         AND json_extract(v.readiness_json,'$.private_consultation_record_id')=?2
         AND json_extract(v.readiness_json,'$.connection_id')=?3
         AND json_extract(v.readiness_json,'$.runtime_connection_id')=?7
         AND json_extract(v.readiness_json,'$.call_id')=?8
         AND json_extract(v.readiness_json,'$.generation')=1
         AND json_extract(v.readiness_json,'$.participant_id')=?15
         AND json_extract(v.readiness_json,'$.membership_epoch')=?16)",
        params![origin.record_id(), record_id, connection_id, s.conversation_id, s.session_id,
            store.runtime_epoch(), origin.runtime_connection_id(), origin.sip_call_id(), s.endpoint_id,
            s.endpoint_revision, s.device_id, original.policy.route_reference, s.route_owner_id,
            endpoint.credential_id, s.target.participant_id, s.target.membership_epoch, origin.codec,
            serde_json::to_string(&s.audience).map_err(|_| Error::storage())?, origin.observation],
        |r| r.get(0),
    ).map_err(|_| Error::storage())?;
    if !valid {
        return Err(Error::forbidden());
    }
    let member = crate::audience::member(c, &endpoint.identity_id, &s.conversation_id, true)?;
    if member.participant_id != s.target.participant_id
        || member.membership_epoch != s.target.membership_epoch
    {
        return Err(Error::forbidden());
    }
    crate::audience::require_original(
        c,
        &endpoint,
        &s.conversation_id,
        connect_protocol::membership::ContentKind::Session,
        &s.session_id,
    )?;
    let present: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM session_participants p JOIN sessions s ON s.id=p.session_id
         JOIN conversations v ON v.id=p.conversation_id AND v.closed=0
         WHERE p.session_id=?1 AND p.participant_id=?2 AND p.membership_epoch=?3 AND p.state='present'
         AND s.state IN ('accepted','ready','active'))",
        params![s.session_id, s.target.participant_id, s.target.membership_epoch], |r| r.get(0),
    ).map_err(|_| Error::storage())?;
    if !present {
        return Err(Error::forbidden());
    }
    let physical_expires = endpoint.expires_at.min(route.expires_at);
    let binding = json!({
        "source_kind":"private-initial-sip", "consultation_record_id":record_id,
        "record_id":origin.record_id(), "connection_id":connection_id,
        "runtime_connection_id":origin.runtime_connection_id(), "sip_call_id":origin.sip_call_id(),
        "generation":1, "participant_id":member.participant_id, "membership_epoch":member.membership_epoch,
        "identity_id":member.identity_id, "endpoint_id":s.endpoint_id, "endpoint_revision":s.endpoint_revision,
        "device_id":s.device_id, "endpoint_credential_id":endpoint.credential_id,
        "route_credential_id":route.credential_id, "route_owner_id":s.route_owner_id,
        "physical_expires_at":physical_expires,
    });
    Ok((
        current_scopes(c, original, false)?,
        binding,
        physical_expires.min(crate::now() + 5),
    ))
}

pub(super) fn remember(
    store: &Store,
    c: &Connection,
    origin: Arc<InitialTelephoneMediaOrigin>,
) -> Result<()> {
    facts(store, c, &origin)?;
    let mut origins = store
        .private_initial_telephone_origins
        .lock()
        .map_err(|_| Error::storage())?;
    if !origins.contains_key(origin.connection_id()) && origins.len() >= store.limits.connections {
        let mut q = c.prepare("SELECT id FROM connections WHERE observed_runtime_id=?1 AND generation=1 AND state IN ('ready','active')").map_err(|_| Error::storage())?;
        let live = q
            .query_map([store.runtime_epoch()], |r| r.get::<_, String>(0))
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<std::collections::BTreeSet<_>>>()
            .map_err(|_| Error::storage())?;
        origins.retain(|id, _| live.contains(id));
        if origins.len() >= store.limits.connections {
            return Err(Error::new(
                "capacity-exceeded",
                "Private SIP metadata origin capacity exhausted",
            ));
        }
    }
    origins.insert(origin.connection_id().into(), origin);
    Ok(())
}
