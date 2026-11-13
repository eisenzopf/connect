//! Fresh, bounded attribution for one replacement start across SDP delivery.
//! This contains no authority interval and cannot allocate or repeat the effect.
use super::*;

pub struct NativeReplacementStartOrigin {
    allocated: Arc<NativeReplacementAllocated>,
    request: crate::PeerAuthorityRequest,
    attempt: String,
    effect: String,
    target: String,
    expires_at: i64,
}
redacted!(NativeReplacementStartOrigin);
impl NativeReplacementStartOrigin {
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
    pub fn operation_record_id(&self) -> &str {
        &self.allocated.attempt.prepared.record
    }
}
fn facts(
    store: &Store,
    c: &Connection,
    origin: &NativeReplacementStartOrigin,
) -> Result<(
    connect_protocol::membership::MemberView,
    serde_json::Value,
    Vec<AuthorityScope>,
    i64,
)> {
    let prepared = &origin.allocated.attempt.prepared;
    let saved = allocated_current(store, c, &origin.allocated)?;
    let exact: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id WHERE s.operation_record_id=?1 AND s.phase='start' AND s.attempt_id=?2 AND s.effect_id=?3 AND s.started=1 AND e.observation_json=?4 AND e.state IN ('submitting','confirmed'))",
        params![prepared.record,origin.attempt,origin.effect,origin.target],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !exact || crate::now() >= origin.expires_at {
        return Err(Error::forbidden());
    }
    let (meta, key, mut scopes, expiry) =
        crate::peer_authority::checked(store, c, &origin.request)?;
    let descriptor: serde_json::Value =
        serde_json::from_str(&origin.target).map_err(|_| Error::storage())?;
    if descriptor["credential_id"] != meta.actor.credential_id
        || descriptor["key_id"] != origin.request.key_id
        || descriptor["public_key"] != serde_json::json!(key.public_key)
        || descriptor["device_id"] != key.device_id
        || !matches!(
            descriptor["intent"]["offer"]["substrate"].as_str(),
            Some("quic" | "webtransport" | "websocket+webrtc")
        )
    {
        return Err(Error::forbidden());
    }
    let context = prepared.selection.context(store, c)?;
    let member =
        crate::audience::member(c, &meta.actor.identity_id, &context.conversation_id, true)?;
    if member.participant_id != context.participant_id
        || member.membership_epoch != context.membership_epoch
    {
        return Err(Error::forbidden());
    }
    scopes.extend(prepared.selection.scopes(store, c, false)?);
    Ok((
        member,
        saved,
        AuthorityScope::merged(&scopes)?,
        expiry.min(origin.expires_at),
    ))
}
// Construct and validate exact immutable provenance in the SAME once-only
// begin transaction. This contains no authority interval and commits atomically
// with the original attempt; callers never poll primary SQL while holding Write
// merely to recover its attribution.
pub(super) fn start_origin(
    store: &Store,
    c: &Connection,
    attempt: &NativeReplacementStartAttempt,
    read: &NativeReplacementStartRead,
) -> Result<Arc<NativeReplacementStartOrigin>> {
    if !read.current()
        || !matches!(
            read.target.endpoint_transport,
            connect_protocol::provisioning::EndpointTransport::Quic
                | connect_protocol::provisioning::EndpointTransport::Webtransport
                | connect_protocol::provisioning::EndpointTransport::Webrtc
        )
        || read.source.selection().origin.record != attempt.allocated.attempt.prepared.record
        || crate::provisioning::canonical_json(&read.descriptor)? != attempt.target
    {
        return Err(Error::forbidden());
    }
    let origin = Arc::new(NativeReplacementStartOrigin {
        allocated: attempt.allocated.clone(),
        request: attempt.request.clone(),
        attempt: attempt.attempt.clone(),
        effect: attempt.effect.clone(),
        target: attempt.target.clone(),
        expires_at: read.expires_at(),
    });
    facts(store, c, &origin)?;
    Ok(origin)
}

impl Store {
    pub fn try_native_replacement_start_read(
        &self,
        origin: &NativeReplacementStartOrigin,
    ) -> Result<NativeReplacementStartRead> {
        let descriptor: serde_json::Value =
            serde_json::from_str(&origin.target).map_err(|_| Error::storage())?;
        self.try_current_packet_projection(
            |c| {
                let (member, saved, scopes, expires) = facts(self, c, origin)?;
                Ok(((member, saved), scopes, expires))
            },
            |c, (member, saved), held, expires| {
                let source = origin
                    .allocated
                    .attempt
                    .prepared
                    .selection
                    .read_with_connection(self, c, held.clone(), expires)?;
                let target = projection(&source, member.clone(), saved, &descriptor)?;
                Ok(NativeReplacementStartRead {
                    source,
                    target,
                    descriptor: descriptor.clone(),
                })
            },
        )
    }
    /// Re-admit only the original signed intent. This neither begins a new
    /// effect nor extends the original target lifetime after its handler exits.
    pub fn resume_native_replacement_start_authority(
        &self,
        origin: &NativeReplacementStartOrigin,
        wait: &crate::AuthorityWait,
    ) -> Result<PeerAuthorityGuard> {
        let (proof, _read) = self.resume_native_replacement_start_with_read(origin, wait)?;
        Ok(proof)
    }
    // One closed fresh admission derives the SAME proof and its current target
    // projection together. No caller-supplied facts or returned reusable token.
    fn resume_native_replacement_start_with_read(
        &self,
        origin: &NativeReplacementStartOrigin,
        wait: &crate::AuthorityWait,
    ) -> Result<(PeerAuthorityGuard, NativeReplacementStartRead)> {
        let expiry = origin.expires_at;
        let remaining = chrono::DateTime::from_timestamp(expiry, 0)
            .and_then(|expiry| (expiry - chrono::Utc::now()).to_std().ok())
            .ok_or_else(Error::forbidden)?;
        let mut proof =
            self.admit_peer_authority_mode(&origin.request, &wait.bounded(remaining), false)?;
        proof.limit_expiry(expiry)?;
        let read = self.native_replacement_start_read_under(origin, &proof)?;
        Ok((proof, read))
    }
    /// The dormant target's initial muted Core activation owns exact current
    /// source/target authority and its SAME just-validated projection. No existing
    /// route or durable lineage changes; the pair retains the original expiry.
    pub fn resume_native_replacement_start_activation_authority(
        &self,
        origin: &NativeReplacementStartOrigin,
        wait: &crate::AuthorityWait,
    ) -> Result<(PeerAuthorityGuard, NativeReplacementStartRead)> {
        self.resume_native_replacement_start_with_read(origin, wait)
    }
    pub fn native_replacement_start_read_under(
        &self,
        origin: &NativeReplacementStartOrigin,
        proof: &PeerAuthorityGuard,
    ) -> Result<NativeReplacementStartRead> {
        if !proof.current()
            || (proof.request.credentials != origin.request.credentials
                || proof.request.device != origin.request.device
                || proof.request.key_id != origin.request.key_id
                || proof.request.algorithm != origin.request.algorithm
                || proof.request.command != origin.request.command)
        {
            return Err(Error::forbidden());
        }
        let (member, saved, descriptor) = {
            let c = self.try_connection()?;
            let (member, saved, scopes, expires) = facts(self, &c, origin)?;
            if !self.authority_covers(Self::peer_scopes(proof), &scopes) || crate::now() >= expires
            {
                return Err(Error::forbidden());
            }
            let descriptor: serde_json::Value =
                serde_json::from_str(&origin.target).map_err(|_| Error::storage())?;
            if descriptor["public_key"] != serde_json::json!(proof.key.public_key)
                || descriptor["key_id"] != proof.key_id()
            {
                return Err(Error::forbidden());
            }
            (member, saved, descriptor)
        };
        let source = origin
            .allocated
            .attempt
            .prepared
            .selection
            .read_with_peer(self, proof)?;
        let target = projection(&source, member, &saved, &descriptor)?;
        Ok(NativeReplacementStartRead {
            source,
            target,
            descriptor,
        })
    }
    pub fn native_replacement_start_record_with_peer(
        &self,
        origin: &NativeReplacementStartOrigin,
        proof: &PeerAuthorityGuard,
    ) -> Result<OperationRecord> {
        let _read = self.native_replacement_start_read_under(origin, proof)?;
        let c = self.try_connection()?;
        let row = rows::by_record(&c, origin.operation_record_id())?;
        admission::require_row(self, Self::peer_scopes(proof), &row)?;
        let mut record = row.record;
        record.result = None;
        record.problem = None;
        record.effects.clear();
        Ok(record)
    }
}

impl Store {
    /// Authenticate the decoder observation against the SAME saved target offer,
    /// key, endpoint and logical stream intent on RTC or native transport. This
    /// begins no transport effect and accepts no server-issued challenge as authority.
    pub fn native_replacement_ready_read_with_peer(
        &self,
        started: &NativeReplacementStarted,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<NativeReplacementStartRead> {
        let a = &started.attempt;
        let (member, saved, descriptor) = {
            let c = self.try_connection()?;
            let saved = allocated_current(self, &c, &a.allocated)?;
            let (member, mut descriptor) = target_facts_for_profile(
                self,
                &c,
                &a.allocated.attempt.prepared,
                credential,
                proof,
                &saved,
                "connect-replacement-ready-v1",
            )?;
            descriptor["intent"]["offer"]["replacement_preparation"]["profile"] =
                serde_json::json!("connect-replacement-media-v1");
            descriptor["intent"]["offer"]["replacement_preparation"]
                .as_object_mut()
                .ok_or_else(invalid)?
                .remove("readiness");
            if crate::provisioning::canonical_json(&descriptor)? != a.target {
                return Err(conflict());
            }
            (member, saved, descriptor)
        };
        let source = a
            .allocated
            .attempt
            .prepared
            .selection
            .read_with_peer(self, proof)?;
        let target = projection(&source, member, &saved, &descriptor)?;
        Ok(NativeReplacementStartRead {
            source,
            target,
            descriptor,
        })
    }
    /// A repeated ready message may observe the original result after adoption;
    /// it cannot recreate readiness or repeat any Core transition.
    pub fn replay_native_replacement_ready_with_peer(
        &self,
        started: &NativeReplacementStarted,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<OperationRecord> {
        let a = &started.attempt;
        let p = &a.allocated.attempt.prepared;
        local_owner(self, &p.owner, &p.runtime)?;
        let c = self.try_connection()?;
        self.require_peer_source(&c, proof, credential)?;
        actor_from_connection(&c, credential)?.require("connection.offer")?;
        let PeerCommand::NativeConnectionOffer(intent) = &proof.request.command else {
            return Err(Error::forbidden());
        };
        let mut normalized = intent.clone();
        let marker = &mut normalized.offer["replacement_preparation"];
        if marker["profile"] != "connect-replacement-ready-v1"
            || marker.as_object().map(|v| v.len()) != Some(3)
        {
            return Err(Error::forbidden());
        }
        marker["profile"] = serde_json::json!("connect-replacement-media-v1");
        marker
            .as_object_mut()
            .ok_or_else(invalid)?
            .remove("readiness");
        let frozen: serde_json::Value =
            serde_json::from_str(&a.target).map_err(|_| Error::storage())?;
        if frozen["intent"] != serde_json::to_value(normalized).map_err(|_| Error::storage())?
            || frozen["credential_id"] != credential
            || frozen["key_id"] != proof.key_id()
            || frozen["device_id"] != proof.key.device_id
            || frozen["public_key"] != serde_json::json!(proof.key.public_key)
        {
            return Err(Error::forbidden());
        }
        let row = rows::by_record(&c, &p.record)?;
        admission::require_row(self, Self::peer_scopes(proof), &row)?;
        let mut record = row.record;
        record.result = None;
        record.problem = None;
        record.effects.clear();
        Ok(record)
    }
}
