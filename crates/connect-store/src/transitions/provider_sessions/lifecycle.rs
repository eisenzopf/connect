use super::*;
pub(super) fn encoded<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|_| Error::storage())
}
fn projection(c: &Connection, id: &str) -> Result<Projection> {
    let raw: String = c
        .query_row(
            "SELECT projection_json FROM provider_session_voice WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| Error::forbidden())?;
    serde_json::from_str(&raw).map_err(|_| Error::storage())
}
pub(super) fn exact(c: &Connection, o: &ProviderSessionVoiceOrigin) -> Result<()> {
    let (credential,source,policy,expires):(String,String,String,i64)=c.query_row("SELECT caller_credential_id,source_json,policy_json,expires_at FROM provider_session_voice WHERE id=?1",[&o.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|Error::forbidden())?;
    if credential != o.credential
        || serde_json::from_str::<Source>(&source).map_err(|_| Error::storage())? != o.source
        || serde_json::from_str::<ProviderSessionVoicePolicy>(&policy)
            .map_err(|_| Error::storage())?
            != o.policy
        || expires != o.expires
        || call_direction(c, o.id())? != o.direction
    {
        return Err(Error::forbidden());
    }
    Ok(())
}
pub(super) fn normalized(raw: &str) -> Option<VoiceState> {
    Some(match raw {
        "queued" | "ringing" => VoiceState::Starting,
        "in-progress" => VoiceState::Active,
        "ended" => VoiceState::Ended,
        "failed" => VoiceState::Failed,
        "uncertain" => VoiceState::Uncertain,
        _ => return None,
    })
}
fn effect_state(c: &Connection, id: &str, state: &str, provider: Option<&str>) -> Result<()> {
    c.execute(
        "UPDATE operation_effects SET state=?2,provider_ref=COALESCE(provider_ref,?3) WHERE id=?1",
        params![id, state, provider],
    )
    .map_err(|_| Error::storage())?;
    Ok(())
}
pub(super) fn touch(c: &Connection, id: &str, store: &Store) -> Result<()> {
    let raw: String = c
        .query_row(
            "SELECT source_json FROM provider_session_voice WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let source: Source = serde_json::from_str(&raw).map_err(|_| Error::storage())?;
    if !crate::conversations::capacity(c, source.audience.len() as u64, 0, store)? {
        return Err(Error::new(
            "capacity-exceeded",
            "Provider Session journal capacity exhausted",
        ));
    }
    let changed=c.execute("UPDATE provider_session_voice SET revision=revision+1 WHERE id=?1 AND revision<9007199254740991",[id]).map_err(|_|Error::storage())?;
    if changed != 1 {
        return Err(conflict());
    }
    let revision: u64 = c
        .query_row(
            "SELECT revision FROM provider_session_voice WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    let data = serde_json::json!({"source_kind":"session","source_id":source.request.session_id,"resource_kind":"provider_call","resource_id":id,"operation_id":source.request.operation_id,"provider_call":load_view(c,id)?});
    for member in &source.audience {
        c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,operation_record_id,resource_revision,data_json) VALUES(?1,?2,?3,'provider_voice',?4,?5,?6)",params![uuid::Uuid::new_v4().to_string(),member.identity_id,source.request.conversation_id,source.invitation_record_id,revision,data.to_string()]).map_err(|_|Error::storage())?;
    }
    Ok(())
}
impl ProviderSessionVoiceDispatch {
    pub fn action(&self) -> ProviderSessionVoiceAction {
        self.action
    }
    pub fn mutation_id(&self) -> &str {
        self.origin.id()
    }
    pub fn current(&self) -> bool {
        crate::now() < self.expires && self.owner.current() && self.worker.current()
    }
    pub fn expires_at(&self) -> i64 {
        self.expires
    }
    pub fn start_request(&self) -> Result<VoiceStart> {
        if self.action != ProviderSessionVoiceAction::Start || !self.current() {
            return Err(Error::forbidden());
        }
        Ok(VoiceStart {
            assistant_id: self.origin.policy.assistant_id.clone(),
            phone_number_id: self.origin.policy.phone_number_id.clone(),
            destination: self.origin.source.address.clone(),
        })
    }
    /// A fresh bounded read for the original attempt, not renewed create custody.
    pub fn reconcile_query(&self) -> Result<connect_connector_api::VoiceReconcileQuery> {
        if !matches!(
            self.action,
            ProviderSessionVoiceAction::Reconcile | ProviderSessionVoiceAction::ResolveQuarantine
        ) || !self.current()
        {
            return Err(Error::forbidden());
        }
        Ok(connect_connector_api::VoiceReconcileQuery {
            request: VoiceStart {
                assistant_id: self.origin.policy.assistant_id.clone(),
                phone_number_id: self.origin.policy.phone_number_id.clone(),
                destination: self.origin.source.address.clone(),
            },
            since: self
                .origin
                .expires
                .saturating_sub(i64::from(self.origin.policy.timeout_seconds) + 60),
            until: self.origin.expires.saturating_add(60),
        })
    }
    /// Private resource capability, supplied only to the configured adapter.
    pub fn existing_resource(&self) -> Result<(&str, Option<&str>)> {
        if self.action == ProviderSessionVoiceAction::Start || !self.current() {
            return Err(Error::forbidden());
        }
        Ok((
            self.provider_id.as_deref().ok_or_else(Error::forbidden)?,
            self.control_url.as_deref(),
        ))
    }
    pub fn capture(
        self,
        result: std::result::Result<VoiceReceipt, Failure>,
    ) -> ProviderSessionVoiceCapture {
        let result = result.and_then(|receipt| {
            let valid_id =
                |v: &str| !v.is_empty() && v.len() <= 256 && !v.chars().any(char::is_control);
            if !valid_id(&receipt.call.id)
                || !valid_id(&receipt.observation.event_id)
                || normalized(&receipt.call.state).map(State::Voice)
                    != Some(receipt.observation.state)
                || receipt.observation.mutation_id != self.origin.id
                || receipt.observation.external_id.as_deref() != Some(receipt.call.id.as_str())
                || receipt.observation.verification == Verification::LocalAttempt
                || (matches!(
                    self.action,
                    ProviderSessionVoiceAction::Reconcile
                        | ProviderSessionVoiceAction::ResolveQuarantine
                ) && receipt.observation.verification != Verification::Reconciliation)
                || self
                    .provider_id
                    .as_ref()
                    .is_some_and(|id| id != &receipt.call.id)
                || receipt
                    .call
                    .control_url
                    .as_ref()
                    .is_some_and(|v| v.len() > 4096 || v.chars().any(char::is_control))
                || receipt
                    .call
                    .ended_reason
                    .as_ref()
                    .is_some_and(|v| v.len() > 256 || v.chars().any(char::is_control))
            {
                return Err(Failure::Uncertain);
            }
            Ok(receipt)
        });
        let (provider_id, control_url, ended_reason, observation) = match result {
            Ok(r) => (
                Some(r.call.id),
                r.call.control_url,
                r.call.ended_reason,
                r.observation,
            ),
            Err(f) => {
                let state = if self.action == ProviderSessionVoiceAction::Start
                    && matches!(f, Failure::Rejected)
                {
                    VoiceState::Failed
                } else {
                    VoiceState::Uncertain
                };
                (
                    None,
                    None,
                    None,
                    Observation {
                        mutation_id: self.origin.id.clone(),
                        external_id: None,
                        event_id: format!("attempt:{}:{:?}:{state:?}", self.origin.id, self.action),
                        state: State::Voice(state),
                        verification: Verification::LocalAttempt,
                    },
                )
            }
        };
        ProviderSessionVoiceCapture {
            origin: self.origin,
            action: self.action,
            provider_id,
            control_url,
            ended_reason,
            observation,
            quarantine: self.quarantine,
            _owner: self.owner,
            manager: self.manager,
            worker: self.worker,
        }
    }
}
impl Store {
    /// The start marker and submitting effect commit before returning transport
    /// custody. Neither replay nor worker recovery can return another Start.
    pub fn begin_provider_session_voice(
        &self,
        worker: &Arc<ProviderSessionVoiceWorker>,
        origin: &ProviderSessionVoiceOrigin,
        action: ProviderSessionVoiceAction,
        manifest: &Manifest,
    ) -> Result<Option<ProviderSessionVoiceDispatch>> {
        worker.require(self)?;
        if action == ProviderSessionVoiceAction::Start
            && origin.direction == ProviderCallDirection::Inbound
        {
            return Err(Error::new(
                "unsupported-connector-operation",
                "An incoming observation cannot submit a new call",
            ));
        }
        worker.before_dispatch()?;
        manifest.validate().map_err(|_| invalid())?;
        let required = match action {
            ProviderSessionVoiceAction::Start => ConnectorOperation::VoiceStart,
            ProviderSessionVoiceAction::Status => ConnectorOperation::VoiceStatus,
            ProviderSessionVoiceAction::Reconcile => ConnectorOperation::VoiceReconcile,
            ProviderSessionVoiceAction::ResolveQuarantine => ConnectorOperation::VoiceReconcile,
            ProviderSessionVoiceAction::End => ConnectorOperation::VoiceEnd,
        };
        if manifest.address != connect_connector_api::AddressKind::E164Pair
            || manifest.name != origin.policy.provider
            || !manifest.operations.contains(&required)
        {
            return Err(Error::new(
                "unsupported-connector-operation",
                "Provider Session operation is not declared",
            ));
        }
        origin.lifetime(self)?;
        let authority = if action == ProviderSessionVoiceAction::Start {
            Some(self.stable_authority(
                &AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
                || {
                    let c = self.connection()?;
                    origin.current(self, &c)?;
                    scopes(&origin.source)
                },
            )?)
        } else {
            None
        };
        let owner = self.require_runtime_owner()?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        exact(&tx, origin)?;
        let (started,end_started,create_effect,provider_id,control_url,closed):(bool,bool,Option<String>,Option<String>,Option<String>,Option<i64>)=tx.query_row("SELECT create_started,end_started,create_effect_id,provider_id,control_url,closed_at FROM provider_session_voice WHERE id=?1",[&origin.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).map_err(|_|Error::storage())?;
        if closed.is_some() {
            return Ok(None);
        }
        let mut expires = crate::now() + 2;
        let mut quarantine = None;
        match action {
            ProviderSessionVoiceAction::Start => {
                if started {
                    return Ok(None);
                }
                expires = expires.min(origin.current(self, &tx)?);
                let (normal,total):(u64,u64)=tx.query_row("SELECT COALESCE(SUM(compensates_effect_id IS NULL),0),COUNT(*) FROM operation_effects WHERE operation_record_id=?1",[&origin.source.invitation_record_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::storage())?;
                if normal >= 32 || total >= 64 {
                    return Err(Error::new(
                        "capacity-exceeded",
                        "Provider Session effect capacity exhausted",
                    ));
                }
                let effect = format!("pvcreate_{}", origin.id);
                tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,submission_generation) VALUES(?1,?2,?3,?4,'none','submitting',1,1)",params![effect,origin.source.invitation_record_id,origin.policy.provider,origin.id]).map_err(|_|Error::storage())?;
                tx.execute("UPDATE provider_session_voice SET create_started=1,create_effect_id=?2,attempt_runtime=?3 WHERE id=?1",params![origin.id,effect,origin.runtime]).map_err(|_|Error::storage())?;
                touch(&tx, &origin.id, self)?;
            }
            ProviderSessionVoiceAction::Status => {
                if !started || provider_id.is_none() {
                    return Err(Error::new(
                        "reconciliation-required",
                        "Provider Session has no attributed resource to observe",
                    ));
                }
            }
            ProviderSessionVoiceAction::Reconcile => {
                let eligible: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM provider_session_voice p JOIN operation_effects e ON e.id=p.create_effect_id WHERE p.id=?1 AND p.direction='outbound' AND p.create_started=1 AND p.provider_id IS NULL AND p.quarantine_json IS NULL AND e.state='unknown')",[&origin.id],|r|r.get(0)).map_err(|_|Error::storage())?;
                if !eligible {
                    return Err(Error::new(
                        "reconciliation-required",
                        "Only an original uncertain, unattributed submission can be correlated",
                    ));
                }
            }
            ProviderSessionVoiceAction::ResolveQuarantine => {
                quarantine = Some(quarantine::admit(&tx, origin)?);
            }
            ProviderSessionVoiceAction::End => {
                if end_started {
                    return Ok(None);
                }
                if !started || provider_id.is_none() {
                    return Err(Error::new(
                        "reconciliation-required",
                        "Unknown provider creation cannot be blindly ended",
                    ));
                }
                let total: u64 = tx
                    .query_row(
                        "SELECT COUNT(*) FROM operation_effects WHERE operation_record_id=?1",
                        [&origin.source.invitation_record_id],
                        |r| r.get(0),
                    )
                    .map_err(|_| Error::storage())?;
                if total >= 64 {
                    return Err(Error::new(
                        "capacity-exceeded",
                        "Provider cleanup effect capacity exhausted",
                    ));
                }
                let effect = format!("pvend_{}", origin.id);
                tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,submission_generation,compensates_effect_id,provider_ref) VALUES(?1,?2,?3,'cleanup_'||?4,'none','submitting',1,1,?5,?6)",params![effect,origin.source.invitation_record_id,origin.policy.provider,origin.id,create_effect,provider_id]).map_err(|_|Error::storage())?;
                tx.execute("UPDATE provider_session_voice SET end_started=1,end_effect_id=?2,end_runtime=?3 WHERE id=?1",params![origin.id,effect,origin.runtime]).map_err(|_|Error::storage())?;
                touch(&tx, &origin.id, self)?;
            }
        }
        tx.commit().map_err(|_| Error::storage())?;
        Ok(Some(ProviderSessionVoiceDispatch {
            origin: origin.clone(),
            action,
            provider_id,
            control_url,
            quarantine,
            _authority: authority,
            expires,
            owner,
            manager: self.authority.clone(),
            worker: worker.clone(),
        }))
    }
    /// Persist an admitted historical response after its transport reader drops.
    /// This updates effects only; it grants no acceptance, media or fresh dispatch.
    pub fn record_provider_session_voice(
        &self,
        captured: &ProviderSessionVoiceCapture,
    ) -> Result<ProviderSessionVoiceView> {
        let origin = &captured.origin;
        if !captured.worker.belongs(self)
            || !Arc::ptr_eq(&self.authority, &captured.manager)
            || origin.runtime != self.runtime_epoch()
        {
            return Err(Error::forbidden());
        }
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        exact(&tx, origin)?;
        let (create_effect,end_effect,provider_id,attempt_runtime,end_runtime):(String,Option<String>,Option<String>,String,Option<String>)=tx.query_row("SELECT create_effect_id,end_effect_id,provider_id,attempt_runtime,end_runtime FROM provider_session_voice WHERE id=?1 AND create_started=1",[&origin.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|_|Error::forbidden())?;
        if (captured.action == ProviderSessionVoiceAction::Start
            && attempt_runtime != origin.runtime)
            || (captured.action == ProviderSessionVoiceAction::End
                && end_runtime.as_deref() != Some(origin.runtime.as_str()))
            || provider_id
                .as_ref()
                .zip(captured.provider_id.as_ref())
                .is_some_and(|(a, b)| a != b)
        {
            return Err(Error::forbidden());
        }
        // A failed read is not an authenticated observation or absence proof.
        // Retain the original unknown effect without manufacturing a new event.
        if matches!(
            captured.action,
            ProviderSessionVoiceAction::Reconcile | ProviderSessionVoiceAction::ResolveQuarantine
        ) && captured.provider_id.is_none()
        {
            drop(tx);
            return Self::provider_session_voice_view_internal(&c, origin);
        }
        if captured.action == ProviderSessionVoiceAction::ResolveQuarantine
            && quarantine::replay(&tx, captured)?
        {
            drop(tx);
            return Self::provider_session_voice_view_internal(&c, origin);
        }
        if captured.action == ProviderSessionVoiceAction::Reconcile
            && tx
                .query_row(
                    "SELECT quarantine_json IS NOT NULL FROM provider_session_voice WHERE id=?1",
                    [&origin.id],
                    |r| r.get::<_, bool>(0),
                )
                .map_err(|_| Error::storage())?
        {
            return Err(Error::new(
                "reconciliation-required",
                "An existing binding conflict requires explicit quarantine resolution",
            ));
        }
        let mut p = projection(&tx, &origin.id)?;
        // A provider identifier is attribution, never permission to take over
        // another logical call. Keep the actual receipt as private history and
        // quarantine this creation, without making its control URL dispatchable.
        let collision = if let Some(provider) = &captured.provider_id {
            tx.query_row("SELECT EXISTS(SELECT 1 FROM provider_session_voice WHERE id!=?1 AND provider_id=?2 AND json_extract(policy_json,'$.provider')=?3 AND json_extract(policy_json,'$.route_reference')=?4 AND json_extract(source_json,'$.route_owner_id')=?5)",params![origin.id,provider,origin.policy.provider,origin.policy.route_reference,origin.source.route_owner_id],|r|r.get::<_,bool>(0)).map_err(|_|Error::storage())?
        } else {
            false
        };
        if collision {
            if captured.action == ProviderSessionVoiceAction::ResolveQuarantine {
                drop(tx);
                return Self::provider_session_voice_view_internal(&c, origin);
            }
            let raw = crate::provisioning::canonical_json(
                &serde_json::json!({"observation":captured.observation,"provider_id":captured.provider_id,"control_url":captured.control_url,"ended_reason":captured.ended_reason}),
            )?;
            let previous: Option<String> = tx
                .query_row(
                    "SELECT quarantine_json FROM provider_session_voice WHERE id=?1",
                    [&origin.id],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            if let Some(previous) = previous {
                if previous != raw {
                    return Err(conflict());
                }
                drop(tx);
                return Self::provider_session_voice_view_internal(&c, origin);
            }
            // Retain the actual receipt in quarantine_json without attributing
            // its state to this call. Even a terminal receipt cannot settle an
            // unattributed creation or release its quota.
            // An already-unknown projection cannot accept a local attempt as
            // reconciliation. Its state is already correct; retain the actual
            // conflicting receipt below without inventing stronger evidence.
            if p.state != State::Voice(VoiceState::Uncertain) {
                p.apply(
                    Observation {
                        mutation_id: origin.id.clone(),
                        external_id: None,
                        event_id: format!("binding-conflict:{}", origin.id),
                        state: State::Voice(VoiceState::Uncertain),
                        verification: Verification::LocalAttempt,
                    },
                    true,
                )
                .map_err(|_| Error::storage())?;
            }
            effect_state(&tx, &create_effect, "unknown", None)?;
            tx.execute("UPDATE provider_session_voice SET projection_json=?2,quarantine_json=?3,quarantined_at=?4,observed_at=?4 WHERE id=?1",params![origin.id,encoded(&p)?,raw,crate::now()]).map_err(|_|Error::storage())?;
            touch(&tx, &origin.id, self)?;
            tx.commit().map_err(|_| Error::storage())?;
            return Self::provider_session_voice_view_internal(&c, origin);
        }
        // Authorization is the retained admitted response custody for this
        // immutable operation/resource. It never renews the caller's grant.
        if captured.action == ProviderSessionVoiceAction::ResolveQuarantine {
            quarantine::record(&tx, captured)?;
        }
        let applied = p.apply(captured.observation.clone(), true).map_err(|_| {
            Error::new(
                "reconciliation-required",
                "Provider Session observation conflicts or requires reconciliation",
            )
        })?;
        if applied == Apply::Duplicate {
            drop(tx);
            return Self::provider_session_voice_view_internal(&c, origin);
        }
        if applied == Apply::Older {
            // Retain its deduplication identity, without replacing current
            // private control capabilities or announcing stale public status.
            tx.execute(
                "UPDATE provider_session_voice SET projection_json=?2 WHERE id=?1",
                params![origin.id, encoded(&p)?],
            )
            .map_err(|_| Error::storage())?;
            if captured.action == ProviderSessionVoiceAction::End {
                if let Some(end) = &end_effect {
                    effect_state(&tx, end, "unknown", captured.provider_id.as_deref())?;
                    touch(&tx, &origin.id, self)?;
                }
            }
            tx.commit().map_err(|_| Error::storage())?;
            return Self::provider_session_voice_view_internal(&c, origin);
        }
        let terminal = p.state.terminal();
        let creating = matches!(captured.action, ProviderSessionVoiceAction::Start);
        let original_state = if creating
            && captured.observation.state == State::Voice(VoiceState::Failed)
            && captured.provider_id.is_none()
        {
            Some("failed")
        } else if creating
            && captured.observation.state == State::Voice(VoiceState::Uncertain)
            && captured.provider_id.is_none()
        {
            Some("unknown")
        } else if captured.provider_id.is_some() {
            Some("confirmed")
        } else {
            None
        };
        if let Some(state) = original_state {
            effect_state(&tx, &create_effect, state, captured.provider_id.as_deref())?;
        }
        if terminal && end_effect.is_none() && captured.provider_id.is_some() {
            let end = format!("pvend_{}", origin.id);
            tx.execute("INSERT INTO operation_effects(id,operation_record_id,provider,correlation_key,idempotency_mode,state,attempt,submission_generation,compensates_effect_id,provider_ref) VALUES(?1,?2,?3,'cleanup_'||?4,'status-query','confirmed',1,1,?5,?6)",params![end,origin.invitation_record_id(),origin.policy.provider,origin.id,create_effect,captured.provider_id]).map_err(|_|Error::storage())?;
            tx.execute("UPDATE provider_session_voice SET provider_id=COALESCE(provider_id,?2),end_started=1,end_effect_id=?3,end_runtime=?4 WHERE id=?1",params![origin.id,captured.provider_id,end,origin.runtime]).map_err(|_|Error::storage())?;
        }
        if let Some(end) = &end_effect {
            if terminal {
                effect_state(&tx, end, "confirmed", captured.provider_id.as_deref())?;
            } else if captured.action == ProviderSessionVoiceAction::End {
                effect_state(&tx, end, "unknown", captured.provider_id.as_deref())?;
            }
        }
        tx.execute("UPDATE provider_session_voice SET projection_json=?2,provider_id=COALESCE(provider_id,?3),control_url=COALESCE(?4,control_url),ended_reason=COALESCE(?5,ended_reason),observed_at=?6,closed_at=CASE WHEN ?7 THEN COALESCE(closed_at,?6) ELSE closed_at END WHERE id=?1",params![origin.id,encoded(&p)?,captured.provider_id,captured.control_url,captured.ended_reason,crate::now(),terminal]).map_err(|_|Error::storage())?;
        touch(&tx, &origin.id, self)?;
        tx.commit().map_err(|_| Error::storage())?;
        Self::provider_session_voice_view_internal(&c, origin)
    }
    fn provider_session_voice_view_internal(
        c: &Connection,
        origin: &ProviderSessionVoiceOrigin,
    ) -> Result<ProviderSessionVoiceView> {
        exact(c, origin)?;
        load_view(c, &origin.id)
    }
    /// Recover an exclusively owned previous worker's interrupted dispatch. Never
    /// retry create/end; polls can later supply explicitly reconciled evidence.
    pub fn recover_provider_session_voice(
        &self,
        worker: &ProviderSessionVoiceWorker,
    ) -> Result<usize> {
        let mut exclusive = worker.require_recovery(self)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let mut q=tx.prepare("SELECT id,invitation_record_id,create_effect_id,end_effect_id FROM provider_session_voice WHERE create_started=1 AND closed_at IS NULL AND (EXISTS(SELECT 1 FROM operation_effects e WHERE e.id=create_effect_id AND e.state='submitting') OR EXISTS(SELECT 1 FROM operation_effects e WHERE e.id=end_effect_id AND e.state='submitting'))").map_err(|_|Error::storage())?;
        let rows = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| Error::storage())?;
        drop(q);
        for (id, _record, create, end) in &rows {
            let mut p = projection(&tx, id)?;
            let event = Observation {
                mutation_id: id.clone(),
                external_id: p.external_id.clone(),
                event_id: format!("recovery:{id}:{}", self.runtime_epoch()),
                state: State::Voice(VoiceState::Uncertain),
                verification: Verification::Reconciliation,
            };
            p.apply(event, true).map_err(|_| Error::storage())?;
            tx.execute("UPDATE operation_effects SET state='unknown' WHERE (id=?1 OR id=?2) AND state='submitting'",params![create,end]).map_err(|_|Error::storage())?;
            tx.execute(
                "UPDATE provider_session_voice SET projection_json=?2,observed_at=?3 WHERE id=?1",
                params![id, encoded(&p)?, crate::now()],
            )
            .map_err(|_| Error::storage())?;
            touch(&tx, id, self)?;
        }
        tx.commit().map_err(|_| Error::storage())?;
        *exclusive = false;
        Ok(rows.len())
    }
    /// Only the same live owner may retire an intent that never acquired a
    /// dispatch. Submitted effects retain their original uncertainty and quota.
    pub fn abandon_provider_session_voice_unstarted(
        &self,
        origin: &ProviderSessionVoiceOrigin,
    ) -> Result<bool> {
        self.refuse_provider_session_voice_unstarted(origin, "not-started")
    }
    pub fn refuse_provider_session_voice_unstarted(
        &self,
        origin: &ProviderSessionVoiceOrigin,
        problem: &str,
    ) -> Result<bool> {
        if !matches!(
            problem,
            "not-started"
                | "cancelled"
                | "expired"
                | "forbidden"
                | "unauthorized"
                | "unsupported-connector-operation"
                | "unsupported-feature"
        ) {
            return Err(invalid());
        }
        origin.lifetime(self)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        exact(&tx, origin)?;
        let (started, closed): (bool, Option<i64>) = tx
            .query_row(
                "SELECT create_started,closed_at FROM provider_session_voice WHERE id=?1",
                [&origin.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| Error::storage())?;
        if started || closed.is_some() {
            return Ok(false);
        }
        let p = Projection::new(origin.id.clone(), State::Voice(VoiceState::Failed))
            .map_err(|_| Error::storage())?;
        tx.execute("UPDATE provider_session_voice SET projection_json=?2,closed_at=?3,observed_at=?3,refusal_code=?4 WHERE id=?1",params![origin.id,encoded(&p)?,crate::now(),problem]).map_err(|_|Error::storage())?;
        touch(&tx, &origin.id, self)?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(true)
    }
}
