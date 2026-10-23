//! Exact replacement intention and provisioned destination; no IO or ready grant.
use super::*;
use connect_protocol::provisioning::EndpointTransport;
use std::sync::Weak;

#[derive(Clone)]
pub struct NativeReplacementOrigin {
    pub(crate) owner: Weak<crate::InstallationOwnership>,
    pub(crate) runtime: String,
    pub(crate) record: String,
    pub(crate) credential: String,
    pub(crate) digest: String,
    pub(crate) request: TransitionRequest,
}
impl std::fmt::Debug for NativeReplacementOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeReplacementOrigin(redacted)")
    }
}
/// Current destination metadata under the original operation footprint. This
/// contains no target credential, media observation, callback or execution lease.
#[derive(Clone, PartialEq, Eq)]
pub struct NativeReplacementContext {
    pub operation_record_id: String,
    pub conversation_id: String,
    pub session_id: String,
    pub participant_id: String,
    pub identity_id: String,
    pub membership_epoch: u64,
    pub role: String,
    pub source_connection_id: String,
    pub source_runtime_connection_id: String,
    pub source_generation: u64,
    pub retained_connection_id: String,
    pub retained_runtime_connection_id: String,
    pub retained_generation: u64,
    pub candidate_connection_id: Option<String>,
    pub target_endpoint_id: String,
    pub target_endpoint_revision: u64,
    pub target_device_id: String,
    pub target_device_kind: String,
    pub target_device_platform: String,
    pub target_transport: EndpointTransport,
    pub target_address: String,
    pub target_capabilities: Vec<String>,
    pub target_secret_ref: Option<String>,
    pub target_route_owner_id: String,
}
impl std::fmt::Debug for NativeReplacementContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeReplacementContext(redacted)")
    }
}
impl NativeReplacementOrigin {
    pub fn record_id(&self) -> &str {
        &self.record
    }
    pub fn request(&self) -> &TransitionRequest {
        &self.request
    }
    pub(crate) fn context(
        &self,
        store: &Store,
        c: &Connection,
    ) -> Result<NativeReplacementContext> {
        context(store, c, self)
    }
    pub(crate) fn require(&self, store: &Store, c: &Connection) -> Result<Actor> {
        let owner = store.require_runtime_owner()?;
        if self.runtime != store.runtime_epoch()
            || !owner.current()
            || !self
                .owner
                .upgrade()
                .is_some_and(|old| Arc::ptr_eq(&old, &owner))
        {
            return Err(Error::forbidden());
        }
        let actor = actor_from_connection(c, &self.credential)?;
        actor.require("connection.replace")?;
        let row = rows::by_record(c, &self.record)?;
        if row.digest != self.digest || rows::request(&row)? != self.request {
            return Err(Error::forbidden());
        }
        if row.native_replacement {
            let exact:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id WHERE n.operation_record_id=?1 AND n.credential_id=?2 AND n.runtime_epoch=?3 AND n.original_digest=?4 AND n.reservation_id=o.lease_owner AND n.expires_at>?5 AND o.lease_expires_at=n.expires_at)",params![self.record,self.credential,self.runtime,self.digest,crate::now()],|r|r.get(0)).map_err(|_|Error::storage())?;
            if !exact || row.record.state != OperationState::Preparing {
                return Err(Error::forbidden());
            }
        } else if row.record.state != OperationState::Accepted
            || row.worker.is_some()
            || !row.record.effects.is_empty()
        {
            return Err(Error::forbidden());
        }
        authority::execution(c, &actor, &row, store)?;
        Ok(actor)
    }
}
impl Store {
    /// A source for an already admitted exact command. It keeps no reader and
    /// cannot reserve a Core candidate or authorize telephone submission.
    pub fn native_replacement_origin(
        &self,
        credential: &str,
        mut command: TransitionCommand,
    ) -> Result<NativeReplacementOrigin> {
        command.normalize()?;
        if !matches!(command.intent, TransitionIntent::Replace { .. }) {
            return Err(Error::forbidden());
        }
        let _held = self.transition_operation_authority(
            credential,
            &command.operation_id,
            &["connection.replace"],
            None,
        )?;
        let c = self.connection()?;
        let actor = actor_from_connection(&c, credential)?;
        let row = rows::find(
            &c,
            &authority::tenant(&c, &actor)?,
            &actor.identity_id,
            &command.operation_id,
        )?
        .ok_or_else(Error::forbidden)?;
        let request = command.with_expiry(row.record.expires_at.ok_or_else(Error::storage)?);
        if rows::request(&row)? != request {
            return Err(Error::forbidden());
        }
        let owner = self.require_runtime_owner()?;
        let origin = NativeReplacementOrigin {
            owner: Arc::downgrade(&owner),
            runtime: self.runtime_epoch().into(),
            record: row.record.record_id,
            credential: credential.into(),
            digest: row.digest,
            request,
        };
        origin.require(self, &c)?;
        context(self, &c, &origin)?;
        Ok(origin)
    }
    pub fn native_replacement_context(
        &self,
        origin: &NativeReplacementOrigin,
    ) -> Result<NativeReplacementContext> {
        let _held = self.transition_operation_authority(
            &origin.credential,
            &origin.request.operation_id,
            &["connection.replace"],
            None,
        )?;
        let c = self.connection()?;
        origin.require(self, &c)?;
        context(self, &c, origin)
    }
}
struct Source {
    participant: String,
    identity: String,
    epoch: u64,
    generation: u64,
    endpoint: String,
    runtime: String,
    role: String,
}
fn source(store: &Store, c: &Connection, request: &TransitionRequest, id: &str) -> Result<Source> {
    c.query_row(
        "SELECT v.participant_id,v.identity_id,v.membership_epoch,v.generation,v.endpoint_id,m.runtime_connection_id,sp.role FROM connections v JOIN sessions s ON s.id=v.session_id JOIN participants p ON p.participant_id=v.participant_id AND p.membership_epoch=v.membership_epoch AND p.active=1 JOIN session_participants sp ON sp.session_id=v.session_id AND sp.participant_id=v.participant_id AND sp.membership_epoch=v.membership_epoch AND sp.state='present' JOIN connection_offers m ON m.connection_id=v.id WHERE v.id=?1 AND v.session_id=?2 AND s.conversation_id=?3 AND v.observed_runtime_id=?4 AND v.state IN ('ready','active','held') AND (EXISTS(SELECT 1 FROM native_connection_lifetimes l JOIN native_admission_sources n ON n.operation_record_id=l.operation_record_id AND n.generation=v.generation AND n.endpoint_id=v.endpoint_id JOIN operations o ON o.record_id=n.operation_record_id AND o.state='completed' JOIN operation_effects e ON e.id=n.effect_id AND e.state='confirmed' WHERE l.connection_id=v.id AND l.generation=v.generation AND l.runtime_epoch=v.observed_runtime_id AND l.retired=0) OR EXISTS(SELECT 1 FROM telephone_connection_lifetimes l JOIN telephone_replacement_commits t ON t.operation_record_id=l.operation_record_id AND t.runtime_epoch=l.runtime_epoch JOIN native_replacements n ON n.operation_record_id=t.operation_record_id AND n.candidate_connection_id=v.id AND n.candidate_runtime_id=m.runtime_connection_id AND n.generation=v.generation JOIN operations o ON o.record_id=t.operation_record_id AND o.state='completed' JOIN operation_effects e ON e.id=t.effect_id AND e.state='confirmed' WHERE l.connection_id=v.id AND l.generation=v.generation AND l.runtime_epoch=v.observed_runtime_id AND l.retired=0))",
        params![id,request.session_id,request.conversation_id,store.runtime_epoch()],
        |r|Ok(Source {participant:r.get(0)?,identity:r.get(1)?,epoch:r.get(2)?,generation:r.get(3)?,endpoint:r.get(4)?,runtime:r.get(5)?,role:r.get(6)?})
    ).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)
}
fn context(
    store: &Store,
    c: &Connection,
    origin: &NativeReplacementOrigin,
) -> Result<NativeReplacementContext> {
    let TransitionIntent::Replace {
        endpoint_id,
        candidate_connid,
        retained_connid,
    } = &origin.request.intent
    else {
        return Err(Error::forbidden());
    };
    let source_id = origin
        .request
        .connection_id
        .as_ref()
        .ok_or_else(Error::forbidden)?;
    let old = source(store, c, &origin.request, source_id)?;
    let remote = source(store, c, &origin.request, retained_connid)?;
    if old.participant == remote.participant {
        return Err(Error::forbidden());
    }
    let endpoint = if let Some(endpoint) = endpoint_id {
        endpoint.clone()
    } else {
        // Candidate selection still grants no readiness or right to adopt an
        // ordinary binding. Managed replacement must separately own its ticket.
        c.query_row("SELECT endpoint_id FROM connections WHERE id=?1 AND session_id=?2 AND participant_id=?3 AND membership_epoch=?4 AND state IN ('preparing','ready','active')",params![candidate_connid,origin.request.session_id,old.participant,old.epoch],|r|r.get(0)).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?
    };
    if endpoint == old.endpoint {
        return Err(Error::new(
            "invalid-request",
            "Endpoint replacement requires a distinct provisioned Endpoint",
        ));
    }
    let target: Option<(u64,String,String,String,String,String,String,Option<String>,String)> = c.query_row(
        "SELECT e.revision,e.device_id,d.kind,d.platform,e.transport,e.address,e.capabilities_json,e.secret_ref,e.route_owner_id FROM endpoints e JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.revoked=0 JOIN identities i ON i.id=e.identity_id AND i.revoked=0 WHERE e.id=?1 AND e.identity_id=?2 AND e.verification_state='verified'",
        params![endpoint,old.identity],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))
    ).optional().map_err(|_|Error::storage())?;
    let (
        revision,
        device,
        kind,
        platform,
        transport,
        address,
        capabilities,
        secret_ref,
        route_owner,
    ) = target.ok_or_else(Error::forbidden)?;
    let frozen:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM operation_endpoints WHERE operation_record_id=?1 AND endpoint_id=?2 AND revision=?3)",params![origin.record,endpoint,revision],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !frozen {
        return Err(Error::forbidden());
    }
    let capabilities: Vec<String> =
        serde_json::from_str(&capabilities).map_err(|_| Error::storage())?;
    if !capabilities.iter().any(|v| v == "audio") {
        return Err(Error::new(
            "unsupported-media-profile",
            "Replacement Endpoint has no audio capability",
        ));
    }
    let transport = serde_json::from_value(serde_json::Value::String(transport))
        .map_err(|_| Error::storage())?;
    Ok(NativeReplacementContext {
        operation_record_id: origin.record.clone(),
        conversation_id: origin.request.conversation_id.clone(),
        session_id: origin.request.session_id.clone(),
        participant_id: old.participant,
        identity_id: old.identity,
        membership_epoch: old.epoch,
        role: old.role,
        source_connection_id: source_id.clone(),
        source_runtime_connection_id: old.runtime,
        source_generation: old.generation,
        retained_connection_id: retained_connid.clone(),
        retained_runtime_connection_id: remote.runtime,
        retained_generation: remote.generation,
        candidate_connection_id: candidate_connid.clone(),
        target_endpoint_id: endpoint,
        target_endpoint_revision: revision,
        target_device_id: device,
        target_device_kind: kind,
        target_device_platform: platform,
        target_transport: transport,
        target_address: address,
        target_capabilities: capabilities,
        target_secret_ref: secret_ref,
        target_route_owner_id: route_owner,
    })
}
