//! Exact same-Unit positive native phase metadata; never an IO authorization.
use super::*;
use rusqlite::OptionalExtension;

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
pub(super) struct Active {
    key: Key,
    nonce: u64,
    root_revision: i64,
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
            let action_matches = match action {
                "effect-birth" | "effect-submit" => key.stage == Stage::Begin,
                "prepare-step-start" | "prepare-effect-submit" => {
                    key.phase == "prepare" && key.stage == Stage::Begin
                }
                "effect-confirm" => key.stage == Stage::Confirm,
                "root-touch-begin" => key.stage == Stage::Begin,
                "root-touch-confirm" => key.stage == Stage::Confirm,
                _ => false,
            };
            let revision_matches = if matches!(action, "root-touch-begin" | "root-touch-confirm") {
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
        if !Arc::ptr_eq(self, &store.authority_writer_boundary)
            || !matches!(key.phase, "prepare" | "attach" | "readiness")
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
        if key.phase == "prepare" {
            let valid: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM native_replacement_steps s JOIN native_replacements n ON n.operation_record_id=s.operation_record_id JOIN operations o ON o.record_id=n.operation_record_id JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=s.operation_record_id WHERE s.operation_record_id=?1 AND s.phase='prepare' AND s.attempt_id=?2 AND s.effect_id=?3 AND s.started=0 AND s.observation_json IS NULL AND o.lease_owner=n.reservation_id AND o.lease_expires_at=n.expires_at AND n.expires_at>?4 AND e.provider='native-replacement-prepare' AND e.correlation_key=s.operation_record_id AND e.idempotency_mode='none' AND e.state='prepared' AND e.attempt=0 AND e.submission_generation=0 AND e.provider_ref IS NULL AND e.observation_json IS NULL AND e.compensates_effect_id IS NULL AND NOT EXISTS(SELECT 1 FROM native_replacement_steps later WHERE later.operation_record_id=s.operation_record_id AND later.phase!='prepare'))",
                rusqlite::params![key.record,key.attempt,key.effect,crate::now()], |r|r.get(0),
            ).map_err(|_|Error::storage())?;
            if key.stage != Stage::Begin || !key.expected.is_empty() || !valid {
                return Err(Error::forbidden());
            }
        } else {
            let expected = crate::provisioning::canonical_json(
                &serde_json::json!({"candidate":candidate,"generation":generation,"phase":key.phase,"attached":true,"private":true,"activated_muted":key.phase!="attach","media_ready":key.phase=="readiness"}),
            )?;
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
            _proof: proof.clone(),
        });
        Ok(Permit {
            state: self.clone(),
            nonce,
            _connection: std::marker::PhantomData,
        })
    }
}
