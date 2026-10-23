//! Original invitation slots for owned Session exchanges. A source is metadata,
//! never a command, Device, media or historical-content authorization grant.
use crate::{
    Actor, AuthorityGuardSet, AuthorityMode, AuthorityScope, InstallationOwnership, Store,
};
use connect_protocol::{
    membership::{ContentKind, MemberView},
    session_control::Request,
    Error, Result,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::sync::Arc;
pub(crate) mod cancellations;
pub use cancellations::SessionInvitationCancellation;
pub(crate) mod decisions;
pub(crate) mod relays;

#[derive(Clone)]
pub struct SessionInvitationSource {
    owner: Arc<InstallationOwnership>,
    runtime: String,
    source: String,
    participant: String,
    identity: String,
    epoch: u64,
}
impl std::fmt::Debug for SessionInvitationSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionInvitationSource(redacted)")
    }
}
impl PartialEq for SessionInvitationSource {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.owner, &other.owner)
            && self.runtime == other.runtime
            && self.source == other.source
            && self.participant == other.participant
            && self.identity == other.identity
            && self.epoch == other.epoch
    }
}
impl Eq for SessionInvitationSource {}
impl SessionInvitationSource {
    pub fn invitation_id(&self) -> &str {
        &self.source
    }
    pub fn target_identity_id(&self) -> &str {
        &self.identity
    }
    pub fn target_participant_id(&self) -> &str {
        &self.participant
    }
    pub fn target_membership_epoch(&self) -> u64 {
        self.epoch
    }
}
/// Frozen semantic input for a future server-owned request. It does not assert
/// that any request was sent or that a response/Connection/media is ready.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionInvitationMetadata {
    pub invitation_id: String,
    pub conversation_id: String,
    pub session_id: String,
    pub from_identity_id: String,
    pub from_participant_id: String,
    pub from_membership_epoch: u64,
    pub target_participant_id: String,
    pub target_membership_epoch: u64,
    pub medium: String,
    pub intent: String,
    pub capabilities_offer: Value,
    pub created_at: i64,
}
impl std::fmt::Debug for SessionInvitationMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionInvitationMetadata(redacted)")
    }
}
pub struct SessionInvitationRead {
    owner: Arc<InstallationOwnership>,
    _held: Arc<AuthorityGuardSet>,
    identity: String,
    credential: String,
    expires_at: i64,
    metadata: SessionInvitationMetadata,
}
impl std::fmt::Debug for SessionInvitationRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionInvitationRead(redacted)")
    }
}
impl SessionInvitationRead {
    pub fn metadata(&self) -> &SessionInvitationMetadata {
        &self.metadata
    }
    pub fn identity_id(&self) -> &str {
        &self.identity
    }
    pub fn credential_id(&self) -> &str {
        &self.credential
    }
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
    /// The reader fences current Store writers. Actual physical identity/key,
    /// lifetime, signature and reply matching remain the transport's obligation.
    pub fn current(&self) -> bool {
        self.owner.current() && crate::now() < self.expires_at
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn freeze(
    c: &Connection,
    store: &Store,
    actor: &Actor,
    r: &Request,
    from: &MemberView,
    targets: &[MemberView],
    medium: &str,
    intent: &str,
    capabilities: &Value,
) -> Result<()> {
    let tenant: String = c
        .query_row(
            "SELECT tenant_id FROM conversations WHERE id=?1",
            [&r.conversation_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let source = format!("sinv_{}", uuid::Uuid::new_v4().simple());
    c.execute("INSERT INTO session_invitation_sources(id,tenant_id,owner_id,operation_id,conversation_id,session_id,from_participant_id,from_epoch,medium,intent,capabilities_json,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![source,tenant,actor.identity_id,r.operation_id,r.conversation_id,r.session_id,from.participant_id,from.membership_epoch,medium,intent,crate::provisioning::canonical_json(capabilities)?,crate::now()]).map_err(|_|Error::storage())?;
    for target in targets {
        c.execute("INSERT INTO session_invitation_targets(source_id,participant_id,identity_id,membership_epoch) VALUES(?1,?2,?3,?4)",params![source,target.participant_id,target.identity_id,target.membership_epoch]).map_err(|_|Error::storage())?;
        c.execute("INSERT INTO session_invitation_slots(session_id,participant_id,membership_epoch,source_id) VALUES(?1,?2,?3,?4) ON CONFLICT(session_id,participant_id,membership_epoch) DO UPDATE SET source_id=excluded.source_id",params![r.session_id,target.participant_id,target.membership_epoch,source]).map_err(|_|Error::storage())?;
    }
    crate::session_gateways::freeze(c, store, &source, r, targets)?;
    Ok(())
}

fn metadata(
    c: &Connection,
    actor: &Actor,
    source: &SessionInvitationSource,
) -> Result<(SessionInvitationMetadata, Vec<AuthorityScope>)> {
    metadata_state(c, actor, source, false)
}
fn metadata_state(
    c: &Connection,
    actor: &Actor,
    source: &SessionInvitationSource,
    resolved: bool,
) -> Result<(SessionInvitationMetadata, Vec<AuthorityScope>)> {
    if actor.identity_id != source.identity
        || !(actor.permits("session_get")
            || actor.permits("session.accept")
            || actor.permits("session.reject"))
    {
        return Err(Error::forbidden());
    }
    // Pending slots are mutable ownership, while original sources/targets never
    // change. A newer invitation, terminal Session or changed epoch cannot be
    // adopted through a cached descriptor from a former invitation.
    let row:Option<(SessionInvitationMetadata,String)>=c.query_row(r#"
SELECT s.id,s.conversation_id,s.session_id,s.owner_id,s.from_participant_id,s.from_epoch,
 t.participant_id,t.membership_epoch,s.medium,s.intent,s.capabilities_json,s.created_at,s.tenant_id
FROM session_invitation_sources s
JOIN session_invitation_targets t ON t.source_id=s.id
JOIN sessions live ON live.id=s.session_id AND live.conversation_id=s.conversation_id
JOIN conversations room ON room.id=s.conversation_id AND room.tenant_id=s.tenant_id
JOIN participants original ON original.participant_id=s.from_participant_id AND original.identity_id=s.owner_id AND original.conversation_id=s.conversation_id AND original.tenant_id=s.tenant_id AND original.membership_epoch=s.from_epoch AND original.active=1
JOIN identities issuer ON issuer.id=original.identity_id AND issuer.tenant_id=s.tenant_id AND issuer.revoked=0
JOIN participants target ON target.participant_id=t.participant_id AND target.identity_id=t.identity_id AND target.conversation_id=s.conversation_id AND target.tenant_id=s.tenant_id AND target.membership_epoch=t.membership_epoch AND target.active=1
JOIN identities recipient ON recipient.id=target.identity_id AND recipient.tenant_id=s.tenant_id AND recipient.revoked=0
JOIN operations op ON op.tenant_id=s.tenant_id AND op.owner_id=s.owner_id AND op.operation_id=s.operation_id AND op.state='completed' AND op.command_type='session.invite'
JOIN operation_control_receipts receipt ON receipt.operation_record_id=op.record_id AND receipt.actor_participant_id=s.from_participant_id AND receipt.actor_membership_epoch=s.from_epoch AND receipt.actor_authority='member' AND receipt.task_id IS NULL AND receipt.grant_id IS NULL
WHERE s.id=?1 AND t.participant_id=?2 AND t.identity_id=?3 AND t.membership_epoch=?4
 AND (?5 OR (live.state NOT IN ('ended','rejected','cancelled','failed') AND room.closed=0
 AND EXISTS(SELECT 1 FROM session_invitation_slots slot JOIN session_participants presence ON presence.session_id=slot.session_id AND presence.participant_id=slot.participant_id AND presence.membership_epoch=slot.membership_epoch AND presence.state='invited'
 WHERE slot.session_id=s.session_id AND slot.participant_id=t.participant_id AND slot.membership_epoch=t.membership_epoch AND slot.source_id=s.id)))
"#,params![source.source,source.participant,source.identity,source.epoch,resolved],|r| {
    let text:String=r.get(10)?;
    let offer=serde_json::from_str(&text).map_err(|_|rusqlite::Error::InvalidQuery)?;
    Ok((SessionInvitationMetadata{invitation_id:r.get(0)?,conversation_id:r.get(1)?,session_id:r.get(2)?,from_identity_id:r.get(3)?,from_participant_id:r.get(4)?,from_membership_epoch:r.get(5)?,target_participant_id:r.get(6)?,target_membership_epoch:r.get(7)?,medium:r.get(8)?,intent:r.get(9)?,capabilities_offer:offer,created_at:r.get(11)?},r.get(12)?))
}).optional().map_err(|_|Error::storage())?;
    let (view, tenant) = row.ok_or_else(Error::forbidden)?;
    let actor_tenant: String = c
        .query_row(
            "SELECT tenant_id FROM identities WHERE id=?1",
            [&actor.identity_id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    if tenant != actor_tenant
        || !actor.permits_recipient(&view.from_identity_id)
        || !actor.permits_recipient(&source.identity)
    {
        return Err(Error::forbidden());
    }
    crate::audience::require_original(
        c,
        actor,
        &view.conversation_id,
        ContentKind::Session,
        &view.session_id,
    )?;
    let mut scopes = vec![
        AuthorityScope::conversation(&view.conversation_id, AuthorityMode::Read)?,
        AuthorityScope::identity(&actor.identity_id, AuthorityMode::Read)?,
        AuthorityScope::identity(&view.from_identity_id, AuthorityMode::Read)?,
    ];
    if let Some(owner) = crate::audience::source_owner(
        c,
        &view.conversation_id,
        ContentKind::Session,
        &view.session_id,
    )? {
        if !actor.permits_recipient(&owner) {
            return Err(Error::forbidden());
        }
        scopes.push(AuthorityScope::identity(&owner, AuthorityMode::Read)?);
    }
    Ok((view, AuthorityScope::merged(&scopes)?))
}
pub(crate) fn read_scopes(
    store: &Store,
    c: &Connection,
    actor: &Actor,
    source: &SessionInvitationSource,
) -> Result<Vec<AuthorityScope>> {
    if !source.owner.current() || source.runtime != store.runtime_epoch {
        return Err(Error::forbidden());
    }
    metadata(c, actor, source).map(|(_, scopes)| scopes)
}
impl Store {
    /// Protected immutable template construction only, never recipient/delivery
    /// authorization. Actual IO must use fresh exact peer/source read authority.
    pub fn session_invitation_template(
        &self,
        source: &SessionInvitationSource,
    ) -> Result<SessionInvitationMetadata> {
        self.require_runtime_owner()?;
        if !source.owner.current() || source.runtime != self.runtime_epoch {
            return Err(Error::forbidden());
        }
        let c = self.connection()?;
        c.query_row("SELECT s.id,s.conversation_id,s.session_id,s.owner_id,s.from_participant_id,s.from_epoch,t.participant_id,t.membership_epoch,s.medium,s.intent,s.capabilities_json,s.created_at FROM session_invitation_sources s JOIN session_invitation_targets t ON t.source_id=s.id WHERE s.id=?1 AND t.participant_id=?2 AND t.identity_id=?3 AND t.membership_epoch=?4",params![source.source,source.participant,source.identity,source.epoch],|r| {
            let text:String=r.get(10)?;let capabilities_offer=serde_json::from_str(&text).map_err(|_|rusqlite::Error::InvalidQuery)?;
            Ok(SessionInvitationMetadata{invitation_id:r.get(0)?,conversation_id:r.get(1)?,session_id:r.get(2)?,from_identity_id:r.get(3)?,from_participant_id:r.get(4)?,from_membership_epoch:r.get(5)?,target_participant_id:r.get(6)?,target_membership_epoch:r.get(7)?,medium:r.get(8)?,intent:r.get(9)?,capabilities_offer,created_at:r.get(11)?})
        }).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)
    }
    /// Metadata under the exact fresh owned peer/source read union. It retains
    /// no additional reader; keep the originating proof through actual IO.
    pub fn session_invitation_metadata_with_peer(
        &self,
        source: &SessionInvitationSource,
        credential: &str,
        peer: &crate::PeerAuthorityGuard,
    ) -> Result<SessionInvitationMetadata> {
        if !matches!(&peer.request.command,crate::PeerCommand::SessionInvitationRead(exact) if exact==source)
        {
            return Err(Error::forbidden());
        }
        let c = self.connection()?;
        self.require_peer_source(&c, peer, credential)?;
        let actor = crate::identity::actor_from_connection(&c, credential)?;
        let (view, scopes) = metadata(&c, &actor, source)?;
        if !self.authority_covers(Self::peer_scopes(peer), &scopes) {
            return Err(Error::forbidden());
        }
        Ok(view)
    }
    /// Protected selection only; actual metadata IO still requires fresh read
    /// authority. Identity, Device and response authority are never minted here.
    pub fn session_invitation_target(
        &self,
        invitation_id: &str,
        identity: &str,
    ) -> Result<SessionInvitationSource> {
        let owner = self.require_runtime_owner()?;
        let c = self.connection()?;
        let (participant, epoch) = c.query_row("SELECT participant_id,membership_epoch FROM session_invitation_targets WHERE source_id=?1 AND identity_id=?2", params![invitation_id,identity], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)?;
        Ok(SessionInvitationSource {
            owner,
            runtime: self.runtime_epoch.clone(),
            source: invitation_id.into(),
            participant,
            identity: identity.into(),
            epoch,
        })
    }
    /// Protected runtime enumeration from an already completed invitation
    /// receipt. This mints metadata only, with no retained authority reader.
    /// Task-scoped sources remain reserved for their G6-055 policy integration.
    pub fn session_invitation_sources(
        &self,
        record_id: &str,
    ) -> Result<Vec<SessionInvitationSource>> {
        let owner = self.require_runtime_owner()?;
        let c = self.connection()?;
        let mut q=c.prepare(r#"SELECT s.id,t.participant_id,t.identity_id,t.membership_epoch
FROM session_invitation_sources s JOIN session_invitation_targets t ON t.source_id=s.id
JOIN operations op ON op.tenant_id=s.tenant_id AND op.owner_id=s.owner_id AND op.operation_id=s.operation_id
JOIN operation_control_receipts receipt ON receipt.operation_record_id=op.record_id AND receipt.actor_participant_id=s.from_participant_id AND receipt.actor_membership_epoch=s.from_epoch AND receipt.actor_authority='member' AND receipt.task_id IS NULL AND receipt.grant_id IS NULL
WHERE op.record_id=?1 AND op.command_type='session.invite' AND op.state='completed' ORDER BY t.participant_id LIMIT 257"#).map_err(|_|Error::storage())?;
        let rows = q
            .query_map([record_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, u64>(3)?,
                ))
            })
            .map_err(|_| Error::storage())?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| Error::storage())?;
        if rows.len() > 256 {
            return Err(Error::new(
                "capacity-exceeded",
                "Invitation source capacity exceeded",
            ));
        }
        Ok(rows
            .into_iter()
            .map(
                |(source, participant, identity, epoch)| SessionInvitationSource {
                    owner: owner.clone(),
                    runtime: self.runtime_epoch.clone(),
                    source,
                    participant,
                    identity,
                    epoch,
                },
            )
            .collect())
    }
    /// Immediate fresh admission, suitable for a bounded actual metadata write.
    /// Never wait/think with this reader: drop it after IO and reacquire for the
    /// next phase. It cannot authorize a response mutation or media negotiation.
    pub fn try_session_invitation_read(
        &self,
        source: &SessionInvitationSource,
        credential: &str,
    ) -> Result<SessionInvitationRead> {
        if !source.owner.current() || source.runtime != self.runtime_epoch {
            return Err(Error::forbidden());
        }
        self.require_runtime_owner()?;
        let needed = {
            let c = self.connection()?;
            let actor = crate::identity::actor_from_connection(&c, credential)?;
            metadata(&c, &actor, source)?.1
        };
        let held = self.try_acquire_authority(&needed)?;
        let c = self.connection()?;
        let actor = crate::identity::actor_from_connection(&c, credential)?;
        let (view, current) = metadata(&c, &actor, source)?;
        if !self.authority_covers(&held, &current) || !source.owner.current() {
            return Err(Error::forbidden());
        }
        Ok(SessionInvitationRead {
            owner: source.owner.clone(),
            _held: held,
            identity: actor.identity_id,
            credential: actor.credential_id,
            expires_at: actor.expires_at,
            metadata: view,
        })
    }
}
