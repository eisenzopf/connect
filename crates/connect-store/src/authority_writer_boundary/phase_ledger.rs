//! Exact same-Unit positive native phase metadata; never an IO authorization.
use super::*;
use rusqlite::OptionalExtension;
pub(crate) mod commit;
mod intent;
pub(crate) mod queued;
pub(crate) mod reservation;
pub(crate) mod scoped;

pub(crate) const FUNCTION: &str = "connect_store_native_phase_ledger_v1";
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stage {
    Begin,
    Confirm,
}
pub(crate) struct Key {
    pub record: String,
    pub phase: &'static str,
    pub attempt: String,
    pub effect: String,
    pub expected: String,
    pub runtime: String,
    pub reservation: String,
    pub digest: String,
    pub stage: Stage,
}
struct ResourceTarget<'a>(Option<&'a str>, Option<&'a crate::PeerAuthorityGuard>);
pub(super) struct Active {
    key: Key,
    nonce: u64,
    root_revision: i64,
    reservation_root: Option<String>,
    candidate_audience: Option<String>,
    commit_capture: Option<commit::Capture>,
    scoped_capture: Option<scoped::Capture>,
    // Pins the original complete Read until this transaction permit ends.
    _proof: Arc<AuthorityGuardSet>,
}
pub(crate) struct Permit<'a> {
    state: Arc<State>,
    nonce: u64,
    _connection: std::marker::PhantomData<&'a Connection>,
}
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let previous = match self.state.slot.lock() {
            Ok(mut slot) => {
                if slot
                    .phase_ledger
                    .as_ref()
                    .is_some_and(|p| p.nonce == self.nonce)
                {
                    slot.phase_ledger.take()
                } else {
                    None
                }
            }
            Err(_) => {
                if let Some(registry) = self.state.registry.upgrade() {
                    registry.invalidate_all();
                }
                None
            }
        };
        // A stronger original proof may contain a WriterNotice. Never release
        // its last reference while the SQL boundary's slot mutex is held.
        drop(previous);
    }
}
impl State {
    pub(super) fn register_phase_ledger(self: &Arc<Self>, connection: &Connection) -> Result<()> {
        let state = self.clone();
        connection
            .create_scalar_function(FUNCTION, 7, FunctionFlags::SQLITE_UTF8, move |context| {
                let action = context.get::<String>(0)?;
                let record = context.get::<String>(1)?;
                let effect = context.get::<Option<String>>(2)?;
                let phase = context.get::<Option<String>>(3)?;
                let expected = context.get::<Option<String>>(4)?;
                let attempt = context.get::<Option<String>>(5)?;
                let revision = context.get::<Option<i64>>(6)?;
                Ok(i64::from(state.phase_ledger_matches(
                    &action,
                    &record,
                    effect.as_deref(),
                    phase.as_deref(),
                    expected.as_deref(),
                    attempt.as_deref(),
                    revision,
                )))
            })
            .map_err(|_| Error::storage())
    }
    fn phase_ledger_matches(
        &self,
        action: &str,
        record: &str,
        effect: Option<&str>,
        phase: Option<&str>,
        expected: Option<&str>,
        attempt: Option<&str>,
        revision: Option<i64>,
    ) -> bool {
        self.slot.lock().is_ok_and(|slot| {
            if !slot.active || slot.failed {
                return false;
            }
            let Some(active) = &slot.phase_ledger else {
                return false;
            };
            let key = &active.key;
            if active.scoped_capture.is_some() {
                return scoped::matches(
                    active, action, record, effect, phase, expected, attempt, revision,
                );
            }
            if key.phase == "commit" {
                return commit::matches(
                    active, action, record, effect, phase, expected, attempt, revision,
                );
            }
            if key.phase == "intent-birth" {
                return phase == Some("intent-birth")
                    && effect.is_none()
                    && attempt.is_none()
                    && revision.is_none()
                    && intent::matches(active, action, record, expected);
            }
            if key.phase == "data-queued" {
                return queued::matches(
                    active, action, record, effect, phase, expected, attempt, revision,
                );
            }
            if key.phase == "reserve" {
                return key.record == record
                    && phase == Some("reserve")
                    && active.reservation_matches(action, effect, expected, attempt, revision);
            }
            if action == "candidate-activation-audience" {
                return key.phase == "activate"
                    && key.stage == Stage::Confirm
                    && key.record == record
                    && phase == Some("activate")
                    && effect == Some(key.effect.as_str())
                    && expected == active.candidate_audience.as_deref()
                    && expected.is_some()
                    && attempt.is_none()
                    && revision.is_none();
            }
            let action_matches = match action {
                "effect-birth"
                | "effect-submit"
                | "resource-step-birth"
                | "resource-step-start" => key.stage == Stage::Begin,
                "prepare-step-start" | "prepare-effect-submit" => {
                    key.phase == "prepare" && key.stage == Stage::Begin
                }
                "effect-confirm" => key.stage == Stage::Confirm,
                "resource-step-confirm" | "resource-effect-confirm" | "resource-root-touch" => {
                    matches!(key.phase, "prepare" | "allocate" | "start")
                        && key.stage == Stage::Confirm
                }
                "root-touch-begin" => key.stage == Stage::Begin,
                "root-touch-confirm" => key.stage == Stage::Confirm,
                _ => false,
            };
            let revision_matches = if matches!(
                action,
                "root-touch-begin" | "root-touch-confirm" | "resource-root-touch"
            ) {
                active.root_revision.checked_add(1) == revision
            } else {
                revision.is_none()
            };
            action_matches
                && revision_matches
                && key.record == record
                && effect == Some(key.effect.as_str())
                && phase == Some(key.phase)
                && expected == Some(key.expected.as_str())
                && if action == "effect-birth" {
                    attempt.is_none()
                } else {
                    attempt == Some(key.attempt.as_str())
                }
        })
    }
    /// Called only by fixed phase methods AFTER BEGIN IMMEDIATE and complete
    /// original fact validation. No caller-supplied callback or SQL is exposed.
    pub(crate) fn native_phase_ledger<'a>(
        self: &Arc<Self>,
        store: &Store,
        connection: &'a Connection,
        proof: &Arc<AuthorityGuardSet>,
        key: Key,
    ) -> Result<Permit<'a>> {
        self.native_phase_ledger_in(store, connection, proof, key, None, None)
    }

    /// Historical immutable capture only. The opaque receipt supplies the
    /// previously submitted target descriptor separately from its observation.
    pub(crate) fn native_resource_capture_ledger<'a>(
        self: &Arc<Self>,
        store: &Store,
        connection: &'a Connection,
        proof: &Arc<AuthorityGuardSet>,
        key: Key,
        target: Option<&str>,
    ) -> Result<Permit<'a>> {
        if key.stage != Stage::Confirm
            || !matches!(
                (key.phase, target),
                ("prepare", None) | ("allocate" | "start", Some(_))
            )
        {
            return Err(Error::forbidden());
        }
        self.native_phase_ledger_in(
            store,
            connection,
            proof,
            key,
            Some(ResourceTarget(target, None)),
            None,
        )
    }

    /// Exact signed dormant resource journal Begin. The fixed caller retains
    /// its complete current source/target Read through this original SQL unit.
    pub(crate) fn native_resource_begin_ledger<'a>(
        self: &Arc<Self>,
        store: &Store,
        connection: &'a Connection,
        proof: &crate::PeerAuthorityGuard,
        source: &'a crate::NativeReplacementPreparationRead,
        key: Key,
        target: &str,
    ) -> Result<Permit<'a>> {
        if key.stage != Stage::Begin
            || !matches!(key.phase, "allocate" | "start")
            || key.expected != target
            || !proof.current()
        {
            return Err(Error::forbidden());
        }
        if !source.current() || source.selection().origin.record_id() != key.record {
            return Err(Error::forbidden());
        }
        let required = source.selection().scopes(store, connection, false)?;
        let (_, _, signed, _) = crate::peer_authority::checked(store, connection, &proof.request)?;
        if !store.authority_covers(Store::peer_scopes(proof), &required)
            || !store.authority_covers(Store::peer_scopes(proof), &signed)
        {
            return Err(Error::forbidden());
        }
        self.native_phase_ledger_in(
            store,
            connection,
            Store::peer_scopes(proof),
            key,
            Some(ResourceTarget(Some(target), Some(proof))),
            None,
        )
    }

    /// Exact dormant candidate-only activation bookkeeping. The complete scene
    /// is checked by the fixed Store entry point before this private Unit permit.
    pub(crate) fn native_candidate_activation_ledger<'a>(
        self: &Arc<Self>,
        store: &Store,
        connection: &'a Connection,
        proof: &Arc<AuthorityGuardSet>,
        key: Key,
        audience: Option<&crate::NativeReplacementAudienceCapture>,
    ) -> Result<Permit<'a>> {
        if key.phase != "activate" || (key.stage == Stage::Confirm) != audience.is_some() {
            return Err(Error::forbidden());
        }
        self.native_phase_ledger_in(store, connection, proof, key, None, Some(audience))
    }

    fn native_phase_ledger_in<'a>(
        self: &Arc<Self>,
        store: &Store,
        connection: &'a Connection,
        proof: &Arc<AuthorityGuardSet>,
        key: Key,
        resource_target: Option<ResourceTarget<'_>>,
        candidate_activation: Option<Option<&crate::NativeReplacementAudienceCapture>>,
    ) -> Result<Permit<'a>> {
        if !Arc::ptr_eq(self, &store.authority_writer_boundary)
            || (candidate_activation.is_none()
                && resource_target.is_none()
                && !matches!(key.phase, "prepare" | "attach" | "readiness"))
            || key.record.is_empty()
            || key.effect.is_empty()
            || key.attempt.is_empty()
            || connection
                .transaction_state(Some(rusqlite::DatabaseName::Main))
                .map_err(|_| Error::storage())?
                != rusqlite::TransactionState::Write
        {
            return Err(Error::forbidden());
        }
        let tag = connection as *const Connection as usize;
        let available = |slot: &Slot| {
            slot.active
                && !slot.failed
                && slot.connection == tag
                && slot.birth.is_none()
                && slot.phase_ledger.is_none()
        };
        {
            let slot = self.slot.lock().map_err(|_| Error::storage())?;
            if !available(&slot) {
                return Err(Error::forbidden());
            }
        }
        let root: Option<(String,String,String,u64,i64)> = connection.query_row(
            "SELECT o.owner_id,o.conversation_id,n.candidate_runtime_id,n.generation,o.revision FROM operations o JOIN native_replacements n ON n.operation_record_id=o.record_id WHERE o.record_id=?1 AND o.command_type='connection.replace' AND o.state='preparing' AND o.cancel_requested=0 AND o.tombstoned=0 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4 AND NOT EXISTS(SELECT 1 FROM native_replacement_steps s WHERE s.operation_record_id=o.record_id AND s.phase='commit' AND s.started=1)",
            rusqlite::params![key.record,key.runtime,key.reservation,key.digest], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(|_|Error::storage())?;
        let (owner, cid, candidate, generation, root_revision) =
            root.ok_or_else(Error::forbidden)?;
        if !store.authority_covers(
            proof,
            &[
                AuthorityScope::identity(&owner, AuthorityMode::Read)?,
                AuthorityScope::conversation(&cid, AuthorityMode::Read)?,
            ],
        ) {
            return Err(Error::forbidden());
        }
        let candidate_audience = if let Some(audience) = candidate_activation {
            let exact_lease: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id WHERE n.operation_record_id=?1 AND o.lease_owner=n.reservation_id AND o.lease_expires_at=n.expires_at AND (?2=0 OR n.expires_at>?3) AND NOT EXISTS(SELECT 1 FROM native_replacement_audiences a WHERE a.operation_record_id=n.operation_record_id AND a.phase='activate'))",
                rusqlite::params![key.record, key.stage == Stage::Begin, crate::now()], |row| row.get(0),
            ).map_err(|_| Error::storage())?;
            if !exact_lease {
                return Err(Error::forbidden());
            }
            let connection_id: String = connection.query_row(
                "SELECT candidate_connection_id FROM native_replacements WHERE operation_record_id=?1 AND NOT EXISTS(SELECT 1 FROM connections c WHERE c.id=native_replacements.candidate_connection_id AND c.state IN ('ready','active','held'))",
                [&key.record], |row| row.get(0),
            ).optional().map_err(|_| Error::storage())?.ok_or_else(Error::forbidden)?;
            audience
                .map(|capture| {
                    capture.candidate_activation_json(
                        &key.record,
                        &key.runtime,
                        &candidate,
                        &connection_id,
                        generation,
                    )
                })
                .transpose()?
        } else {
            None
        };
        if let Some(ResourceTarget(target, begin_proof)) = resource_target {
            if key.stage == Stage::Begin {
                self.require_resource_begin(
                    store,
                    connection,
                    &key,
                    begin_proof.ok_or_else(Error::forbidden)?,
                    target.ok_or_else(Error::forbidden)?,
                )?;
            } else {
                let plan: Option<String> = connection.query_row(
                "SELECT n.plan_json FROM native_replacement_steps s JOIN native_replacements n ON n.operation_record_id=s.operation_record_id JOIN operations o ON o.record_id=n.operation_record_id JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=s.operation_record_id WHERE s.operation_record_id=?1 AND s.phase=?2 AND s.attempt_id=?3 AND s.effect_id=?4 AND s.started=1 AND s.observation_json IS NULL AND o.lease_owner=n.reservation_id AND o.lease_expires_at=n.expires_at AND e.provider='native-replacement-'||s.phase AND e.correlation_key=CASE s.phase WHEN 'prepare' THEN s.operation_record_id ELSE e.id END AND e.idempotency_mode='none' AND e.state='submitting' AND e.attempt=1 AND e.submission_generation=1 AND e.provider_ref IS NULL AND e.observation_json IS ?5 AND e.compensates_effect_id IS NULL AND NOT EXISTS(SELECT 1 FROM native_replacement_steps later WHERE later.operation_record_id=s.operation_record_id AND (later.phase='commit' OR (s.phase='allocate' AND later.phase='start') OR (s.phase='prepare' AND later.phase!='prepare')))",
                rusqlite::params![key.record,key.phase,key.attempt,key.effect,target], |r|r.get(0),
            ).optional().map_err(|_|Error::storage())?;
                let plan: serde_json::Value =
                    serde_json::from_str(&plan.ok_or_else(Error::forbidden)?)
                        .map_err(|_| Error::storage())?;
                let expected = match (key.phase, target) {
                    ("prepare", None) => plan["expected_preparation"].clone(),
                    ("allocate", Some(target)) => {
                        let target: serde_json::Value =
                            serde_json::from_str(target).map_err(|_| Error::storage())?;
                        let mut expected = plan["expected_preparation"].clone();
                        expected["target_operation_id"] = target["intent"]["operation_id"].clone();
                        expected["target_key_id"] = target["key_id"].clone();
                        expected["allocated"] = serde_json::json!(true);
                        expected
                    }
                    ("start", Some(target)) => {
                        let target: serde_json::Value =
                            serde_json::from_str(target).map_err(|_| Error::storage())?;
                        let streams = target["intent"]["offer"]["streams_offered"]
                            .as_array()
                            .ok_or_else(Error::forbidden)?
                            .iter()
                            .map(|stream| stream["id"].clone())
                            .collect::<Vec<_>>();
                        serde_json::json!({"candidate":candidate,"generation":generation,"stream_revision":2,"streams":streams,"resources_prepared":true,"prepared_muted":true,"media_ready":false})
                    }
                    _ => return Err(Error::forbidden()),
                };
                if !expected.is_object()
                    || key.expected != crate::provisioning::canonical_json(&expected)?
                {
                    return Err(Error::forbidden());
                }
            }
        } else if key.phase == "prepare" {
            let valid: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM native_replacement_steps s JOIN native_replacements n ON n.operation_record_id=s.operation_record_id JOIN operations o ON o.record_id=n.operation_record_id JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=s.operation_record_id WHERE s.operation_record_id=?1 AND s.phase='prepare' AND s.attempt_id=?2 AND s.effect_id=?3 AND s.started=0 AND s.observation_json IS NULL AND o.lease_owner=n.reservation_id AND o.lease_expires_at=n.expires_at AND n.expires_at>?4 AND e.provider='native-replacement-prepare' AND e.correlation_key=s.operation_record_id AND e.idempotency_mode='none' AND e.state='prepared' AND e.attempt=0 AND e.submission_generation=0 AND e.provider_ref IS NULL AND e.observation_json IS NULL AND e.compensates_effect_id IS NULL AND NOT EXISTS(SELECT 1 FROM native_replacement_steps later WHERE later.operation_record_id=s.operation_record_id AND later.phase!='prepare'))",
                rusqlite::params![key.record,key.attempt,key.effect,crate::now()], |r|r.get(0),
            ).map_err(|_|Error::storage())?;
            if key.stage != Stage::Begin || !key.expected.is_empty() || !valid {
                return Err(Error::forbidden());
            }
        } else {
            let mut observation = serde_json::json!({"candidate":candidate,"generation":generation,"phase":key.phase,"attached":true,"private":true,"activated_muted":key.phase!="attach","media_ready":key.phase=="readiness"});
            if candidate_activation.is_some() {
                observation["candidate_only"] = serde_json::json!(true);
            }
            let expected = crate::provisioning::canonical_json(&observation)?;
            if key.expected != expected {
                return Err(Error::forbidden());
            }
            let valid: bool = match key.stage {
            Stage::Begin => connection.query_row("SELECT NOT EXISTS(SELECT 1 FROM native_replacement_media_steps WHERE operation_record_id=?1 AND phase=?2) AND NOT EXISTS(SELECT 1 FROM operation_effects WHERE id=?3) AND NOT EXISTS(SELECT 1 FROM native_replacement_media_steps WHERE attempt_id=?4 OR effect_id=?3)",rusqlite::params![key.record,key.phase,key.effect,key.attempt],|r|r.get(0)),
            Stage::Confirm => connection.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_media_steps s JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=s.operation_record_id WHERE s.operation_record_id=?1 AND s.phase=?2 AND s.attempt_id=?3 AND s.effect_id=?4 AND s.started=1 AND s.observation_json IS NULL AND e.provider='native-replacement-'||s.phase AND e.correlation_key=e.id AND e.idempotency_mode='none' AND e.state='submitting' AND e.attempt=1 AND e.submission_generation=1 AND e.observation_json=?5 AND e.provider_ref IS NULL AND e.compensates_effect_id IS NULL)",rusqlite::params![key.record,key.phase,key.attempt,key.effect,key.expected],|r|r.get(0)),
        }.map_err(|_|Error::storage())?;
            if !valid {
                return Err(Error::forbidden());
            }
            if candidate_activation.is_some() {
                let attached: bool = connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM native_replacement_media_steps s JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=s.operation_record_id WHERE s.operation_record_id=?1 AND s.phase='attach' AND s.started=1 AND s.observation_json IS NOT NULL AND s.observation_json=e.observation_json AND e.state='confirmed')",
                    [&key.record], |row| row.get(0),
                ).map_err(|_| Error::storage())?;
                if !attached {
                    return Err(Error::forbidden());
                }
            }
            if key.stage == Stage::Begin && key.phase == "readiness" {
                let previous:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM native_replacement_media_steps s JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=s.operation_record_id WHERE s.operation_record_id=?1 AND s.phase='activate' AND s.started=1 AND s.observation_json IS NOT NULL AND s.observation_json=e.observation_json AND e.state='confirmed')",[&key.record],|r|r.get(0)).map_err(|_|Error::storage())?;
                if !previous {
                    return Err(Error::forbidden());
                }
            }
        }
        let mut slot = self.slot.lock().map_err(|_| Error::storage())?;
        if !available(&slot) || connection.is_autocommit() {
            return Err(Error::forbidden());
        }
        let nonce = slot
            .next_birth_nonce
            .checked_add(1)
            .ok_or_else(Error::storage)?;
        slot.next_birth_nonce = nonce;
        slot.phase_ledger = Some(Active {
            key,
            nonce,
            root_revision,
            reservation_root: None,
            candidate_audience,
            commit_capture: None,
            scoped_capture: None,
            _proof: proof.clone(),
        });
        Ok(Permit {
            state: self.clone(),
            nonce,
            _connection: std::marker::PhantomData,
        })
    }
    fn require_resource_begin(
        &self,
        store: &Store,
        c: &Connection,
        key: &Key,
        proof: &crate::PeerAuthorityGuard,
        target: &str,
    ) -> Result<()> {
        let descriptor: serde_json::Value =
            serde_json::from_str(target).map_err(|_| Error::forbidden())?;
        if crate::provisioning::canonical_json(&descriptor)? != target {
            return Err(Error::forbidden());
        }
        let credential = descriptor["credential_id"]
            .as_str()
            .ok_or_else(Error::forbidden)?;
        store.require_peer_source(c, proof, credential)?;
        let crate::PeerCommand::NativeConnectionOffer(intent) = &proof.request.command else {
            return Err(Error::forbidden());
        };
        let mut intent = intent.clone();
        intent.normalize()?;
        let profile = if key.phase == "allocate" {
            "connect-replacement-preparation-v1"
        } else {
            "connect-replacement-media-v1"
        };
        let saved: Option<(String, String, String, String)> = c.query_row(
            "SELECT n.plan_json,o.operation_id,o.conversation_id,o.session_id FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id WHERE n.operation_record_id=?1 AND o.command_type='connection.replace' AND o.state='preparing' AND o.cancel_requested=0 AND o.tombstoned=0 AND o.worker_runtime=?2 AND o.lease_owner=n.reservation_id AND o.lease_expires_at=n.expires_at AND n.runtime_epoch=?3 AND n.reservation_id=?4 AND n.plan_digest=?5 AND n.expires_at>?6 AND o.expires_at>?6 AND NOT EXISTS(SELECT 1 FROM connections live WHERE live.id=n.candidate_connection_id AND live.state IN ('ready','active','held')) AND NOT EXISTS(SELECT 1 FROM native_replacement_lifetimes live WHERE live.operation_record_id=n.operation_record_id) AND NOT EXISTS(SELECT 1 FROM native_replacement_steps later WHERE later.operation_record_id=n.operation_record_id AND (later.phase='commit' OR later.phase=?7 OR (?7='allocate' AND later.phase='start'))) AND NOT EXISTS(SELECT 1 FROM native_replacement_media_steps later WHERE later.operation_record_id=n.operation_record_id) AND NOT EXISTS(SELECT 1 FROM client_enrollments e WHERE e.tenant_id=o.tenant_id AND e.issuer_identity_id=o.owner_id AND e.complete_operation_id=o.operation_id)",
            rusqlite::params![key.record,store.transition_runtime,key.runtime,key.reservation,key.digest,crate::now(),key.phase],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
        ).optional().map_err(|_| Error::storage())?;
        let (saved, operation, cid, sid) = saved.ok_or_else(Error::forbidden)?;
        let plan: serde_json::Value = serde_json::from_str(&saved).map_err(|_| Error::storage())?;
        let expected = serde_json::json!({"intent":intent,"credential_id":credential,"key_id":proof.request.key_id,"algorithm":proof.request.algorithm,"public_key":proof.key.public_key,"device_id":proof.key.device_id,"endpoint_id":plan["expected_preparation"]["endpoint"],"endpoint_revision":plan["target_endpoint_revision"]});
        if descriptor != expected
            || !proof
                .request
                .credentials
                .iter()
                .any(|value| value == credential)
            || intent.offer["replacement_preparation"]
                .as_object()
                .map(|value| value.len())
                != Some(2)
            || intent.offer["replacement_preparation"]["profile"] != profile
            || intent.offer["replacement_preparation"]["operation_record_id"] != key.record
            || intent.operation_id != operation
            || intent.conversation_id != cid
            || intent.session_id != sid
            || intent.connection_id != plan["candidate_connection_id"]
            || intent.expected_revision != Some(0)
            || proof.key.device_id != plan["expected_preparation"]["device"]
            || proof.request.device.id != proof.key.device_id
        {
            return Err(Error::forbidden());
        }
        let predecessor = if key.phase == "allocate" {
            "prepare"
        } else {
            "allocate"
        };
        let mut previous = plan["expected_preparation"].clone();
        let previous_target = if key.phase == "start" {
            previous["target_operation_id"] = descriptor["intent"]["operation_id"].clone();
            previous["target_key_id"] = descriptor["key_id"].clone();
            previous["allocated"] = serde_json::json!(true);
            let mut original = descriptor.clone();
            original["intent"]["offer"]["replacement_preparation"]["profile"] =
                serde_json::json!("connect-replacement-preparation-v1");
            Some(crate::provisioning::canonical_json(&original)?)
        } else {
            None
        };
        let previous = crate::provisioning::canonical_json(&previous)?;
        let confirmed: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=s.operation_record_id WHERE s.operation_record_id=?1 AND s.phase=?2 AND s.started=1 AND s.observation_json=?3 AND e.provider='native-replacement-'||s.phase AND e.correlation_key=CASE s.phase WHEN 'prepare' THEN s.operation_record_id ELSE e.id END AND e.idempotency_mode='none' AND e.state='confirmed' AND e.attempt=1 AND e.submission_generation=1 AND e.observation_json IS ?4 AND e.provider_ref IS NULL AND e.compensates_effect_id IS NULL)",
            rusqlite::params![key.record,predecessor,previous,previous_target], |row| row.get(0),
        ).map_err(|_| Error::storage())?;
        let unused: bool = c.query_row(
            "SELECT NOT EXISTS(SELECT 1 FROM operation_effects e WHERE e.id=?1 OR (e.operation_record_id=?2 AND e.provider='native-replacement-'||?3)) AND NOT EXISTS(SELECT 1 FROM native_replacement_steps s WHERE s.attempt_id=?4 OR s.effect_id=?1)",
            rusqlite::params![key.effect,key.record,key.phase,key.attempt], |row| row.get(0),
        ).map_err(|_| Error::storage())?;
        if !confirmed || !unused || !proof.current() {
            return Err(Error::forbidden());
        }
        Ok(())
    }
}
