//! One current, exact, positive historical Commit receipt; never routing authority.
use super::*;

pub(super) struct Capture {
    audience: String,
    connections: Vec<String>,
    until: i64,
    recorded_at: i64,
    root_touch: String,
}
#[derive(Clone, Copy)]
pub(crate) struct Decision {
    pub(crate) reader: bool,
    pub(crate) reason: &'static str,
}
struct Facts {
    capture: Capture,
    required: Vec<AuthorityScope>,
    revision: i64,
}

fn decision_facts(
    store: &Store,
    c: &Connection,
    captured: &crate::NativeReplacementCommitted,
    held: Option<&Arc<AuthorityGuardSet>>,
) -> Result<(Option<Facts>, &'static str)> {
    let key = captured.commit_ledger_key();
    if key.runtime != store.runtime_epoch() {
        return Ok((None, "runtime-mismatch"));
    }
    let Some(audience) = captured.commit_ledger_audience() else {
        return Ok((None, "audience-absent"));
    };
    let row: Option<(i64,String,String,u64,i64)> = c.query_row(
        "SELECT o.revision,n.candidate_connection_id,n.candidate_runtime_id,n.generation,min(o.expires_at,n.expires_at) FROM operations o JOIN native_replacements n ON n.operation_record_id=o.record_id JOIN native_replacement_steps s ON s.operation_record_id=o.record_id AND s.phase='commit' JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=o.record_id WHERE o.record_id=?1 AND o.command_type='connection.replace' AND o.state='preparing' AND o.cancel_requested=0 AND o.tombstoned=0 AND n.runtime_epoch=?2 AND n.reservation_id=?3 AND n.plan_digest=?4 AND o.worker_runtime=?5 AND o.lease_owner=n.reservation_id AND o.lease_expires_at=n.expires_at AND o.expires_at>?6 AND n.expires_at>?6 AND s.attempt_id=?7 AND s.effect_id=?8 AND s.started=1 AND s.observation_json IS NULL AND e.provider='native-replacement-commit' AND e.correlation_key=e.id AND e.idempotency_mode='none' AND e.state='submitting' AND e.attempt=1 AND e.submission_generation=1 AND e.provider_ref IS NULL AND e.compensates_effect_id IS NULL AND e.observation_json=?9 AND NOT EXISTS(SELECT 1 FROM native_replacement_lifetimes l WHERE l.operation_record_id=o.record_id) AND NOT EXISTS(SELECT 1 FROM native_replacement_audiences a WHERE a.operation_record_id=o.record_id AND a.phase='commit')",
        rusqlite::params![key.record,key.runtime,key.reservation,key.digest,store.transition_runtime,crate::now(),key.attempt,key.effect,captured.commit_ledger_descriptor()],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
    ).optional().map_err(|_|Error::storage())?;
    let Some((revision, candidate, runtime, generation, until)) = row else {
        return Ok((None, "root-effect-not-current"));
    };
    // A known outcome after revocation/expiry is still historical, but must
    // retain the existing Writer path. Never mint this permit from receipt IDs.
    let (required, current_until, basis) =
        match captured.commit_ledger_authority_decision(store, c, held) {
            Ok(facts) => facts,
            Err(e)
                if matches!(
                    e.code.as_str(),
                    "forbidden" | "unauthorized" | "not-ready" | "conflict"
                ) =>
            {
                return Ok((
                    None,
                    match e.code.as_str() {
                        "unauthorized" => "current-authority-unauthorized",
                        "not-ready" => "current-authority-not-ready",
                        "conflict" => "current-authority-conflict",
                        _ => "current-authority-forbidden",
                    },
                ));
            }
            Err(e) => return Err(e),
        };
    let Some((audience, connections)) = audience.positive_commit_json(
        store,
        c,
        &key.record,
        &key.runtime,
        &candidate,
        &runtime,
        generation,
    )?
    else {
        return Ok((None, "audience-delta-not-exact"));
    };
    let until = until.min(current_until);
    if crate::now() >= until {
        return Ok((None, "original-expired"));
    }
    let recorded_at = crate::now();
    let root_touch: String = c
        .query_row(
            "SELECT json_array(?1,?2)",
            rusqlite::params![key.expected, recorded_at],
            |r| r.get(0),
        )
        .map_err(|_| Error::storage())?;
    Ok((
        Some(Facts {
            capture: Capture {
                audience,
                connections,
                until,
                recorded_at,
                root_touch,
            },
            required,
            revision,
        }),
        basis,
    ))
}

/// Effect-free eligibility only. The exact constructor repeats it under the
/// same actual primary Unit transaction and pins the complete original Read.
pub(crate) fn eligible(
    store: &Store,
    c: &Connection,
    captured: &crate::NativeReplacementCommitted,
) -> Result<bool> {
    Ok(decision(store, c, captured)?.reader)
}

/// Closed metadata labels, never authority or a replacement for same-Unit facts.
pub(crate) fn decision(
    store: &Store,
    c: &Connection,
    captured: &crate::NativeReplacementCommitted,
) -> Result<Decision> {
    Ok(admission_preview(store, c, captured)?.0)
}

/// Effect-free scope metadata from the SAME complete eligibility derivation.
/// These are dependencies for a fresh admission, never a held proof or grant.
/// The real same-Unit constructor repeats every current fact independently.
pub(crate) fn admission_preview(
    store: &Store,
    c: &Connection,
    captured: &crate::NativeReplacementCommitted,
) -> Result<(Decision, Vec<AuthorityScope>)> {
    let (facts, reason) = decision_facts(store, c, captured, None)?;
    let decision = Decision {
        reader: facts.is_some(),
        reason,
    };
    Ok((
        decision,
        facts.map(|facts| facts.required).unwrap_or_default(),
    ))
}

pub(super) fn matches(
    active: &Active,
    action: &str,
    record: &str,
    effect: Option<&str>,
    phase: Option<&str>,
    expected: Option<&str>,
    attempt: Option<&str>,
    revision: Option<i64>,
) -> bool {
    let key = &active.key;
    let Some(capture) = &active.commit_capture else {
        return false;
    };
    if key.phase != "commit"
        || key.stage != Stage::Confirm
        || key.record != record
        || phase != Some("commit")
        || effect != Some(key.effect.as_str())
        || crate::now() >= capture.until
    {
        return false;
    }
    match action {
        "commit-audience" => {
            attempt.is_none() && revision.is_none() && expected == Some(capture.audience.as_str())
        }
        "commit-connection-revision" => {
            attempt.is_none()
                && revision.is_none()
                && expected.is_some_and(|v| capture.connections.iter().any(|s| s == v))
        }
        "commit-step-confirm" | "commit-effect-confirm" => {
            attempt == Some(key.attempt.as_str())
                && revision.is_none()
                && expected == Some(key.expected.as_str())
        }
        "commit-root-touch" => {
            attempt == Some(key.attempt.as_str())
                && active.root_revision.checked_add(1) == revision
                && expected == Some(capture.root_touch.as_str())
        }
        _ => false,
    }
}

impl State {
    pub(crate) fn native_commit_capture_ledger<'a>(
        self: &Arc<Self>,
        store: &Store,
        c: &'a Connection,
        proof: &Arc<AuthorityGuardSet>,
        captured: &crate::NativeReplacementCommitted,
    ) -> Result<Permit<'a>> {
        let tag = c as *const Connection as usize;
        let available = |slot: &Slot| {
            slot.active
                && !slot.failed
                && slot.connection == tag
                && slot.birth.is_none()
                && slot.phase_ledger.is_none()
        };
        if !Arc::ptr_eq(self, &store.authority_writer_boundary)
            || c.transaction_state(Some(rusqlite::DatabaseName::Main))
                .map_err(|_| Error::storage())?
                != rusqlite::TransactionState::Write
            || !available(&*self.slot.lock().map_err(|_| Error::storage())?)
        {
            return Err(Error::forbidden());
        }
        let facts = decision_facts(store, c, captured, Some(proof))?
            .0
            .ok_or_else(|| {
                Error::new(
                    "authority-busy",
                    "Commit capture now requires original writer custody",
                )
            })?;
        if !store.authority_covers(proof, &facts.required) {
            return Err(Error::forbidden());
        }
        let mut slot = self.slot.lock().map_err(|_| Error::storage())?;
        if !available(&slot) || c.is_autocommit() || crate::now() >= facts.capture.until {
            return Err(Error::forbidden());
        }
        let nonce = slot
            .next_birth_nonce
            .checked_add(1)
            .ok_or_else(Error::storage)?;
        slot.next_birth_nonce = nonce;
        slot.phase_ledger = Some(Active {
            key: captured.commit_ledger_key(),
            nonce,
            root_revision: facts.revision,
            reservation_root: None,
            candidate_audience: None,
            commit_capture: Some(facts.capture),
            scoped_capture: None,
            _proof: proof.clone(),
        });
        Ok(Permit {
            state: self.clone(),
            nonce,
            _connection: std::marker::PhantomData,
        })
    }
}
impl Permit<'_> {
    pub(crate) fn commit_capture_recorded_at(&self) -> Result<i64> {
        let slot = self.state.slot.lock().map_err(|_| Error::storage())?;
        slot.phase_ledger
            .as_ref()
            .filter(|active| slot.active && !slot.failed && active.nonce == self.nonce)
            .and_then(|active| active.commit_capture.as_ref())
            .map(|capture| capture.recorded_at)
            .ok_or_else(Error::forbidden)
    }
    pub(crate) fn require_commit_capture_current(&self) -> Result<()> {
        let slot = self.state.slot.lock().map_err(|_| Error::storage())?;
        if slot.active
            && !slot.failed
            && slot.phase_ledger.as_ref().is_some_and(|active| {
                active.nonce == self.nonce
                    && active
                        .commit_capture
                        .as_ref()
                        .is_some_and(|capture| crate::now() < capture.until)
            })
        {
            Ok(())
        } else {
            Err(Error::forbidden())
        }
    }
}
