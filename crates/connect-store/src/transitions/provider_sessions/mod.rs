//! Canonical Session-owned normalized provider effects, separate from media.
use super::*;
use crate::{AuthorityGuardSet, AuthorityScope, AuthorityWait, InstallationOwnership};
use connect_connector_api::{lifecycle::*, manifest::*, Failure, VoiceReceipt, VoiceStart};
use connect_protocol::{membership::MemberView, session_control, Operation};
use serde::{Deserialize, Serialize};
use std::sync::Weak;
mod ingress;
mod lifecycle;
pub use ingress::{
    ProviderIncomingVoiceResolution, ProviderIncomingVoiceResolutionCapture,
    ProviderIncomingVoiceResolutionPreparation, ProviderIncomingVoiceResolutionResult,
    ProviderVoiceImport, ProviderVoiceIngressPolicy, ProviderVoiceScan, ProviderVoiceScanCapture,
};
mod ownership;
mod quarantine;
pub use ownership::ProviderSessionVoiceWorker;
#[cfg(test)]
mod tests;

/// Protected host configuration; never a wire-selectable route or principal.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderSessionVoicePolicy {
    pub endpoint_id: String,
    pub endpoint_credential_id: String,
    pub route_credential_id: String,
    pub route_reference: String,
    pub provider: String,
    pub assistant_id: String,
    pub assistant_endpoint_id: String,
    pub assistant_credential_id: String,
    pub phone_number_id: String,
    pub timeout_seconds: u32,
}
impl ProviderSessionVoicePolicy {
    pub fn validate(&self) -> Result<()> {
        for value in [
            &self.endpoint_id,
            &self.endpoint_credential_id,
            &self.route_credential_id,
            &self.route_reference,
            &self.provider,
            &self.assistant_id,
            &self.assistant_endpoint_id,
            &self.assistant_credential_id,
            &self.phone_number_id,
        ] {
            id(value)?;
        }
        if !(1..=30).contains(&self.timeout_seconds) {
            return Err(invalid());
        }
        Ok(())
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    invitation_record_id: String,
    request: session_control::Request,
    caller: MemberView,
    target: MemberView,
    audience: Vec<MemberView>,
    endpoint_id: String,
    endpoint_revision: u64,
    device_id: String,
    device_kind: String,
    device_platform: String,
    address: String,
    route_owner_id: String,
    assistant_endpoint_revision: u64,
    assistant_device_id: String,
}
#[derive(Clone)]
pub struct ProviderSessionVoiceOrigin {
    owner: Weak<InstallationOwnership>,
    runtime: String,
    credential: String,
    id: String,
    source: Source,
    policy: ProviderSessionVoicePolicy,
    expires: i64,
    direction: ProviderCallDirection,
}
pub use connect_protocol::events::{
    ProviderCallDirection, ProviderCallState, ProviderSessionVoiceView,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderSessionVoiceAction {
    Start,
    Status,
    Reconcile,
    ResolveQuarantine,
    End,
}
/// One bounded dispatch to an exact captured resource. No public constructor.
pub struct ProviderSessionVoiceDispatch {
    origin: ProviderSessionVoiceOrigin,
    action: ProviderSessionVoiceAction,
    provider_id: Option<String>,
    control_url: Option<String>,
    quarantine: Option<quarantine::Ticket>,
    _authority: Option<Arc<AuthorityGuardSet>>,
    expires: i64,
    owner: Arc<InstallationOwnership>,
    manager: Arc<crate::authority_fences::Manager>,
    worker: Arc<ProviderSessionVoiceWorker>,
}
/// Historical result custody releases the dispatch reader before any writer.
pub struct ProviderSessionVoiceCapture {
    origin: ProviderSessionVoiceOrigin,
    action: ProviderSessionVoiceAction,
    provider_id: Option<String>,
    control_url: Option<String>,
    ended_reason: Option<String>,
    observation: Observation,
    quarantine: Option<quarantine::Ticket>,
    _owner: Arc<InstallationOwnership>,
    manager: Arc<crate::authority_fences::Manager>,
    worker: Arc<ProviderSessionVoiceWorker>,
}
macro_rules! redacted {($($t:ty),*)=>{$(impl std::fmt::Debug for $t {fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{ f.write_str(concat!(stringify!($t),"(redacted)")) }})*};}
redacted!(
    ProviderSessionVoicePolicy,
    ProviderSessionVoiceOrigin,
    ProviderSessionVoiceDispatch,
    ProviderSessionVoiceCapture
);
impl ProviderSessionVoiceOrigin {
    pub fn direction(&self) -> ProviderCallDirection {
        self.direction
    }
    pub fn policy(&self) -> &ProviderSessionVoicePolicy {
        &self.policy
    }
    pub fn operation_id(&self) -> &str {
        &self.source.request.operation_id
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn expires_at(&self) -> i64 {
        self.expires
    }
    pub fn session_id(&self) -> &str {
        &self.source.request.session_id
    }
    pub fn invitation_record_id(&self) -> &str {
        &self.source.invitation_record_id
    }
    pub fn endpoint_credential_id(&self) -> &str {
        &self.policy.endpoint_credential_id
    }
    pub fn participant_id(&self) -> &str {
        &self.source.target.participant_id
    }
    pub fn conversation_id(&self) -> &str {
        &self.source.request.conversation_id
    }
    fn lifetime(&self, store: &Store) -> Result<()> {
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
        Ok(())
    }
    fn current(&self, store: &Store, c: &Connection) -> Result<i64> {
        self.lifetime(store)?;
        let actor = actor_from_connection(c, &self.credential)?;
        let (current, expiry) = source(
            c,
            &actor,
            &self.source.request,
            &self.source.target.participant_id,
            &self.policy,
        )?;
        if current != self.source || crate::now() >= self.expires {
            return Err(Error::forbidden());
        }
        Ok(expiry.min(self.expires))
    }
}
fn scopes(source: &Source) -> Result<Vec<AuthorityScope>> {
    let mut scopes = vec![
        AuthorityScope::conversation(&source.request.conversation_id, crate::AuthorityMode::Read)?,
        AuthorityScope::identity(&source.route_owner_id, crate::AuthorityMode::Read)?,
    ];
    for m in &source.audience {
        scopes.push(AuthorityScope::identity(
            &m.identity_id,
            crate::AuthorityMode::Read,
        )?);
    }
    Ok(scopes)
}

fn source(
    c: &Connection,
    caller: &Actor,
    request: &session_control::Request,
    target: &str,
    policy: &ProviderSessionVoicePolicy,
) -> Result<(Source, i64)> {
    policy.validate()?;
    caller.require("session.invite")?;
    let mut normalized = request.clone();
    normalized.normalize()?;
    if &normalized != request {
        return Err(invalid());
    }
    let session_control::Action::Invite {
        participants,
        medium,
        ..
    } = &request.action
    else {
        return Err(Error::forbidden());
    };
    if !participants.iter().any(|p| p == target)
        || !session_control::media(medium)?.iter().any(|v| v == "audio")
    {
        return Err(Error::forbidden());
    }
    let saved =
        crate::operations::receipts::receipt(c, &caller.identity_id, &request.operation_id)?
            .ok_or_else(Error::forbidden)?;
    crate::operations::receipts::check_receipt_authority(c, caller, &saved, None)?;
    if saved.operation
        != (Operation::SessionControl {
            request: request.clone(),
        })
        || saved.result.is_err()
    {
        return Err(Error::forbidden());
    }
    let parent:String=c.query_row("SELECT record_id FROM operations WHERE owner_id=?1 AND operation_id=?2 AND state='completed'",params![caller.identity_id,request.operation_id],|r|r.get(0)).map_err(|_|Error::forbidden())?;
    let own = crate::audience::member(c, &caller.identity_id, &request.conversation_id, true)?;
    let identity: String = c
        .query_row(
            "SELECT identity_id FROM participants WHERE participant_id=?1 AND conversation_id=?2",
            params![target, request.conversation_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::forbidden())?;
    let target = crate::audience::member(c, &identity, &request.conversation_id, true)?;
    if target.identity_id == caller.identity_id || !saved.resources.contains(&identity) {
        return Err(Error::forbidden());
    }
    let present:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM session_participants p JOIN sessions s ON s.id=p.session_id JOIN conversations v ON v.id=s.conversation_id WHERE p.session_id=?1 AND p.participant_id=?2 AND p.membership_epoch=?3 AND p.state IN ('invited','present') AND s.state IN ('invited','accepted','ready') AND v.closed=0)",params![request.session_id,target.participant_id,target.membership_epoch],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !present {
        return Err(Error::forbidden());
    }
    let duplex_roles:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM session_participants a JOIN session_participants b ON b.session_id=a.session_id WHERE a.session_id=?1 AND a.participant_id=?2 AND a.membership_epoch=?3 AND a.role!='observer' AND b.participant_id=?4 AND b.membership_epoch=?5 AND b.role!='observer')",params![request.session_id,own.participant_id,own.membership_epoch,target.participant_id,target.membership_epoch],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !duplex_roles || own.role == "observer" || target.role == "observer" {
        return Err(Error::new(
            "unsupported-feature",
            "Provider telephone control cannot enforce observer audio roles",
        ));
    }
    let ep = actor_from_connection(c, &policy.endpoint_credential_id)?;
    let route = actor_from_connection(c, &policy.route_credential_id)?;
    ep.require("connection.offer")?;
    route.require("connection.offer")?;
    ep.require("session.accept")?;
    let assistant = actor_from_connection(c, &policy.assistant_credential_id)?;
    assistant.require("connection.offer")?;
    assistant.require("session.invite")?;
    if assistant.identity_id != caller.identity_id
        || own.kind != connect_protocol::provisioning::IdentityKind::Ai
        || authority::tenant(c, &assistant)? != authority::tenant(c, caller)?
    {
        return Err(Error::forbidden());
    }
    let (assistant_endpoint_revision,assistant_device_id):(u64,String)=c.query_row("SELECT e.revision,e.device_id FROM endpoints e JOIN identities i ON i.id=e.identity_id AND i.kind='ai' AND i.revoked=0 JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.revoked=0 WHERE e.id=?1 AND e.identity_id=?2 AND e.transport='vapi-voice' AND e.verification_state='verified' AND e.address=?3 AND e.secret_ref=?4 AND e.route_owner_id=?5 AND EXISTS(SELECT 1 FROM json_each(e.capabilities_json) WHERE value='audio')",params![policy.assistant_endpoint_id,assistant.identity_id,policy.assistant_id,policy.route_reference,route.identity_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::forbidden())?;

    if authority::tenant(c, &ep)? != authority::tenant(c, caller)?
        || authority::tenant(c, &route)? != authority::tenant(c, caller)?
    {
        return Err(Error::forbidden());
    }
    if ep.identity_id != identity {
        return Err(Error::forbidden());
    }
    let mut audience = vec![];
    for identity in saved.resources {
        if !ep.permits_recipient(&identity)
            || !route.permits_recipient(&identity)
            || !assistant.permits_recipient(&identity)
        {
            return Err(Error::forbidden());
        }
        audience.push(crate::audience::member(
            c,
            &identity,
            &request.conversation_id,
            true,
        )?);
    }
    audience.sort_by(|a, b| a.participant_id.cmp(&b.participant_id));
    crate::audience::require_original(
        c,
        &ep,
        &request.conversation_id,
        connect_protocol::membership::ContentKind::Session,
        &request.session_id,
    )?;
    let (revision,device,kind,platform,address,owner,secret,caps):(u64,String,String,String,String,String,Option<String>,String)=c.query_row("SELECT e.revision,e.device_id,d.kind,d.platform,e.address,e.route_owner_id,e.secret_ref,e.capabilities_json FROM endpoints e JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.revoked=0 WHERE e.id=?1 AND e.identity_id=?2 AND e.verification_state='verified' AND e.transport='telephone'",params![policy.endpoint_id,identity],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).map_err(|_|Error::forbidden())?;
    let capabilities: Vec<String> = serde_json::from_str(&caps).map_err(|_| Error::storage())?;
    if owner != route.identity_id
        || secret.as_deref() != Some(&policy.route_reference)
        || !capabilities.iter().any(|v| v == "audio")
        || !(9..=16).contains(&address.len())
        || !address.starts_with('+')
        || address.as_bytes()[1] == b'0'
        || !address.as_bytes()[1..].iter().all(u8::is_ascii_digit)
    {
        return Err(Error::forbidden());
    }
    let expires = caller
        .expires_at
        .min(ep.expires_at)
        .min(route.expires_at)
        .min(assistant.expires_at);
    Ok((
        Source {
            invitation_record_id: parent,
            request: request.clone(),
            caller: own,
            target,
            audience,
            endpoint_id: policy.endpoint_id.clone(),
            endpoint_revision: revision,
            device_id: device,
            device_kind: kind,
            device_platform: platform,
            address,
            route_owner_id: owner,
            assistant_endpoint_revision,
            assistant_device_id,
        },
        expires,
    ))
}
impl Store {
    /// Attach a provider-control intent to the exact caller's completed invite.
    /// No old Call command, application Task, local Connection or media grant is created.
    pub fn prepare_provider_session_voice(
        &self,
        credential: &str,
        mut request: session_control::Request,
        target: &str,
        policy: ProviderSessionVoicePolicy,
    ) -> Result<ProviderSessionVoiceOrigin> {
        request.normalize()?;
        let _guard = self.stable_authority(
            &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let actor = actor_from_connection(&c, credential)?;
                scopes(&source(&c, &actor, &request, target, &policy)?.0)
            },
        )?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential)?;
        let (source, expiry) = source(&tx, &actor, &request, target, &policy)?;
        let id = format!(
            "pvoice_{}",
            hex::encode(Sha256::digest(format!(
                "{}\0{}",
                source.invitation_record_id, target
            )))
        );
        let old:Option<(String,String,String,i64)>=tx.query_row("SELECT caller_credential_id,source_json,policy_json,expires_at FROM provider_session_voice WHERE id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|Error::storage())?;
        let expires = if let Some((saved_credential, saved_source, saved_policy, expires)) = old {
            if saved_credential != credential
                || serde_json::from_str::<Source>(&saved_source).map_err(|_| Error::storage())?
                    != source
                || serde_json::from_str::<ProviderSessionVoicePolicy>(&saved_policy)
                    .map_err(|_| Error::storage())?
                    != policy
            {
                return Err(conflict());
            }
            expires
        } else {
            let (total,active):(u64,u64)=tx.query_row("SELECT COUNT(*),COALESCE(SUM(closed_at IS NULL),0) FROM provider_session_voice",[],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::storage())?;
            if total >= self.limits.operation_receipts || active >= self.limits.connections as u64 {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Provider Session capacity exhausted",
                ));
            }
            let projection = Projection::new(id.clone(), State::Voice(VoiceState::Starting))
                .map_err(|_| invalid())?;
            let expires = (crate::now() + i64::from(policy.timeout_seconds)).min(expiry);
            tx.execute("INSERT INTO provider_session_voice(id,invitation_record_id,caller_credential_id,target_participant_id,endpoint_id,source_json,policy_json,projection_json,expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![id,source.invitation_record_id,credential,target,policy.endpoint_id,serde_json::to_string(&source).map_err(|_|Error::storage())?,serde_json::to_string(&policy).map_err(|_|Error::storage())?,serde_json::to_string(&projection).map_err(|_|Error::storage())?,expires]).map_err(|_|Error::storage())?;
            expires
        };
        let owner = self.require_runtime_owner()?;
        let result = ProviderSessionVoiceOrigin {
            owner: Arc::downgrade(&owner),
            runtime: self.runtime_epoch().into(),
            credential: credential.into(),
            id,
            source,
            policy,
            expires,
            direction: ProviderCallDirection::Outbound,
        };
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
    /// Read a provider observation under current Session/audience/epoch policy.
    pub fn provider_session_voice_view(
        &self,
        credential: &str,
        id: &str,
    ) -> Result<ProviderSessionVoiceView> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let actor = actor_from_connection(&tx, credential)?;
        view_for_actor(&tx, &actor, id)
    }
}
fn public_state(state: VoiceState) -> ProviderCallState {
    match state {
        VoiceState::Starting => ProviderCallState::Starting,
        VoiceState::Active => ProviderCallState::Active,
        VoiceState::Ended => ProviderCallState::Ended,
        VoiceState::Failed => ProviderCallState::Failed,
        VoiceState::Uncertain => ProviderCallState::Unknown,
    }
}
fn load_source(c: &Connection, id: &str) -> Result<Source> {
    let raw: String = c
        .query_row(
            "SELECT source_json FROM provider_session_voice WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| Error::forbidden())?;
    serde_json::from_str(&raw).map_err(|_| Error::storage())
}
pub(crate) fn load_view(c: &Connection, id: &str) -> Result<ProviderSessionVoiceView> {
    let source = load_source(c, id)?;
    let (raw,observed_at,closed_at,ended_reason,quarantined,revision,refusal):(String,Option<i64>,Option<i64>,Option<String>,bool,u64,Option<String>)=c.query_row("SELECT projection_json,observed_at,closed_at,ended_reason,quarantine_json IS NOT NULL AND NOT EXISTS(SELECT 1 FROM provider_voice_quarantine_resolutions r WHERE r.call_id=provider_session_voice.id AND r.provider_id=provider_session_voice.provider_id),revision,refusal_code FROM provider_session_voice WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).map_err(|_|Error::storage())?;
    let projection: Projection = serde_json::from_str(&raw).map_err(|_| Error::storage())?;
    let State::Voice(state) = projection.state else {
        return Err(Error::storage());
    };
    let ended_reason = if let Some(reason) = ended_reason {
        let (provider, control, historical): (Option<String>, Option<String>, Option<String>) = c
            .query_row(
                "SELECT provider_id,control_url,quarantine_json FROM provider_session_voice WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|_| Error::storage())?;
        let mut private = vec![provider, control];
        if let Some(raw) = historical {
            let old: serde_json::Value =
                serde_json::from_str(&raw).map_err(|_| Error::storage())?;
            for key in ["provider_id", "control_url"] {
                private.push(
                    old.get(key)
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                );
            }
        }
        if reason.contains("://")
            || private
                .iter()
                .filter_map(|s| s.as_deref())
                .filter(|v| !v.is_empty())
                .any(|private| reason.contains(private))
        {
            None
        } else {
            Some(reason)
        }
    } else {
        None
    };
    Ok(ProviderSessionVoiceView {
        id: id.into(),
        operation_id: source.request.operation_id,
        conversation_id: source.request.conversation_id,
        session_id: source.request.session_id,
        participant_id: source.target.participant_id,
        membership_epoch: source.target.membership_epoch,
        endpoint_id: source.endpoint_id,
        state: public_state(state),
        direction: call_direction(c, id)?,
        remote_identity_verified: false,
        revision,
        observed_at,
        closed_at,
        ended_reason,
        problem: if quarantined {
            Some("provider-binding-conflict".into())
        } else {
            refusal
        },
        local_media: false,
    })
}
pub(crate) fn view_for_actor(
    c: &Connection,
    actor: &Actor,
    id: &str,
) -> Result<ProviderSessionVoiceView> {
    actor.require("session_get")?;
    let source = load_source(c, id)?;
    crate::audience::require_original(
        c,
        actor,
        &source.request.conversation_id,
        connect_protocol::membership::ContentKind::Session,
        &source.request.session_id,
    )?;
    let member =
        crate::audience::member(c, &actor.identity_id, &source.request.conversation_id, true)?;
    if !source.audience.iter().any(|m| {
        m.participant_id == member.participant_id
            && m.identity_id == member.identity_id
            && m.membership_epoch == member.membership_epoch
    }) {
        return Err(Error::forbidden());
    }
    load_view(c, id)
}
pub(crate) fn views_for_conversation(
    c: &Connection,
    actor: &Actor,
    cid: &str,
) -> Result<Vec<ProviderSessionVoiceView>> {
    if !actor.permits("session_get") {
        return Ok(vec![]);
    }
    let mut q=c.prepare("SELECT id FROM provider_session_voice WHERE json_extract(source_json,'$.request.conversation_id')=?1 ORDER BY id").map_err(|_|Error::storage())?;
    let ids = q
        .query_map([cid], |r| r.get::<_, String>(0))
        .map_err(|_| Error::storage())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| Error::storage())?;
    let mut views = vec![];
    for id in ids {
        match view_for_actor(c, actor, &id) {
            Ok(v) => views.push(v),
            Err(e) if e.code == "forbidden" || e.code == "unauthorized" => (),
            Err(e) => return Err(e),
        }
    }
    Ok(views)
}

fn call_direction(c: &Connection, id: &str) -> Result<ProviderCallDirection> {
    let value: String = c
        .query_row(
            "SELECT direction FROM provider_session_voice WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    match value.as_str() {
        "inbound" => Ok(ProviderCallDirection::Inbound),
        "outbound" => Ok(ProviderCallDirection::Outbound),
        _ => Err(Error::storage()),
    }
}
