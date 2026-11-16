//! Original public-video setup/phase journal. No row authorizes media or native IO.
use super::*;
use crate::{AuthorityGuardSet, AuthorityMode, AuthorityScope};
use connect_protocol::{
    webrtc_public_video_phase as wire, webrtc_video_receive_reservation as reservation,
};
use std::sync::Arc;
mod atomic_prepare;
use atomic_prepare::reserve_intent_in;

/// Immutable original setup metadata. There is no public constructor or grant.
pub struct NativeReplacementPublicVideoReservation {
    original: Arc<NativeReplacementPrepared>,
    intent: reservation::Intent,
    saved: serde_json::Value,
    expires_at: i64,
}
impl NativeReplacementPublicVideoReservation {
    pub fn intent(&self) -> &reservation::Intent {
        &self.intent
    }
    pub fn operation_record_id(&self) -> &str {
        &self.original.record
    }
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
}
/// Only the specialized authorized operation projection may create this view.
pub struct PublicVideoOperationSnapshot {
    pub operation: OperationRecord,
    pub intent: Option<reservation::Intent>,
    pub phase: Option<wire::Phase>,
}
/// Cleanup-only historical custody. It neither renews expired target authority
/// nor authorizes publication, media, readiness or restoration of OLD.
pub struct NativeReplacementUnpublishedRead {
    original: Arc<NativeReplacementPrepared>,
    saved: serde_json::Value,
    held: Arc<AuthorityGuardSet>,
    wait: crate::AuthorityWait,
    actor_identity: String,
    original_expiry: i64,
}
impl NativeReplacementUnpublishedRead {
    pub fn current(&self) -> bool {
        self.wait.check().is_ok() && self.original.owner.upgrade().is_some_and(|o| o.current())
    }
    pub fn actor_identity_id(&self) -> &str {
        &self.actor_identity
    }
    /// Historical target cutoff only; cleanup currentness uses the supplied wait.
    pub fn original_expires_at(&self) -> i64 {
        self.original_expiry
    }
    pub fn operation_record_id(&self) -> &str {
        &self.original.record
    }
    pub fn candidate_connection_id(&self) -> &str {
        self.saved["candidate_connection_id"]
            .as_str()
            .expect("validated candidate")
    }
    pub fn candidate_runtime_connection_id(&self) -> &str {
        self.saved["candidate_runtime_id"]
            .as_str()
            .expect("validated runtime")
    }
    pub fn conversation_id(&self) -> &str {
        &self.original.selection.origin.request().conversation_id
    }
    pub fn session_id(&self) -> &str {
        &self.original.selection.origin.request().session_id
    }
    pub fn participant_id(&self) -> &str {
        self.saved["expected_preparation"]["participant"]
            .as_str()
            .expect("validated participant")
    }
    pub fn target_endpoint_id(&self) -> &str {
        self.saved["expected_preparation"]["endpoint"]
            .as_str()
            .expect("validated endpoint")
    }
    pub fn target_device_id(&self) -> &str {
        self.saved["expected_preparation"]["device"]
            .as_str()
            .expect("validated device")
    }
}
redacted!(
    NativeReplacementPublicVideoReservation,
    NativeReplacementUnpublishedRead,
    PublicVideoOperationSnapshot
);

fn canonical(v: &serde_json::Value) -> Result<String> {
    crate::provisioning::canonical_json(v)
}
fn string<'a>(v: &'a serde_json::Value, key: &str) -> Result<&'a str> {
    v[key].as_str().ok_or_else(Error::storage)
}
fn original_read_scopes(
    store: &Store,
    c: &Connection,
    p: &NativeReplacementPrepared,
) -> Result<Vec<AuthorityScope>> {
    let mut scopes = p.selection.scopes(store, c, false)?;
    scopes.push(AuthorityScope::record(&p.record, AuthorityMode::Read)?);
    AuthorityScope::merged(&scopes)
}
fn mapping(saved: &serde_json::Value) -> Result<(String, String)> {
    let video: connect_protocol::replacement_video::ReplacementVideo =
        serde_json::from_value(saved["video"].clone()).map_err(|_| unsupported())?;
    video.validate()?;
    if video.publications.len() != 1 || video.receivers.len() != 1 {
        return Err(unsupported());
    }
    let a = &video.publications[0].replacement;
    let b = &video.receivers[0].replacement;
    if a == b {
        return Err(unsupported());
    }
    Ok((a.clone(), b.clone()))
}
fn unsupported() -> Error {
    Error::new(
        "unsupported-media-profile",
        "Public video reservation requires one original A publication and B receiver mapping",
    )
}
fn public_sources(
    store: &Store,
    c: &Connection,
    read: &crate::NativeReplacementPreparationRead,
) -> Result<bool> {
    let Some(source) = read.source().native() else {
        return Ok(false);
    };
    if [source.target(), read.retained().target()]
        .into_iter()
        .any(|view| view.private_consultation.is_some() || view.handoff_preparation)
    {
        return Ok(false);
    }
    // An established native/native primary deliberately retains its ordinary
    // media projection. Its authoritative private floor/partner is therefore
    // a separate cold metadata fact, never inferred from that projection.
    for id in [
        source.target().connection.connection_id.as_str(),
        read.retained().target().connection.connection_id.as_str(),
    ] {
        if crate::connection_control::private_consultation::for_connection(
            store,
            c,
            id,
            crate::connection_control::private_consultation::Purpose::Media,
        )?
        .is_some()
            || crate::connection_control::private_consultation::private_partner(c, id)?.is_some()
        {
            return Ok(false);
        }
    }
    Ok(true)
}
fn require_public_sources(
    store: &Store,
    c: &Connection,
    read: &crate::NativeReplacementPreparationRead,
) -> Result<()> {
    if public_sources(store, c, read)? {
        return Ok(());
    }
    Err(Error::new(
        "unsupported-media-profile",
        "Public video reservation requires ordinary public original sources",
    ))
}
fn before_offer(c: &Connection, p: &NativeReplacementPrepared) -> Result<()> {
    let advanced:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_steps WHERE operation_record_id=?1 AND phase IN ('allocate','start','commit'))",[&p.record],|r|r.get(0)).map_err(|_|Error::storage())?;
    if advanced {
        Err(conflict())
    } else {
        Ok(())
    }
}
fn stored_intent(
    c: &Connection,
    p: &NativeReplacementPrepared,
    saved: &serde_json::Value,
) -> Result<Option<(reservation::Intent, i64)>> {
    let old:Option<(String,i64)>=c.query_row("SELECT intent_json,expires_at FROM native_replacement_public_video_reservations WHERE operation_record_id=?1 AND parent_reservation_id=?2 AND runtime_epoch=?3 AND plan_digest=?4 AND candidate_connection_id=?5 AND candidate_runtime_id=?6 AND target_generation=?7",params![p.record,p.reservation,p.runtime,p.hash,saved["candidate_connection_id"].as_str(),saved["candidate_runtime_id"].as_str(),saved["generation"].as_u64()],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?;
    old.map(|(raw, expiry)| {
        let i: reservation::Intent = serde_json::from_str(&raw).map_err(|_| Error::storage())?;
        let (a, b) = mapping(saved)?;
        i.validate_mapping(&a, &b).map_err(|_| Error::storage())?;
        Ok((i, expiry))
    })
    .transpose()
}
fn terminal(c: &Connection, record: &str) -> Result<bool> {
    c.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_public_video_terminals WHERE operation_record_id=?1)",[record],|r|r.get(0)).map_err(|_|Error::storage())
}
impl Store {
    /// Retained kernel controls only. Production opt-in confirms Prepare and
    /// this intent together through the atomic named API; late opt-in is absent.
    #[cfg(test)]
    pub fn reserve_native_replacement_public_video_intent(
        &self,
        p: Arc<NativeReplacementPrepared>,
    ) -> Result<NativeReplacementPublicVideoReservation> {
        // Test-only retained kernel controls include deliberately private/full
        // scenes. Their cold footprint derivation need not fit production's
        // five-millisecond captured-metadata cadence. Use the SAME existing
        // bounded preparation SQL/Read custody; no authority survives an SQL
        // wait, and reserve_intent_in rechecks every exact current scope/fact.
        // This function is absent from production; its budgets/APIs are intact.
        let wait = crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT);
        let (held, mut c) = self.native_replacement_preparation_sql(
            &wait,
            false,
            "test-public-video-authority",
            "test-public-video-sql",
            |c| {
                allocation::current_prepared(self, c, &p)?;
                original_read_scopes(self, c, &p)
            },
        )?;
        c.bind_available_mutation_authority(self, &held)?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let result = reserve_intent_in(self, &tx, &held, p)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
    /// Exact current operation.get authorization, with phase projection kept OUT
    /// of generic rows::load and packet/authority row construction.
    pub fn native_replacement_public_video_snapshot(
        &self,
        credential: &str,
        claim: crate::PeerOperationClaim,
    ) -> Result<PublicVideoOperationSnapshot> {
        if !matches!(&claim.action, OperationAction::Get { .. }) {
            return Err(Error::forbidden());
        }
        // Initial SQL may wait only before acquiring the compatible Read.
        // Revalidation/final projection never queues on SQL while holding it.
        let (held, c) = self.try_replacement_metadata_projection(|c| {
            let actor = actor_from_connection(c, credential)?;
            super::super::peer::operation_source_scopes(c, &actor, &claim, true)
        })?;
        snapshot(self, &c, credential, &claim, &held)
    }
    /// Reuse the SAME actual signed operation.get Read. Never reacquire a
    /// nested Reader behind a queued Writer while the original peer owns one.
    pub fn native_replacement_public_video_snapshot_with_peer(
        &self,
        credential: &str,
        claim: &crate::PeerOperationClaim,
        peer: &crate::PeerAuthorityGuard,
    ) -> Result<PublicVideoOperationSnapshot> {
        if !matches!(&claim.action, OperationAction::Get { .. }) {
            return Err(Error::forbidden());
        }
        let c = self.try_connection()?;
        self.require_peer_operation(&c, peer, credential, &claim.action)?;
        let result = snapshot(self, &c, credential, claim, Self::peer_scopes(peer))?;
        self.require_peer_source(&c, peer, credential)?;
        Ok(result)
    }
    /// The original owned reservation can be cleaned after expiry/cancel/revoke.
    /// Fresh compatible Record+scene Readers exclude Commit Writer. This does
    /// not use current_prepared or renew any target media/old-source permission.
    pub fn native_replacement_known_unpublished_read(
        &self,
        p: Arc<NativeReplacementPrepared>,
        wait: &crate::AuthorityWait,
    ) -> Result<NativeReplacementUnpublishedRead> {
        wait.check()?;
        let (saved, scopes) = {
            let c = self.try_connection_with_wait(wait)?;
            historical(self, &c, &p)?
        };
        let held = self.acquire_authority(&scopes, wait)?;
        // A queued SQL owner cannot extend a retained cleanup Reader.
        let c = self.try_connection()?;
        let (actual, current) = historical(self, &c, &p)?;
        if actual != saved || !self.authority_covers(&held, &current) {
            return Err(Error::forbidden());
        }
        wait.check()?;
        let (actor_identity,original_expiry):(String,i64)=c.query_row("SELECT o.owner_id,n.expires_at FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id WHERE n.operation_record_id=?1",[&p.record],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::storage())?;
        Ok(NativeReplacementUnpublishedRead {
            original: p,
            saved,
            held,
            wait: wait.clone(),
            actor_identity,
            original_expiry,
        })
    }
}

fn snapshot(
    store: &Store,
    c: &Connection,
    credential: &str,
    claim: &crate::PeerOperationClaim,
    held: &Arc<AuthorityGuardSet>,
) -> Result<PublicVideoOperationSnapshot> {
    let OperationAction::Get { operation_id } = &claim.action else {
        return Err(Error::forbidden());
    };
    let actor = actor_from_connection(c, credential)?;
    if !store.authority_covers(
        held,
        &super::super::peer::operation_source_scopes(c, &actor, claim, true)?,
    ) {
        return Err(Error::forbidden());
    }
    let row = rows::find(
        c,
        &authority::tenant(c, &actor)?,
        &actor.identity_id,
        operation_id,
    )?
    .ok_or_else(Error::forbidden)?;
    authority::read(c, &actor, &row)?;
    if row.tombstoned || row.record.command_type != "connection.replace" {
        return Err(Error::forbidden());
    }
    let (intent, phase) = project(c, &row.record)?;
    let operation = rows::view(c, &actor, row)?;
    actor_from_connection(c, credential)?;
    Ok(PublicVideoOperationSnapshot {
        operation,
        intent,
        phase,
    })
}

/// The signed original target offer must echo the stored intent exactly. No
/// ledger row means no optional intent is accepted. Existing non-opt-in offers
/// remain identical; no downgrade after an original reservation exists.
pub(super) fn check_offer(
    c: &Connection,
    p: &NativeReplacementPrepared,
    saved: &serde_json::Value,
    offer: &serde_json::Value,
) -> Result<()> {
    let old = stored_intent(c, p, saved)?;
    match (old, offer.get("webrtc_video_receive_reservation")) {
        (None, None) => Ok(()),
        (Some((original, _)), Some(value)) if !terminal(c, &p.record)? => {
            let actual: reservation::Intent =
                serde_json::from_value(value.clone()).map_err(|_| invalid())?;
            actual.validate()?;
            if actual == original {
                Ok(())
            } else {
                Err(conflict())
            }
        }
        _ => Err(conflict()),
    }
}
fn never_public_candidate(
    c: &Connection,
    p: &NativeReplacementPrepared,
    saved: &serde_json::Value,
) -> Result<()> {
    let target = string(saved, "candidate_connection_id")?;
    let conflict:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM connections WHERE id=?1) OR EXISTS(SELECT 1 FROM native_connection_lifetimes WHERE connection_id=?1) OR EXISTS(SELECT 1 FROM native_replacement_lifetimes l JOIN native_replacements n ON n.operation_record_id=l.operation_record_id WHERE n.candidate_connection_id=?1) OR EXISTS(SELECT 1 FROM native_replacement_audiences a JOIN native_replacements n ON n.operation_record_id=a.operation_record_id WHERE n.candidate_connection_id=?1 AND a.phase='commit') OR EXISTS(SELECT 1 FROM native_replacement_candidates q WHERE q.candidate_connection_id=?1 AND q.operation_record_id!=?2) OR EXISTS(SELECT 1 FROM native_offer_operations v JOIN operations o ON o.record_id=v.operation_record_id WHERE o.connection_id=?1)",params![target,p.record],|row|row.get(0)).map_err(|_|Error::storage())?;
    if conflict {
        Err(Error::forbidden())
    } else {
        Ok(())
    }
}
fn historical(
    store: &Store,
    c: &Connection,
    p: &NativeReplacementPrepared,
) -> Result<(serde_json::Value, Vec<AuthorityScope>)> {
    local_owner(store, &p.owner, &p.runtime)?;
    let raw:Option<(String,String,String)>=c.query_row("SELECT n.plan_json,o.owner_id,o.conversation_id FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='prepare' JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=s.operation_record_id WHERE n.operation_record_id=?1 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4 AND s.attempt_id=?5 AND s.effect_id=?6 AND s.started=1 AND s.observation_json=?7 AND e.state='confirmed' AND o.command_type='connection.replace' AND (o.state IN ('preparing','cancelled','failed') OR (o.state='unknown' AND o.cancel_requested=1)) AND NOT EXISTS(SELECT 1 FROM native_replacement_steps k WHERE k.operation_record_id=n.operation_record_id AND k.phase='commit') AND NOT EXISTS(SELECT 1 FROM operation_effects ce WHERE ce.operation_record_id=n.operation_record_id AND ce.provider='native-replacement-commit')",params![p.record,p.runtime,p.reservation,p.hash,p.attempt,p.effect,p.observation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_|Error::storage())?;
    let (raw, owner, cid) = raw.ok_or_else(Error::forbidden)?;
    let saved: serde_json::Value = serde_json::from_str(&raw).map_err(|_| Error::storage())?;
    for key in ["candidate_connection_id", "candidate_runtime_id"] {
        id(string(&saved, key)?)?;
    }
    for key in ["participant", "endpoint", "device"] {
        id(string(&saved["expected_preparation"], key)?)?;
    }
    never_public_candidate(c, p, &saved)?;
    let (intent, _) = stored_intent(c, p, &saved)?.ok_or_else(Error::forbidden)?;
    intent.validate()?;
    let mut scopes = vec![
        AuthorityScope::identity(&owner, AuthorityMode::Read)?,
        AuthorityScope::conversation(&cid, AuthorityMode::Read)?,
        AuthorityScope::record(&p.record, AuthorityMode::Read)?,
    ];
    for connection in [
        p.selection
            .origin
            .request()
            .connection_id
            .as_deref()
            .ok_or_else(Error::storage)?,
        p.selection.origin.request().intent.retained(),
        string(&saved, "candidate_connection_id")?,
    ] {
        scopes.push(AuthorityScope::connection(connection, AuthorityMode::Read)?);
    }
    Ok((saved, AuthorityScope::merged(&scopes)?))
}
fn project(
    c: &Connection,
    record: &OperationRecord,
) -> Result<(Option<reservation::Intent>, Option<wire::Phase>)> {
    let old:Option<(String,String,u64,String)>=c.query_row("SELECT v.intent_json,v.candidate_connection_id,v.target_generation,n.plan_json FROM native_replacement_public_video_reservations v JOIN native_replacements n ON n.operation_record_id=v.operation_record_id AND n.reservation_id=v.parent_reservation_id AND n.runtime_epoch=v.runtime_epoch AND n.plan_digest=v.plan_digest AND n.candidate_connection_id=v.candidate_connection_id AND n.candidate_runtime_id=v.candidate_runtime_id AND n.generation=v.target_generation AND n.expires_at=v.expires_at WHERE v.operation_record_id=?1",[&record.record_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|Error::storage())?;
    let Some((raw, target, generation, plan)) = old else {
        return Ok((None, None));
    };
    let intent: reservation::Intent = serde_json::from_str(&raw).map_err(|_| Error::storage())?;
    let saved: serde_json::Value = serde_json::from_str(&plan).map_err(|_| Error::storage())?;
    let (a, b) = mapping(&saved)?;
    intent
        .validate_mapping(&a, &b)
        .map_err(|_| Error::storage())?;
    let rows=c.prepare("SELECT phase_json,private_readiness_nonce FROM native_replacement_public_video_phases WHERE operation_record_id=?1 ORDER BY ordinal").map_err(|_|Error::storage())?.query_map([&record.record_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(|_|Error::storage())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|_|Error::storage())?;
    if rows.len() > 5 {
        return Err(Error::storage());
    }
    // Existing private challenge is not synthesized from an ID/Intent. Until
    // the physical phase issuer exists, no production code appends these rows.
    let mut last = None;
    let mut original_nonce = None;
    for (raw, private_nonce) in rows {
        if original_nonce.as_ref().is_some_and(|n| n != &private_nonce) {
            return Err(Error::storage());
        }
        original_nonce = Some(private_nonce.clone());
        let phase: wire::Phase = serde_json::from_str(&raw).map_err(|_| Error::storage())?;
        validate_chain(
            last.as_ref(),
            &phase,
            &intent,
            &record.record_id,
            &record.operation_id,
            &target,
            generation,
            &private_nonce,
            record.state == OperationState::Completed,
        )?;
        last = Some(phase);
    }
    Ok((Some(intent), last))
}
// Internal append kernel ONLY. No public Store entrypoint accepts raw Phase,
// Binding, a completed bool or client JSON as a native/cut grant.
fn validate_chain(
    old: Option<&wire::Phase>,
    next: &wire::Phase,
    intent: &reservation::Intent,
    record: &str,
    operation: &str,
    target: &str,
    generation: u64,
    private_nonce: &str,
    completed: bool,
) -> Result<()> {
    let value = serde_json::to_value(next).map_err(|_| Error::storage())?;
    // Exact private challenge comes from the same original server-owned phase
    // capture, never a derived placeholder or public Intent nonce.
    next.validate(&wire::Context {
        intent,
        operation_record_id: record,
        operation_id: operation,
        target_connection_id: target,
        target_generation: generation,
        private_readiness_nonce: private_nonce,
        completed,
    })?;
    let name = value["phase"].as_str().ok_or_else(Error::storage)?;
    let previous = old
        .map(serde_json::to_value)
        .transpose()
        .map_err(|_| Error::storage())?;
    if let Some(previous) = previous {
        if previous["binding"] != value["binding"] {
            return Err(conflict());
        }
        if previous.get("source_cut_id").is_some()
            && previous["source_cut_id"] != value["source_cut_id"]
        {
            return Err(conflict());
        }
        if previous.get("public_readiness").is_some()
            && value.get("public_readiness").is_some()
            && previous["public_readiness"] != value["public_readiness"]
        {
            return Err(conflict());
        }
        let before = previous["phase"].as_str().ok_or_else(Error::storage)?;
        if !matches!(
            (before, name),
            ("negotiated-never-fed", "cut-bound-post-f" | "discarded")
                | ("cut-bound-post-f", "decoded-ready" | "discarded")
                | ("decoded-ready", "activated" | "discarded")
        ) {
            return Err(conflict());
        }
    } else if name != "negotiated-never-fed" {
        return Err(conflict());
    }
    Ok(())
}

// A Store-only kernel, deliberately without a production native/cut issuer.
// The caller must first own the exact transaction Read; no row is an IO grant.
fn append_phase(
    store: &Store,
    c: &Connection,
    held: &Arc<AuthorityGuardSet>,
    r: &NativeReplacementPublicVideoReservation,
    next: &wire::Phase,
    private_nonce: &str,
) -> Result<()> {
    let p = &r.original;
    local_owner(store, &p.owner, &p.runtime)?;
    id(private_nonce)?;
    if private_nonce == r.intent.nonce {
        return Err(conflict());
    }
    let root:Option<(String,String,bool)>=c.query_row("SELECT operation_id,state,(state='completed') FROM operations WHERE record_id=?1 AND command_type='connection.replace' AND tombstoned=0",[&p.record],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(|_|Error::storage())?;
    let (operation, state, completed) = root.ok_or_else(Error::forbidden)?;
    let value = serde_json::to_value(next).map_err(|_| Error::storage())?;
    let name = value["phase"].as_str().ok_or_else(Error::storage)?;
    if (name == "activated" && !completed)
        || (name != "activated" && state != "preparing")
        || terminal(c, &p.record)?
    {
        return Err(Error::forbidden());
    }
    let encoded = canonical(&value)?;
    if let Some((old,nonce))=c.query_row("SELECT phase_json,private_readiness_nonce FROM native_replacement_public_video_phases WHERE operation_record_id=?1 AND phase=?2",params![p.record,name],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?))).optional().map_err(|_|Error::storage())? {
        return if old==encoded && nonce==private_nonce {Ok(())} else {Err(conflict())};
    }
    let previous:Option<(u64,String,String)>=c.query_row("SELECT ordinal,phase_json,private_readiness_nonce FROM native_replacement_public_video_phases WHERE operation_record_id=?1 ORDER BY ordinal DESC LIMIT 1",[&p.record],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(|_|Error::storage())?;
    let ordinal = previous.as_ref().map_or(1, |(n, _, _)| n + 1);
    if ordinal > 5
        || previous
            .as_ref()
            .is_some_and(|(_, _, nonce)| nonce != private_nonce)
    {
        return Err(conflict());
    }
    let previous = previous
        .map(|(_, raw, _)| serde_json::from_str::<wire::Phase>(&raw).map_err(|_| Error::storage()))
        .transpose()?;
    validate_chain(
        previous.as_ref(),
        next,
        &r.intent,
        &p.record,
        &operation,
        string(&r.saved, "candidate_connection_id")?,
        r.saved["generation"].as_u64().ok_or_else(Error::storage)?,
        private_nonce,
        completed,
    )?;
    let expected = canonical(&serde_json::json!([
        p.record,
        name,
        ordinal,
        private_nonce,
        encoded
    ]))?;
    let permit = store.authority_writer_boundary.public_video_metadata(
        store,
        c,
        held,
        &p.record,
        &p.runtime,
        &p.reservation,
        &p.hash,
        "public-video-phase",
        expected,
    )?;
    c.execute("INSERT INTO native_replacement_public_video_phases(operation_record_id,phase,ordinal,private_readiness_nonce,phase_json) VALUES(?1,?2,?3,?4,?5)",params![p.record,name,ordinal,private_nonce,encoded]).map_err(|_|Error::storage())?;
    drop(permit);
    Ok(())
}
fn append_terminal(
    store: &Store,
    c: &Connection,
    held: &Arc<AuthorityGuardSet>,
    r: &NativeReplacementUnpublishedRead,
    reason: wire::DiscardReason,
) -> Result<()> {
    if !r.current() || !store.authority_covers(&r.held, &historical(store, c, &r.original)?.1) {
        return Err(Error::forbidden());
    }
    let phases:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_public_video_phases WHERE operation_record_id=?1)",[&r.original.record],|row|row.get(0)).map_err(|_|Error::storage())?;
    // This internal marker deliberately covers ONLY intent-before-binding.
    // Actual bound Discarded requires the future opaque physical close issuer.
    if phases {
        return Err(conflict());
    }
    let reason = serde_json::to_value(reason)
        .map_err(|_| Error::storage())?
        .as_str()
        .ok_or_else(Error::storage)?
        .to_owned();
    if let Some(old)=c.query_row("SELECT reason FROM native_replacement_public_video_terminals WHERE operation_record_id=?1",[&r.original.record],|row|row.get::<_,String>(0)).optional().map_err(|_|Error::storage())? {
        return if old==reason {Ok(())} else {Err(conflict())};
    }
    let p = &r.original;
    let expected = canonical(&serde_json::json!([p.record, reason]))?;
    let permit = store.authority_writer_boundary.public_video_metadata(
        store,
        c,
        held,
        &p.record,
        &p.runtime,
        &p.reservation,
        &p.hash,
        "public-video-terminal",
        expected,
    )?;
    c.execute("INSERT INTO native_replacement_public_video_terminals(operation_record_id,reason) VALUES(?1,?2)",params![p.record,reason]).map_err(|_|Error::storage())?;
    drop(permit);
    Ok(())
}

#[cfg(test)]
impl Store {
    // Explicit controlled Store observations, never an SDK/native qualification.
    pub(crate) fn test_append_public_video_phase(
        &self,
        r: &NativeReplacementPublicVideoReservation,
        next: &wire::Phase,
        nonce: &str,
    ) -> Result<()> {
        let p = &r.original;
        let (held, mut c) =
            self.try_replacement_metadata_projection(|c| original_read_scopes(self, c, p))?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        append_phase(self, &tx, &held, r, next, nonce)?;
        tx.commit().map_err(|_| Error::storage())
    }
    pub(crate) fn test_append_public_video_terminal(
        &self,
        r: &NativeReplacementUnpublishedRead,
        reason: wire::DiscardReason,
    ) -> Result<()> {
        let mut c = self.try_connection()?;
        c.bind_available_mutation_authority(self, &r.held)?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        append_terminal(self, &tx, &r.held, r, reason)?;
        tx.commit().map_err(|_| Error::storage())
    }
}
