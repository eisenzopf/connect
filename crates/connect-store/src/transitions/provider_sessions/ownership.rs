use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex, OnceLock,
};
/// Exclusive worker/recovery custody. In-flight and captured results retain the
/// role after cancellation, so another worker cannot recover a live submission.
pub struct ProviderSessionVoiceWorker {
    owner: Arc<InstallationOwnership>,
    manager: Arc<crate::authority_fences::Manager>,
    runtime: String,
    pub(super) id: String,
    retired: AtomicBool,
    pristine: Mutex<bool>,
}
impl std::fmt::Debug for ProviderSessionVoiceWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProviderSessionVoiceWorker(redacted)")
    }
}
impl ProviderSessionVoiceWorker {
    pub fn retire(&self) {
        self.retired.store(true, Ordering::Release);
    }
    pub(super) fn current(&self) -> bool {
        self.owner.current() && !self.retired.load(Ordering::Acquire)
    }
    pub(super) fn belongs(&self, store: &Store) -> bool {
        Arc::ptr_eq(&self.manager, &store.authority) && self.runtime == store.runtime_epoch()
    }
    pub(super) fn require(&self, store: &Store) -> Result<()> {
        if !self.current() || !self.belongs(store) {
            return Err(Error::forbidden());
        }
        Ok(())
    }
    pub(super) fn before_dispatch(&self) -> Result<()> {
        *self.pristine.lock().map_err(|_| Error::storage())? = false;
        Ok(())
    }
    pub(super) fn require_recovery(
        &self,
        store: &Store,
    ) -> Result<std::sync::MutexGuard<'_, bool>> {
        self.require(store)?;
        let phase = self.pristine.lock().map_err(|_| Error::storage())?;
        if !*phase {
            return Err(Error::new(
                "conflict",
                "Provider Session recovery must precede dispatch and runs once per owned worker",
            ));
        }
        Ok(phase)
    }
}
impl Store {
    pub fn acquire_provider_session_voice_worker(&self) -> Result<Arc<ProviderSessionVoiceWorker>> {
        type Key = (usize, String);
        static ROLES: OnceLock<
            Mutex<std::collections::BTreeMap<Key, Weak<ProviderSessionVoiceWorker>>>,
        > = OnceLock::new();
        let owner = self.require_runtime_owner()?;
        let mut roles = ROLES
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| Error::storage())?;
        roles.retain(|_, role| role.strong_count() != 0);
        let key = (
            Arc::as_ptr(&self.authority) as usize,
            self.runtime_epoch().to_owned(),
        );
        if roles.contains_key(&key) {
            return Err(Error::new(
                "conflict",
                "Provider Session worker still owns this runtime",
            ));
        }
        if roles.len() >= 1024 {
            return Err(Error::new(
                "capacity-exceeded",
                "Provider Session worker ownership capacity exhausted",
            ));
        }
        let role = Arc::new(ProviderSessionVoiceWorker {
            owner,
            manager: self.authority.clone(),
            runtime: self.runtime_epoch().into(),
            id: uuid::Uuid::new_v4().to_string(),
            retired: AtomicBool::new(false),
            pristine: Mutex::new(true),
        });
        roles.insert(key, Arc::downgrade(&role));
        Ok(role)
    }
    /// Bounded protected recovery enumeration. Original start deadlines remain
    /// fixed, and private provider references do not enter public views.
    pub fn pending_provider_session_voice(
        &self,
        worker: &ProviderSessionVoiceWorker,
    ) -> Result<Vec<ProviderSessionVoiceOrigin>> {
        worker.require(self)?;
        let c = self.connection()?;
        let mut q=c.prepare("SELECT id FROM provider_session_voice WHERE closed_at IS NULL ORDER BY id LIMIT ?1").map_err(|_|Error::storage())?;
        let ids = q
            .query_map([self.limits.connections as u64], |r| r.get::<_, String>(0))
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| Error::storage())?;
        drop(q);
        drop(c);
        ids.iter()
            .map(|id| self.resume_provider_session_voice(worker, id))
            .collect()
    }
    /// Reattach an existing exact provider-control binding after exclusive
    /// recovery. This never renews the original start deadline or permissions.
    pub fn resume_provider_session_voice(
        &self,
        worker: &ProviderSessionVoiceWorker,
        id: &str,
    ) -> Result<ProviderSessionVoiceOrigin> {
        worker.require(self)?;
        let c = self.connection()?;
        let (credential,source,policy,expires):(String,String,String,i64)=c.query_row("SELECT caller_credential_id,source_json,policy_json,expires_at FROM provider_session_voice WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|Error::forbidden())?;
        Ok(ProviderSessionVoiceOrigin {
            owner: Arc::downgrade(&worker.owner),
            runtime: self.runtime_epoch().into(),
            credential,
            id: id.into(),
            direction: call_direction(&c, id)?,
            source: serde_json::from_str(&source).map_err(|_| Error::storage())?,
            policy: serde_json::from_str(&policy).map_err(|_| Error::storage())?,
            expires,
        })
    }
}
impl Store {
    /// Validate protected listener/SDK composition before it admits any jobs.
    /// Actual invitation, audience and packet authority are rechecked later.
    pub fn provider_session_voice_binding(
        &self,
        policy: &ProviderSessionVoicePolicy,
    ) -> Result<(String, String)> {
        let c = self.connection()?;
        binding(&c, policy)
    }

    /// A monitor may decide to stop an owned resource, never renew the original
    /// invitation grant or authorize a fresh provider create. Established calls
    /// use their own configured AI/phone/route credentials rather than retaining
    /// the initiating caller token as a permanent grant.
    pub fn provider_session_voice_current(
        &self,
        origin: &ProviderSessionVoiceOrigin,
        established: bool,
    ) -> Result<bool> {
        origin.lifetime(self)?;
        let c = self.try_connection()?;
        if !established {
            return origin.current(self, &c).map(|_| true);
        }
        self.provider_session_voice_current_in(&c, origin)
    }
    fn provider_session_voice_current_in(
        &self,
        c: &Connection,
        origin: &ProviderSessionVoiceOrigin,
    ) -> Result<bool> {
        let ai = actor_from_connection(c, &origin.policy.assistant_credential_id)?;
        let phone = actor_from_connection(c, &origin.policy.endpoint_credential_id)?;
        let route = actor_from_connection(c, &origin.policy.route_credential_id)?;
        ai.require("connection.offer")?;
        phone.require("connection.offer")?;
        route.require("connection.offer")?;
        if ai.identity_id != origin.source.caller.identity_id
            || phone.identity_id != origin.source.target.identity_id
            || route.identity_id != origin.source.route_owner_id
        {
            return Err(Error::forbidden());
        }
        for old in &origin.source.audience {
            let now = crate::audience::member(c, &old.identity_id, origin.conversation_id(), true)?;
            if now.participant_id != old.participant_id
                || now.membership_epoch != old.membership_epoch
                || (now.role == "observer"
                    && (now.identity_id == ai.identity_id || now.identity_id == phone.identity_id))
                || !ai.permits_recipient(&old.identity_id)
                || !phone.permits_recipient(&old.identity_id)
                || !route.permits_recipient(&old.identity_id)
            {
                return Err(Error::forbidden());
            }
        }
        let current:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM endpoints e JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.revoked=0 WHERE e.id=?1 AND e.revision=?2 AND e.identity_id=?3 AND e.device_id=?4 AND e.verification_state='verified' AND e.secret_ref=?5 AND e.route_owner_id=?6) AND EXISTS(SELECT 1 FROM endpoints e JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.revoked=0 WHERE e.id=?7 AND e.revision=?8 AND e.device_id=?9 AND e.identity_id=?10 AND e.verification_state='verified' AND e.secret_ref=?5 AND e.route_owner_id=?6) AND EXISTS(SELECT 1 FROM sessions s JOIN conversations v ON v.id=s.conversation_id AND v.closed=0 JOIN session_participants a ON a.session_id=s.id JOIN session_participants b ON b.session_id=s.id WHERE s.id=?11 AND s.state IN ('invited','accepted','ready') AND a.participant_id=?12 AND a.membership_epoch=?13 AND a.state IN ('present','invited') AND a.role!='observer' AND b.participant_id=?14 AND b.membership_epoch=?15 AND b.state IN ('present','invited') AND b.role!='observer')",params![origin.policy.assistant_endpoint_id,origin.source.assistant_endpoint_revision,ai.identity_id,origin.source.assistant_device_id,origin.policy.route_reference,route.identity_id,origin.policy.endpoint_id,origin.source.endpoint_revision,origin.source.device_id,phone.identity_id,origin.session_id(),origin.source.caller.participant_id,origin.source.caller.membership_epoch,origin.source.target.participant_id,origin.source.target.membership_epoch],|r|r.get(0)).map_err(|_|Error::storage())?;
        if !current {
            return Err(Error::forbidden());
        }
        Ok(true)
    }
}

impl Store {
    pub fn provider_session_voice_observed(
        &self,
        worker: &ProviderSessionVoiceWorker,
        origin: &ProviderSessionVoiceOrigin,
    ) -> Result<ProviderSessionVoiceView> {
        worker.require(self)?;
        origin.lifetime(self)?;
        let c = self.connection()?;
        super::lifecycle::exact(&c, origin)?;
        load_view(&c, origin.id())
    }
    pub fn provider_session_voice_started(
        &self,
        worker: &ProviderSessionVoiceWorker,
        origin: &ProviderSessionVoiceOrigin,
    ) -> Result<(bool, bool)> {
        worker.require(self)?;
        origin.lifetime(self)?;
        let c = self.connection()?;
        super::lifecycle::exact(&c, origin)?;
        c.query_row(
            "SELECT create_started,end_started FROM provider_session_voice WHERE id=?1",
            [origin.id()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| Error::storage())
    }
}
impl Store {
    /// The already admitted invitation authorizes selecting its explicit target;
    /// an automatic host does not additionally borrow a roster-read grant.
    pub fn provider_session_voice_invited_target(
        &self,
        credential: &str,
        request: &session_control::Request,
        speaker: &str,
        target: &str,
    ) -> Result<Option<String>> {
        let c = self.connection()?;
        let actor = actor_from_connection(&c, credential)?;
        if actor.identity_id != speaker {
            return Ok(None);
        }
        actor.require("session.invite")?;
        let receipt =
            crate::operations::receipts::receipt(&c, &actor.identity_id, &request.operation_id)?
                .ok_or_else(Error::forbidden)?;
        crate::operations::receipts::check_receipt_authority(&c, &actor, &receipt, None)?;
        if receipt.operation
            != (Operation::SessionControl {
                request: request.clone(),
            })
            || receipt.result.is_err()
        {
            return Err(Error::forbidden());
        }
        if !receipt.resources.iter().any(|id| id == target) {
            return Ok(None);
        }
        let member = crate::audience::member(&c, target, &request.conversation_id, true)?;
        let session_control::Action::Invite { participants, .. } = &request.action else {
            return Ok(None);
        };
        Ok(participants
            .contains(&member.participant_id)
            .then_some(member.participant_id))
    }
}

pub(super) fn binding(
    c: &Connection,
    policy: &ProviderSessionVoicePolicy,
) -> Result<(String, String)> {
    policy.validate()?;
    let ai = actor_from_connection(c, &policy.assistant_credential_id)?;
    let phone = actor_from_connection(c, &policy.endpoint_credential_id)?;
    let route = actor_from_connection(c, &policy.route_credential_id)?;
    ai.require("session.invite")?;
    ai.require("connection.offer")?;
    phone.require("connection.offer")?;
    phone.require("session.accept")?;
    route.require("connection.offer")?;
    if authority::tenant(c, &ai)? != authority::tenant(c, &phone)?
        || authority::tenant(c, &ai)? != authority::tenant(c, &route)?
    {
        return Err(Error::forbidden());
    }
    let valid:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM endpoints e JOIN identities i ON i.id=e.identity_id AND i.kind='ai' AND i.revoked=0 JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.revoked=0 WHERE e.id=?1 AND e.identity_id=?2 AND e.address=?3 AND e.transport='vapi-voice' AND e.verification_state='verified' AND e.secret_ref=?4 AND e.route_owner_id=?5 AND EXISTS(SELECT 1 FROM json_each(e.capabilities_json) WHERE value='audio')) AND EXISTS(SELECT 1 FROM endpoints e JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.revoked=0 WHERE e.id=?6 AND e.identity_id=?7 AND e.transport='telephone' AND e.verification_state='verified' AND e.secret_ref=?4 AND e.route_owner_id=?5 AND EXISTS(SELECT 1 FROM json_each(e.capabilities_json) WHERE value='audio'))",params![policy.assistant_endpoint_id,ai.identity_id,policy.assistant_id,policy.route_reference,route.identity_id,policy.endpoint_id,phone.identity_id],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !valid {
        return Err(Error::forbidden());
    }
    Ok((ai.identity_id, phone.identity_id))
}
