//! Current exact message authority; historical snapshots never grant delivery.
use super::*;
use connect_protocol::{
    membership::ContentKind,
    native_streams::{Command, Intent},
};

pub struct NativeDataRead {
    media: NativeMediaRead,
    expires_at: i64,
}
impl std::fmt::Debug for NativeDataRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeDataRead(redacted)")
    }
}
impl NativeDataRead {
    pub fn media(&self) -> &NativeMediaRead {
        &self.media
    }
    pub fn current(&self) -> bool {
        self.media.current() && crate::now() < self.expires_at
    }
}
fn facts(
    store: &Store,
    c: &Connection,
    target: &NativeMediaOrigin,
    publisher: &NativeMediaOrigin,
    receiving: bool,
    wire: &Intent,
) -> Result<(Vec<AuthorityScope>, i64)> {
    if wire.command != Command::Data
        || wire.conversation_id != publisher.conversation_id()
        || wire.session_id != publisher.session_id()
        || wire.connection_id != publisher.connection_id()
    {
        return Err(Error::forbidden());
    }
    let credential = publisher
        .binding
        .credentials
        .last()
        .ok_or_else(Error::forbidden)?;
    let actor = crate::identity::actor_from_connection(c, credential)?;
    let (source, _) = super::super::native_streams::projection(
        store,
        c,
        &actor,
        &publisher.binding.device.id,
        &publisher.binding.key_id,
        wire,
    )?;
    if source.parent_record_id != publisher.attempt.record_id()
        || source.connection.runtime_connection_id != publisher.runtime_connection_id()
    {
        return Err(Error::forbidden());
    }
    // Source profile, complete intent digest and frozen original addresses.
    // A message that has never begun publication is not a queued source.
    let members = super::super::native_data::recipients(c, &actor, wire, &source)?;
    let expires: Option<i64> = c.query_row("SELECT o.expires_at FROM native_data_messages d JOIN operations o ON o.record_id=d.operation_record_id JOIN native_stream_operations n ON n.operation_record_id=o.record_id JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=o.record_id WHERE o.owner_id=?1 AND o.operation_id=?2 AND o.connection_id=?3 AND o.conversation_id=?4 AND o.session_id=?5 AND n.parent_record_id=?6 AND n.generation=?7 AND n.attempt_runtime=?8 AND n.effect_started=1 AND e.state IN ('submitting','confirmed','unknown') AND o.state IN ('preparing','committing','completed','unknown') AND o.cancel_requested=0 AND o.expires_at>?9",params![actor.identity_id,wire.operation_id,wire.connection_id,wire.conversation_id,wire.session_id,publisher.attempt.record_id(),publisher.generation(),store.runtime_epoch(),crate::now()], |r|r.get(0)).optional().map_err(|_|Error::storage())?;
    let expires = expires.ok_or_else(|| {
        Error::new(
            "not-recipient",
            "Original native publication is cancelled, expired or unavailable",
        )
    })?;
    let message = connect_protocol::native_data::message(&wire.payload)?;
    crate::audience::require_original(
        c,
        &actor,
        &wire.conversation_id,
        ContentKind::Message,
        &message.message_id,
    )?;
    if receiving {
        let target_actor = crate::identity::actor_from_connection(
            c,
            target
                .binding
                .credentials
                .last()
                .ok_or_else(Error::forbidden)?,
        )?;
        target_actor.require("history")?;
        target_actor.require("stream.subscribe")?;
        if !members.iter().any(|m| {
            m.participant_id == target.participant_id()
                && m.membership_epoch == target.membership_epoch()
                && m.identity_id == target_actor.identity_id
        }) {
            return Err(Error::new(
                "not-recipient",
                "Target is outside the original native message audience",
            ));
        }
        crate::audience::require_original(
            c,
            &target_actor,
            &wire.conversation_id,
            ContentKind::Message,
            &message.message_id,
        )?;
    }
    let scopes = super::super::native_streams::scopes(
        store,
        c,
        &actor,
        &publisher.binding.device.id,
        &publisher.binding.key_id,
        wire,
    )?;
    Ok((scopes, expires))
}
fn prepared(
    store: &Store,
    c: &Connection,
    target: &NativeMediaOrigin,
    source: Option<&NativeMediaOrigin>,
    wire: &Intent,
) -> Result<(
    PeerConnectionProjection,
    Option<PeerConnectionProjection>,
    Vec<AuthorityScope>,
    i64,
)> {
    let (target_view, source_view, mut scopes, expiry) = views(store, c, target, source)?;
    if !matches!(target_view.connection.state.as_str(), "ready" | "active")
        || !super::super::native_data::data_floor(&target_view)
    {
        return Err(Error::forbidden());
    }
    let (data_scopes, data_expiry) = facts(
        store,
        c,
        target,
        source.unwrap_or(target),
        source.is_some(),
        wire,
    )?;
    scopes.extend(data_scopes);
    Ok((
        target_view,
        source_view,
        AuthorityScope::merged(&scopes)?,
        expiry.min(data_expiry),
    ))
}
impl Store {
    /// Initial source capture, outside engine fences, using only the exact
    /// original signed command's readers and its one marked publication attempt.
    pub fn native_data_read_with_peer(
        &self,
        source: &NativeMediaOrigin,
        proof: &PeerAuthorityGuard,
        lease: &crate::NativeStreamLease,
    ) -> Result<NativeDataRead> {
        let PeerCommand::NativeStream(wire) = &proof.request.command else {
            return Err(Error::forbidden());
        };
        if wire.command != Command::Data {
            return Err(Error::forbidden());
        }
        let media = self.native_stream_media_read_with_peer(source, None, proof, lease)?;
        let c = self.try_connection()?;
        let (scopes, expires_at) = facts(self, &c, source, source, false, wire)?;
        if !self.authority_covers(&proof._scopes, &scopes) {
            return Err(Error::forbidden());
        }
        Ok(NativeDataRead {
            media,
            expires_at: expires_at.min(lease.expires_at()).min(proof.expires_at()),
        })
    }
    /// Fresh bounded queued-message delivery. Never reenter this path while
    /// retaining an initial command read; the engine supplies sealed facts only.
    pub fn try_native_data_read(
        &self,
        target: &NativeMediaOrigin,
        source: &NativeMediaOrigin,
        wire: &Intent,
    ) -> Result<NativeDataRead> {
        self.try_native_data_at(target, Some(source), wire)
    }
    pub fn try_native_data_publish_read(
        &self,
        source: &NativeMediaOrigin,
        wire: &Intent,
    ) -> Result<NativeDataRead> {
        self.try_native_data_at(source, None, wire)
    }
    fn try_native_data_at(
        &self,
        target: &NativeMediaOrigin,
        source: Option<&NativeMediaOrigin>,
        wire: &Intent,
    ) -> Result<NativeDataRead> {
        // Complete message/audience projections are expensive. Capture once,
        // release SQL for fresh file admission, then seal the exact SQL/clock
        // version under that interval. A changed version refuses rather than
        // lending historical message facts to a queued delivery. Clock edges
        // rederive with the already-owned complete interval.
        let ((target_view, source_view, expires_at), held) = snapshot::acquire(self, |c| {
            let (target, source, scopes, expires) = prepared(self, c, target, source, wire)?;
            Ok(((target, source, expires), scopes))
        })?;
        if crate::now() >= expires_at {
            return Err(Error::forbidden());
        }
        let owner = current_owner(self, target)?;
        let media = NativeMediaRead {
            target: target_view,
            source: source_view,
            _scopes: held,
            owner,
            expires_at,
            may_subscribe: true,
        };
        Ok(NativeDataRead { media, expires_at })
    }
}
