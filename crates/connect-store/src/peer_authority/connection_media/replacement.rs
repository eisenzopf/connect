//! Bounded current control for reserving a managed replacement, before target IO.
use super::*;
use crate::{NativeReplacementContext, NativeReplacementOrigin};

/// Exact physical origin of the connection being replaced. Telephone origins
/// are independent established SIP lifetimes, never native peer admissions.
#[derive(Clone)]
pub enum NativeReplacementSourceOrigin {
    Native(Arc<NativeMediaOrigin>),
    Telephone(Arc<crate::TelephoneMediaOrigin>),
}
impl NativeReplacementSourceOrigin {
    pub fn connection_id(&self) -> &str {
        match self {
            Self::Native(n) => n.connection_id(),
            Self::Telephone(p) => &p.snapshot().connection_id,
        }
    }
    pub fn runtime_connection_id(&self) -> &str {
        match self {
            Self::Native(n) => n.runtime_connection_id(),
            Self::Telephone(p) => &p.snapshot().runtime_connection_id,
        }
    }
    pub fn conversation_id(&self) -> &str {
        match self {
            Self::Native(n) => n.conversation_id(),
            Self::Telephone(p) => &p.snapshot().conversation_id,
        }
    }
    pub fn session_id(&self) -> &str {
        match self {
            Self::Native(n) => n.session_id(),
            Self::Telephone(p) => &p.snapshot().session_id,
        }
    }
    pub fn participant_id(&self) -> &str {
        match self {
            Self::Native(n) => n.participant_id(),
            Self::Telephone(p) => &p.snapshot().participant_id,
        }
    }
    pub fn membership_epoch(&self) -> u64 {
        match self {
            Self::Native(n) => n.membership_epoch(),
            Self::Telephone(p) => p.snapshot().membership_epoch,
        }
    }
    pub fn generation(&self) -> u64 {
        match self {
            Self::Native(n) => n.generation(),
            Self::Telephone(p) => p.snapshot().generation,
        }
    }
    fn physical_expiry(&self) -> i64 {
        match self {
            Self::Native(n) => n.expires_at,
            Self::Telephone(_) => i64::MAX,
        }
    }
    fn snapshot(&self) -> serde_json::Value {
        match self {
            Self::Native(n) => returns::describe(n),
            Self::Telephone(p) => {
                serde_json::json!({"kind":"telephone","operation_record_id":p.record_id(),"snapshot":p.snapshot()})
            }
        }
    }
}
/// Current source authority. The common Session projection contains only
/// logical membership facts; physical IO always dispatches to its real variant.
pub enum NativeReplacementSourceRead {
    Native(NativeMediaRead),
    Telephone(crate::TelephoneMediaRead, PeerSessionProjection),
}
impl NativeReplacementSourceRead {
    pub fn native(&self) -> Option<&NativeMediaRead> {
        match self {
            Self::Native(n) => Some(n),
            _ => None,
        }
    }
    pub fn telephone(&self) -> Option<&crate::TelephoneMediaRead> {
        match self {
            Self::Telephone(p, _) => Some(p),
            _ => None,
        }
    }
    pub fn session(&self) -> &PeerSessionProjection {
        match self {
            Self::Native(n) => &n.target().session,
            Self::Telephone(_, v) => v,
        }
    }
    pub fn accepted_capabilities(&self) -> &serde_json::Value {
        match self {
            Self::Native(n) => &n.target().accepted_capabilities,
            Self::Telephone(p, _) => p.accepted_capabilities(),
        }
    }
    pub fn current(&self) -> bool {
        match self {
            Self::Native(n) => n.current(),
            Self::Telephone(p, _) => p.current(),
        }
    }
    pub fn expires_at(&self) -> i64 {
        match self {
            Self::Native(n) => n.expires_at(),
            Self::Telephone(p, _) => p.expires_at(),
        }
    }
}
/// Owns source/retained physical grants and the original operator interval.
/// This does not grant a candidate route, readiness, callback or final commit.
pub struct NativeReplacementPreparationRead {
    selection: NativeReplacementSelection,
    context: NativeReplacementContext,
    caller: PeerSessionProjection,
    source: NativeReplacementSourceRead,
    retained: NativeMediaRead,
    expires_at: i64,
}
impl NativeReplacementPreparationRead {
    pub(crate) fn selection(&self) -> &NativeReplacementSelection {
        &self.selection
    }
    pub(crate) fn into_selection(self) -> Result<NativeReplacementSelection> {
        if !self.current() {
            return Err(Error::forbidden());
        }
        Ok(self.selection)
    }
    pub(crate) fn limit(&mut self, expires: i64) -> Result<()> {
        self.expires_at = self.expires_at.min(expires);
        if self.current() {
            Ok(())
        } else {
            Err(Error::forbidden())
        }
    }
    pub fn context(&self) -> &NativeReplacementContext {
        &self.context
    }
    pub fn caller(&self) -> &PeerSessionProjection {
        &self.caller
    }
    pub fn source(&self) -> &NativeReplacementSourceRead {
        &self.source
    }
    pub fn retained(&self) -> &NativeMediaRead {
        &self.retained
    }
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
    pub fn current(&self) -> bool {
        crate::now() < self.expires_at && self.source.current() && self.retained.current()
    }
}
/// Immutable source pins, never an owned authority interval.
#[derive(Clone)]
pub struct NativeReplacementSelection {
    pub(crate) origin: NativeReplacementOrigin,
    source: NativeReplacementSourceOrigin,
    retained: Arc<NativeMediaOrigin>,
}
impl NativeReplacementSelection {
    pub(crate) fn source(&self) -> &NativeReplacementSourceOrigin {
        &self.source
    }
    pub(crate) fn physical_expiry(&self) -> i64 {
        self.source.physical_expiry().min(self.retained.expires_at)
    }
    pub(crate) fn read(&self, store: &Store) -> Result<NativeReplacementPreparationRead> {
        store.try_native_replacement_preparation_read(&self.origin, &self.source, &self.retained)
    }
    pub(crate) fn read_with_peer(
        &self,
        store: &Store,
        proof: &PeerAuthorityGuard,
    ) -> Result<NativeReplacementPreparationRead> {
        if !proof.current() {
            return Err(Error::forbidden());
        }
        self.read_with_held(store, proof._scopes.clone(), proof.expires_at())
    }
    pub(crate) fn read_with_held(
        &self,
        store: &Store,
        held: Arc<AuthorityGuardSet>,
        expires: i64,
    ) -> Result<NativeReplacementPreparationRead> {
        let c = store.try_connection()?;
        self.read_with_connection(store, &c, held, expires)
    }
    pub(crate) fn read_with_connection(
        &self,
        store: &Store,
        c: &Connection,
        held: Arc<AuthorityGuardSet>,
        expires: i64,
    ) -> Result<NativeReplacementPreparationRead> {
        let resolved = resolve(store, c, &self.origin, &self.source, &self.retained)?;
        if !store.authority_covers(&held, &resolved.scopes) {
            return Err(Error::forbidden());
        }
        let mut read = make_read(store, c, self.clone(), resolved, held)?;
        read.limit(expires)?;
        Ok(read)
    }
    pub(crate) fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({"source":self.source.snapshot(),"retained":returns::describe(&self.retained)})
    }
    pub(crate) fn scopes(
        &self,
        store: &Store,
        c: &Connection,
        write: bool,
    ) -> Result<Vec<AuthorityScope>> {
        let mut resolved = resolve(store, c, &self.origin, &self.source, &self.retained)?;
        if write {
            resolved.scopes.push(AuthorityScope::identity(
                &resolved.caller.actor.identity_id,
                AuthorityMode::Write,
            )?);
            resolved
                .scopes
                .extend(crate::authority_admission::conversation_scopes(
                    c,
                    &resolved.context.conversation_id,
                    AuthorityMode::Write,
                )?);
        }
        AuthorityScope::merged(&resolved.scopes)
    }
    pub(crate) fn context(
        &self,
        store: &Store,
        c: &Connection,
    ) -> Result<NativeReplacementContext> {
        Ok(resolve(store, c, &self.origin, &self.source, &self.retained)?.context)
    }
}
struct Resolved {
    context: NativeReplacementContext,
    caller: PeerSessionProjection,
    source: Option<PeerConnectionProjection>,
    retained: PeerConnectionProjection,
    scopes: Vec<AuthorityScope>,
    expires_at: i64,
    subscriptions: [bool; 2],
}
fn resolve(
    store: &Store,
    c: &Connection,
    origin: &NativeReplacementOrigin,
    old: &NativeReplacementSourceOrigin,
    remote: &NativeMediaOrigin,
) -> Result<Resolved> {
    let actor = origin.require(store, c)?;
    let context = origin.context(store, c)?;
    if context.target_transport == connect_protocol::provisioning::EndpointTransport::Telephone {
        actor.require("sip_dial")?;
        let owner_current: bool = c
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM identities WHERE id=?1 AND revoked=0)",
                [&context.target_route_owner_id],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if !owner_current {
            return Err(Error::forbidden());
        }
    }
    if old.connection_id() != context.source_connection_id
        || old.runtime_connection_id() != context.source_runtime_connection_id
        || old.generation() != context.source_generation
        || old.participant_id() != context.participant_id
        || old.membership_epoch() != context.membership_epoch
        || remote.connection_id() != context.retained_connection_id
        || remote.runtime_connection_id() != context.retained_runtime_connection_id
        || remote.generation() != context.retained_generation
        || old.session_id() != context.session_id
        || old.conversation_id() != context.conversation_id
        || remote.session_id() != context.session_id
        || remote.conversation_id() != context.conversation_id
    {
        return Err(Error::forbidden());
    }
    let (source, mut scopes, old_expiry, old_subscribe) = match old {
        NativeReplacementSourceOrigin::Native(n) => {
            let (view, scopes, expires) = facts(store, c, n)?;
            (Some(view), scopes, expires, subscription_permission(c, n)?)
        }
        NativeReplacementSourceOrigin::Telephone(p) => (
            None,
            store.telephone_established_scopes(c, p)?,
            i64::MAX,
            false,
        ),
    };
    let (retained, remote_scopes, remote_expiry) = facts(store, c, remote)?;
    scopes.extend(remote_scopes);
    scopes.push(AuthorityScope::identity(
        &actor.identity_id,
        AuthorityMode::Read,
    )?);
    if context.target_transport == connect_protocol::provisioning::EndpointTransport::Telephone {
        scopes.push(AuthorityScope::identity(
            &context.target_route_owner_id,
            AuthorityMode::Read,
        )?);
    }
    scopes.extend(crate::authority_admission::conversation_scopes(
        c,
        &context.conversation_id,
        AuthorityMode::Read,
    )?);
    let member = crate::audience::member(c, &actor.identity_id, &context.conversation_id, true)?;
    if !actor.permits_recipient(&context.identity_id)
        || !actor.permits_recipient(&retained.session.actor.identity_id)
    {
        return Err(Error::forbidden());
    }
    let mut caller = retained.session.clone();
    caller.present=c.query_row("SELECT EXISTS(SELECT 1 FROM session_participants WHERE session_id=?1 AND participant_id=?2 AND membership_epoch=?3 AND state='present')",params![context.session_id,member.participant_id,member.membership_epoch],|r|r.get(0)).map_err(|_|Error::storage())?;
    caller.actor = member;
    let expires_at = (crate::now() + 5)
        .min(actor.expires_at)
        .min(origin.request.expires_at)
        .min(old_expiry)
        .min(remote_expiry);
    Ok(Resolved {
        context,
        caller,
        source,
        retained,
        scopes: AuthorityScope::merged(&scopes)?,
        expires_at,
        subscriptions: [old_subscribe, subscription_permission(c, remote)?],
    })
}
impl Store {
    /// Acquire and resolve the exact original source/retained incarnations and
    /// provisioned target under one merged interval. Never nest a second reader
    /// behind a waiting writer or retain this interval in the prepared ticket.
    pub fn try_native_replacement_preparation_read(
        &self,
        origin: &NativeReplacementOrigin,
        source: &NativeReplacementSourceOrigin,
        retained: &Arc<NativeMediaOrigin>,
    ) -> Result<NativeReplacementPreparationRead> {
        let scopes = {
            let c = self.try_connection()?;
            resolve(self, &c, origin, source, retained)?.scopes
        };
        let held = self.try_acquire_authority(&scopes)?;
        let c = self.try_connection()?;
        let resolved = resolve(self, &c, origin, source, retained)?;
        if !self.authority_covers(&held, &resolved.scopes) || crate::now() >= resolved.expires_at {
            return Err(Error::forbidden());
        }
        make_read(
            self,
            &c,
            NativeReplacementSelection {
                origin: origin.clone(),
                source: source.clone(),
                retained: retained.clone(),
            },
            resolved,
            held,
        )
    }
}

fn make_read(
    store: &Store,
    c: &Connection,
    selection: NativeReplacementSelection,
    resolved: Resolved,
    held: Arc<AuthorityGuardSet>,
) -> Result<NativeReplacementPreparationRead> {
    let owner = store.require_runtime_owner()?;
    let make = |target, may_subscribe| NativeMediaRead {
        target,
        source: None,
        _scopes: held.clone(),
        owner: owner.clone(),
        expires_at: resolved.expires_at,
        may_subscribe,
    };
    let source = match &selection.source {
        NativeReplacementSourceOrigin::Native(_) => NativeReplacementSourceRead::Native(make(
            resolved.source.ok_or_else(Error::forbidden)?,
            resolved.subscriptions[0],
        )),
        NativeReplacementSourceOrigin::Telephone(p) => {
            let phone = store.telephone_media_established_in(
                c,
                p.clone(),
                held.clone(),
                resolved.expires_at,
            )?;
            if !phone
                .route_actor()
                .permits_recipient(&resolved.retained.session.actor.identity_id)
            {
                return Err(Error::forbidden());
            }
            let session = PeerSessionProjection {
                session: phone.session().clone(),
                actor: phone.member().clone(),
                conversation_revision: phone.conversation_revision(),
                present: true,
            };
            NativeReplacementSourceRead::Telephone(phone, session)
        }
    };
    let expires_at = resolved.expires_at.min(source.expires_at());
    Ok(NativeReplacementPreparationRead {
        selection,
        context: resolved.context,
        caller: resolved.caller,
        source,
        retained: make(resolved.retained, resolved.subscriptions[1]),
        expires_at,
    })
}
