//! Exact original media incarnations and observed credential-epoch rotation.
//! Plans contain metadata only. The returned peer guard spans the actual Core
//! rotation; a captured applied receipt can only record that result, never IO.
use super::*;
use sha2::Digest;
use std::collections::{BTreeMap, BTreeSet};

pub struct NativeCredentialRefreshTarget {
    original: Arc<NativeMediaOrigin>,
    next: Arc<NativeMediaOrigin>,
    view: PeerConnectionProjection,
    stream_revision: u64,
}
impl NativeCredentialRefreshTarget {
    pub fn original(&self) -> &Arc<NativeMediaOrigin> {
        &self.original
    }
    pub fn next(&self) -> &Arc<NativeMediaOrigin> {
        &self.next
    }
    pub fn view(&self) -> &PeerConnectionProjection {
        &self.view
    }
    pub fn stream_revision(&self) -> u64 {
        self.stream_revision
    }
    pub fn expected_credential_revision(&self) -> u64 {
        self.original.credential_epoch + 1
    }
}
pub struct NativeCredentialRefreshPlan {
    owner: Weak<InstallationOwnership>,
    runtime: String,
    binding: PeerAuthorityRequest,
    targets: Vec<NativeCredentialRefreshTarget>,
    expires_at: i64,
}
impl NativeCredentialRefreshPlan {
    pub fn targets(&self) -> &[NativeCredentialRefreshTarget] {
        &self.targets
    }
    pub fn expires_at(&self) -> i64 {
        self.expires_at
    }
    pub fn current(&self) -> bool {
        crate::now() < self.expires_at && self.owner.upgrade().is_some_and(|o| o.current())
    }
}
struct Reserved {
    id: String,
    parent: String,
    epoch: u64,
    snapshot: String,
}
pub struct NativeCredentialRefreshAttempt {
    owner: Weak<InstallationOwnership>,
    runtime: String,
    refresh: String,
    rows: Vec<Reserved>,
}
pub struct NativeCredentialRefreshApplied(NativeCredentialRefreshAttempt);
macro_rules! redacted {
    ($($t:ty),*) => { $(impl std::fmt::Debug for $t {
        fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {
            f.write_str(concat!(stringify!($t),"(redacted)"))
        }
    })* };
}
redacted!(
    NativeCredentialRefreshTarget,
    NativeCredentialRefreshPlan,
    NativeCredentialRefreshAttempt,
    NativeCredentialRefreshApplied
);

pub(super) fn require_epoch(
    store: &Store,
    c: &Connection,
    origin: &NativeMediaOrigin,
) -> Result<()> {
    let uncertain: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM native_media_credential_bindings WHERE parent_record_id=?1 AND runtime_epoch=?2 AND state='uncertain')", params![origin.offer_record_id(),store.runtime_epoch()], |r|r.get(0)).map_err(|_|Error::storage())?;
    let current: Option<(u64,String,String,String,i64)> = c.query_row("SELECT epoch,credential_id,key_id,device_id,source_expires_at FROM native_media_credential_bindings WHERE parent_record_id=?1 AND runtime_epoch=?2 AND state='applied' ORDER BY epoch DESC LIMIT 1", params![origin.offer_record_id(),store.runtime_epoch()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(|_|Error::storage())?;
    let pending: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM native_media_credential_bindings WHERE parent_record_id=?1 AND runtime_epoch=?2 AND state='prepared' AND started=1)", params![origin.offer_record_id(),store.runtime_epoch()], |r|r.get(0)).map_err(|_|Error::storage())?;
    if pending {
        return Err(Error::new(
            "authority-busy",
            "Native credential rotation is in progress",
        ));
    }
    if uncertain {
        return Err(Error::new(
            "reconciliation-required",
            "Native credential rotation is uncertain",
        ));
    }
    match current {
        None if origin.credential_epoch == 0 => Ok(()),
        Some((epoch, credential, key, device, expires))
            if epoch == origin.credential_epoch
                && origin.binding.credentials.last() == Some(&credential)
                && origin.binding.key_id == key
                && origin.binding.device.id == device
                && origin.expires_at == expires =>
        {
            Ok(())
        }
        _ => Err(Error::forbidden()),
    }
}
fn derive(
    store: &Store,
    c: &Connection,
    binding: &PeerAuthorityRequest,
    origins: &[Arc<NativeMediaOrigin>],
) -> Result<(
    CredentialMetadata,
    PeerSigningKey,
    Vec<AuthorityScope>,
    i64,
    Vec<PeerConnectionProjection>,
)> {
    if binding.credentials.len() != 2
        || binding.credentials[0] == binding.credentials[1]
        || !matches!(binding.command, PeerCommand::Binding)
        || origins.is_empty()
        || origins.len() > store.limits().connections.min(64)
    {
        return Err(Error::forbidden());
    }
    let (meta, key, mut scopes, mut expires) = checked(store, c, binding)?;
    let mut seen = BTreeSet::new();
    let mut views = Vec::new();
    for origin in origins {
        if !seen.insert(origin.offer_record_id())
            || origin.binding.credentials.last() != binding.credentials.first()
            || origin.binding.device != binding.device
            || origin.binding.key_id != binding.key_id
            || origin.binding.algorithm != binding.algorithm
            || origin.public_key != key.public_key
            || origin.credential_epoch >= 9_007_199_254_740_990
        {
            return Err(Error::forbidden());
        }
        let (view, extra, until) = facts_for(store, c, origin, ReadPurpose::Signal)?;
        let active: bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_connection_lifetimes life JOIN native_admission_sources n ON n.operation_record_id=life.operation_record_id WHERE life.operation_record_id=?1 AND life.runtime_epoch=?2 AND life.generation=?3 AND life.retired=0 AND n.state='completed' AND n.effect_started=1)",params![origin.offer_record_id(),store.runtime_epoch(),origin.generation()],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !active
            || !matches!(view.connection.state.as_str(), "ready" | "active")
            || view.session.actor.identity_id != meta.actor.identity_id
        {
            return Err(Error::forbidden());
        }
        let pending:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM native_stream_operations WHERE parent_record_id=?1 AND closed=0)",[origin.offer_record_id()],|r|r.get(0)).map_err(|_|Error::storage())?;
        if pending {
            return Err(Error::new(
                "authority-busy",
                "Original native control must settle before credential rotation",
            ));
        }
        scopes.extend(extra);
        expires = expires.min(until);
        views.push(view);
    }
    Ok((
        meta,
        key,
        AuthorityScope::merged(&scopes)?,
        expires.min(crate::now() + 5),
        views,
    ))
}
fn snapshot(t: &NativeCredentialRefreshTarget, b: &PeerAuthorityRequest) -> Result<String> {
    crate::provisioning::canonical_json(&serde_json::json!({
        "parent_record_id":t.original.offer_record_id(),"connection_id":t.original.connection_id(),
        "runtime_connection_id":t.original.runtime_connection_id(),"generation":t.original.generation(),
        "participant_id":t.original.participant_id(),"membership_epoch":t.original.membership_epoch(),
        "endpoint_id":t.original.endpoint_id,"previous_epoch":t.original.credential_epoch,
        "epoch":t.next.credential_epoch,"previous_credential_id":b.credentials[0],"credential_id":b.credentials[1],
        "key_id":b.key_id,"algorithm":b.algorithm,"device_id":b.device.id,
        "device_kind":b.device.kind,"device_platform":b.device.platform,
        "public_key_sha256":hex::encode(sha2::Sha256::digest(&t.original.public_key)),
        "source_expires_at":t.next.expires_at,"stream_revision":t.stream_revision,
        "expected_credential_revision":t.expected_credential_revision()
    }))
}
fn owner(
    store: &Store,
    weak: &Weak<InstallationOwnership>,
    runtime: &str,
) -> Result<Arc<InstallationOwnership>> {
    let owner = store.require_runtime_owner()?;
    if runtime != store.runtime_epoch()
        || !weak.upgrade().is_some_and(|old| Arc::ptr_eq(&old, &owner))
    {
        return Err(Error::forbidden());
    }
    Ok(owner)
}
fn exact(
    c: &Connection,
    a: &NativeCredentialRefreshAttempt,
    row: &Reserved,
) -> Result<(String, bool)> {
    c.query_row("SELECT state,started FROM native_media_credential_bindings WHERE id=?1 AND parent_record_id=?2 AND runtime_epoch=?3 AND refresh_id=?4 AND epoch=?5 AND snapshot_json=?6",params![row.id,row.parent,a.runtime,a.refresh,row.epoch,row.snapshot],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|Error::storage())?.ok_or_else(Error::forbidden)
}
impl NativeCredentialRefreshAttempt {
    /// Trusted runtime boundary only after ALL original Core controllers return
    /// these actual credential revisions. A map read from a wire is not proof.
    pub fn applied(
        self,
        revisions: BTreeMap<String, u64>,
    ) -> Result<NativeCredentialRefreshApplied> {
        if revisions.len() != self.rows.len()
            || self
                .rows
                .iter()
                .any(|r| revisions.get(&r.parent) != Some(&(r.epoch + 1)))
        {
            return Err(Error::new(
                "conflict",
                "Native credential observation differs from the original attempt",
            ));
        }
        Ok(NativeCredentialRefreshApplied(self))
    }
}
impl Store {
    /// Fresh union admission. The host must release any earlier Binding-only
    /// lease before entering here, never nest readers behind its queued writer.
    /// The returned plan is inert; keep the exact peer guard through Core/IO.
    pub fn admit_native_credential_refresh(
        &self,
        binding: &PeerAuthorityRequest,
        origins: &[Arc<NativeMediaOrigin>],
        wait: &AuthorityWait,
    ) -> Result<(PeerAuthorityGuard, NativeCredentialRefreshPlan)> {
        let owner = self.require_runtime_owner()?;
        let mut originals = origins.to_vec();
        originals.sort_by(|a, b| a.offer_record_id().cmp(b.offer_record_id()));
        let held = self.stable_authority(wait, || {
            let c = self.connection()?;
            wait.check()?;
            derive(self, &c, binding, &originals).map(|(_, _, s, _, _)| s)
        })?;
        let c = self.connection()?;
        wait.check()?;
        let (metadata, key, scopes, expires_at, views) = derive(self, &c, binding, &originals)?;
        if !self.authority_covers(&held, &scopes) || expires_at <= crate::now() {
            return Err(Error::forbidden());
        }
        let next_expiry = metadata.actor.expires_at.min(key.expires_at);
        let mut next_binding = binding.clone();
        next_binding.credentials = vec![binding.credentials[1].clone()];
        let mut targets = Vec::new();
        for (origin, view) in originals.into_iter().zip(views) {
            let mut next = origin.as_ref().clone();
            next.credential_epoch += 1;
            next.binding = next_binding.clone();
            next.expires_at = next_expiry;
            let revision:u64=c.query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1 AND generation=?2",params![origin.connection_id(),origin.generation()],|r|r.get(0)).map_err(|_|Error::storage())?;
            targets.push(NativeCredentialRefreshTarget {
                original: origin,
                next: Arc::new(next),
                view,
                stream_revision: revision,
            });
        }
        wait.check()?;
        let plan = NativeCredentialRefreshPlan {
            owner: Arc::downgrade(&owner),
            runtime: self.runtime_epoch().into(),
            binding: binding.clone(),
            targets,
            expires_at,
        };
        Ok((
            PeerAuthorityGuard {
                _owner: owner,
                _scopes: held,
                metadata,
                key,
                expires_at,
                request: binding.clone(),
                creation_id: None,
                native_member: None,
            },
            plan,
        ))
    }
    /// Metadata reservation before any Core call. This neither changes current
    /// packet authority nor permits a second actual credential-rotation attempt.
    pub fn prepare_native_credential_refresh(
        &self,
        plan: &NativeCredentialRefreshPlan,
        proof: &PeerAuthorityGuard,
        refresh: &str,
    ) -> Result<NativeCredentialRefreshAttempt> {
        refresh_identifier(refresh)?;
        let current = owner(self, &plan.owner, &plan.runtime)?;
        if !plan.current()
            || !proof.current()
            || !Arc::ptr_eq(&current, &proof._owner)
            || proof.request.credentials != plan.binding.credentials
            || proof.request.device != plan.binding.device
            || proof.request.key_id != plan.binding.key_id
            || proof.request.algorithm != plan.binding.algorithm
            || !matches!(proof.request.command, PeerCommand::Binding)
        {
            return Err(Error::forbidden());
        }
        let origins = plan
            .targets
            .iter()
            .map(|t| t.original.clone())
            .collect::<Vec<_>>();
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let (_, _, scopes, _, _) = derive(self, &tx, &plan.binding, &origins)?;
        if !self.authority_covers(&proof._scopes, &scopes) {
            return Err(Error::forbidden());
        }
        let mut rows = Vec::new();
        for target in &plan.targets {
            let revision:u64=tx.query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",[target.original.connection_id()],|r|r.get(0)).map_err(|_|Error::storage())?;
            if revision != target.stream_revision {
                return Err(Error::new(
                    "conflict",
                    "Native stream revision changed before credential rotation",
                ));
            }
            let encoded = snapshot(target, &plan.binding)?;
            let saved:Option<(String,String,bool,String)>=tx.query_row("SELECT id,snapshot_json,started,state FROM native_media_credential_bindings WHERE parent_record_id=?1 AND refresh_id=?2",params![target.original.offer_record_id(),refresh],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|_|Error::storage())?;
            let id = if let Some((id, old, started, state)) = saved {
                if old != encoded {
                    return Err(Error::new(
                        "conflict",
                        "Native credential refresh intent changed",
                    ));
                }
                if started || state != "prepared" {
                    return Err(Error::new(
                        "reconciliation-required",
                        "Native credential refresh was already attempted",
                    ));
                }
                id
            } else {
                let reserved: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM native_media_credential_bindings WHERE parent_record_id=?1 AND epoch=?2 AND state!='refused')", params![target.original.offer_record_id(),target.next.credential_epoch], |r|r.get(0)).map_err(|_|Error::storage())?;
                if reserved {
                    return Err(Error::new(
                        "reconciliation-required",
                        "Native credential epoch is already reserved",
                    ));
                }
                let id = format!("ncr_{}", uuid::Uuid::new_v4().simple());
                tx.execute("INSERT INTO native_media_credential_bindings(id,parent_record_id,refresh_id,runtime_epoch,epoch,previous_credential_id,credential_id,key_id,device_id,source_expires_at,stream_revision,snapshot_json,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",params![id,target.original.offer_record_id(),refresh,plan.runtime,target.next.credential_epoch,plan.binding.credentials[0],plan.binding.credentials[1],plan.binding.key_id,plan.binding.device.id,target.next.expires_at,target.stream_revision,encoded,crate::now()]).map_err(|_|Error::storage())?;
                id
            };
            rows.push(Reserved {
                id,
                parent: target.original.offer_record_id().into(),
                epoch: target.next.credential_epoch,
                snapshot: encoded,
            });
        }
        if !proof.current() || !plan.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(NativeCredentialRefreshAttempt {
            owner: plan.owner.clone(),
            runtime: plan.runtime.clone(),
            refresh: refresh.into(),
            rows,
        })
    }
    pub fn begin_native_credential_refresh(
        &self,
        attempt: &NativeCredentialRefreshAttempt,
        plan: &NativeCredentialRefreshPlan,
        proof: &PeerAuthorityGuard,
    ) -> Result<()> {
        owner(self, &attempt.owner, &attempt.runtime)?;
        if !proof.current()
            || !plan.current()
            || attempt.rows.len() != plan.targets.len()
            || attempt.runtime != plan.runtime
            || !Arc::ptr_eq(&owner(self, &plan.owner, &plan.runtime)?, &proof._owner)
            || proof.request.credentials != plan.binding.credentials
            || proof.request.device != plan.binding.device
            || proof.request.key_id != plan.binding.key_id
            || proof.request.algorithm != plan.binding.algorithm
            || !matches!(proof.request.command, PeerCommand::Binding)
        {
            return Err(Error::forbidden());
        }
        let origins = plan
            .targets
            .iter()
            .map(|t| t.original.clone())
            .collect::<Vec<_>>();
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let (_, _, scopes, _, _) = derive(self, &tx, &plan.binding, &origins)?;
        if proof.request.credentials != plan.binding.credentials
            || !self.authority_covers(&proof._scopes, &scopes)
        {
            return Err(Error::forbidden());
        }
        for (row, target) in attempt.rows.iter().zip(&plan.targets) {
            if row.parent != target.original.offer_record_id()
                || row.snapshot != snapshot(target, &plan.binding)?
                || exact(&tx, attempt, row)? != ("prepared".into(), false)
            {
                return Err(Error::new(
                    "reconciliation-required",
                    "Native credential refresh cannot submit again",
                ));
            }
            tx.execute(
                "UPDATE native_media_credential_bindings SET started=1 WHERE id=?1",
                [&row.id],
            )
            .map_err(|_| Error::storage())?;
        }
        if !proof.current() || !plan.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())
    }
    pub fn record_native_credential_refresh_applied(
        &self,
        receipt: &NativeCredentialRefreshApplied,
    ) -> Result<()> {
        let a = &receipt.0;
        owner(self, &a.owner, &a.runtime)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        for row in &a.rows {
            match exact(&tx, a, row)? {
                (state, true) if state == "applied" => (),
                (state, true) if state == "prepared" || state == "uncertain" => {
                    // Uncertain rows cannot be overwritten: only this original
                    // captured result may resolve them, not a reconstructed map.
                    tx.execute("UPDATE native_media_credential_bindings SET state='applied',core_credential_revision=?2,observed_at=?3 WHERE id=?1",params![row.id,row.epoch+1,crate::now()]).map_err(|_|Error::storage())?;
                }
                _ => {
                    return Err(Error::new(
                        "conflict",
                        "Native credential observation conflicts",
                    ))
                }
            }
        }
        tx.commit().map_err(|_| Error::storage())
    }
    /// A trusted original attempt may record refusal/uncertainty, never make
    /// either credential usable or execute a retry from a stored row.
    pub fn record_native_credential_refresh_stopped(
        &self,
        a: &NativeCredentialRefreshAttempt,
        uncertain: bool,
    ) -> Result<()> {
        owner(self, &a.owner, &a.runtime)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let desired = if uncertain { "uncertain" } else { "refused" };
        for row in &a.rows {
            let (state, started) = exact(&tx, a, row)?;
            if state == desired {
                continue;
            }
            if state != "prepared" || (uncertain && !started) || (!uncertain && started) {
                return Err(Error::forbidden());
            }
            tx.execute(
                "UPDATE native_media_credential_bindings SET state=?2,observed_at=?3 WHERE id=?1",
                params![row.id, desired, crate::now()],
            )
            .map_err(|_| Error::storage())?;
        }
        tx.commit().map_err(|_| Error::storage())
    }
    pub fn recover_native_credential_refreshes(&self, limit: usize) -> Result<usize> {
        if limit == 0 || limit > 1000 {
            return Err(Error::new(
                "invalid-request",
                "Invalid credential recovery bound",
            ));
        }
        self.require_runtime_owner()?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let mut q=tx.prepare("SELECT id FROM native_media_credential_bindings WHERE runtime_epoch!=?1 AND state='prepared' ORDER BY id LIMIT ?2").map_err(|_|Error::storage())?;
        let ids = q
            .query_map(params![self.runtime_epoch(), limit], |r| {
                r.get::<_, String>(0)
            })
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| Error::storage())?;
        drop(q);
        for id in &ids {
            tx.execute("UPDATE native_media_credential_bindings SET state=CASE WHEN started=1 THEN 'uncertain' ELSE 'refused' END,observed_at=?2 WHERE id=?1",params![id,crate::now()]).map_err(|_|Error::storage())?;
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(ids.len())
    }
}
fn refresh_identifier(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._~-:".contains(&b))
    {
        return Err(Error::new(
            "invalid-request",
            "Invalid original credential refresh ID",
        ));
    }
    Ok(())
}
