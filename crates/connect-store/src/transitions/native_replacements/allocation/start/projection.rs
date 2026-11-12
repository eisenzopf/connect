//! Brief admission for captured facts; no authority writer parked behind SQL.
use super::*;
use crate::installation_ownership::ConnectionGuard;

/// Fixed-size counters owned by one immutable physical capture. These contain
/// no credentials, packet data, Store reader or IO permission.
#[derive(Default)]
pub(super) struct AdmissionCounters {
    attempts: std::sync::atomic::AtomicU64,
    admitted: std::sync::atomic::AtomicU64,
    incomplete: std::sync::atomic::AtomicU64,
    elapsed_us: [std::sync::atomic::AtomicU64; 4],
    failed_stage: [std::sync::atomic::AtomicU64; 5],
    refusals: [std::sync::atomic::AtomicU64; 3],
    transaction_elapsed_us: [std::sync::atomic::AtomicU64; 3],
    transaction_outcomes: [std::sync::atomic::AtomicU64; 5],
}
fn add(counter: &std::sync::atomic::AtomicU64, value: u64) {
    use std::sync::atomic::Ordering::Relaxed;
    let _ = counter.fetch_update(Relaxed, Relaxed, |old| Some(old.saturating_add(value)));
}
impl AdmissionCounters {
    pub(super) fn snapshot(&self) -> serde_json::Value {
        use std::sync::atomic::Ordering::Relaxed;
        let times = self.elapsed_us.each_ref().map(|v| v.load(Relaxed));
        let failures = self.failed_stage.each_ref().map(|v| v.load(Relaxed));
        let refusals = self.refusals.each_ref().map(|v| v.load(Relaxed));
        let sql_times = self
            .transaction_elapsed_us
            .each_ref()
            .map(|v| v.load(Relaxed));
        let sql_outcomes = self
            .transaction_outcomes
            .each_ref()
            .map(|v| v.load(Relaxed));
        serde_json::json!({
            "attempts":self.attempts.load(Relaxed), "admitted":self.admitted.load(Relaxed),
            "incomplete":self.incomplete.load(Relaxed),
            "derive_before_us":times[0], "authority_wait_us":times[1],
            "sql_reacquire_us":times[2], "derive_after_us":times[3],
            "failed_derive_before":failures[0], "failed_authority_wait":failures[1],
            "failed_sql_reacquire":failures[2], "failed_derive_after":failures[3],
            "failed_scope_revalidation":failures[4],
            "authority_busy":refusals[0], "storage_unavailable":refusals[1],
            "permanent_refusal":refusals[2],
            "transaction": {
                "begin_us":sql_times[0], "body_us":sql_times[1], "commit_us":sql_times[2],
                "begin_failed":sql_outcomes[0], "body_aborted":sql_outcomes[1],
                "commit_failed":sql_outcomes[2], "committed":sql_outcomes[3],
                "unchanged":sql_outcomes[4]
            }
        })
    }
}

/// One admitted metadata transaction. The body includes validation, local
/// observations and serialization, not only SQLite execution. Failed begin or
/// commit includes the driver's cleanup; an aborted body includes rollback.
/// No SQL, logging, allocation or authority is introduced by this observer.
pub(super) struct TransactionTiming<'a> {
    counters: Option<&'a AdmissionCounters>,
    began: Option<std::time::Instant>,
    elapsed: [u64; 3],
    // 0/1/2 are the active begin/body/commit phases. 3/4 are terminal
    // committed/unchanged outcomes. Drop in an active phase means failure.
    stage: usize,
}
impl<'a> TransactionTiming<'a> {
    pub(super) fn new(counters: Option<&'a AdmissionCounters>) -> Self {
        Self {
            counters,
            began: counters.map(|_| std::time::Instant::now()),
            elapsed: [0; 3],
            stage: 0,
        }
    }
    fn advance(&mut self, stage: usize) {
        if let Some(began) = self.began.take() {
            let now = std::time::Instant::now();
            self.elapsed[self.stage] = now
                .duration_since(began)
                .as_micros()
                .try_into()
                .unwrap_or(u64::MAX);
            if stage < 3 {
                self.began = Some(now);
            }
        }
        self.stage = stage;
    }
    pub(super) fn body(&mut self) {
        self.advance(1);
    }
    pub(super) fn committing(&mut self) {
        self.advance(2);
    }
    pub(super) fn committed(&mut self) {
        self.advance(3);
    }
    pub(super) fn unchanged(&mut self) {
        self.advance(4);
    }
}
impl Drop for TransactionTiming<'_> {
    fn drop(&mut self) {
        let Some(counters) = self.counters else {
            return;
        };
        if let Some(began) = self.began.take() {
            self.elapsed[self.stage] = began.elapsed().as_micros().try_into().unwrap_or(u64::MAX);
        }
        for (counter, elapsed) in counters.transaction_elapsed_us.iter().zip(self.elapsed) {
            add(counter, elapsed);
        }
        add(&counters.transaction_outcomes[self.stage], 1);
    }
}
struct AdmissionTiming<'a> {
    counters: Option<&'a AdmissionCounters>,
    elapsed: [u64; 4],
    failed_stage: Option<usize>,
    refusal: usize,
    admitted: bool,
}
impl AdmissionTiming<'_> {
    fn step<T>(&mut self, index: usize, call: impl FnOnce() -> Result<T>) -> Result<T> {
        if self.counters.is_none() {
            return call();
        }
        let began = std::time::Instant::now();
        let result = call();
        self.elapsed[index] = began.elapsed().as_micros().try_into().unwrap_or(u64::MAX);
        if let Err(error) = &result {
            self.failed_stage = Some(index);
            self.refusal = match error.code.as_str() {
                "authority-busy" => 0,
                "storage-unavailable" => 1,
                _ => 2,
            };
        }
        result
    }
}
impl Drop for AdmissionTiming<'_> {
    fn drop(&mut self) {
        let Some(counters) = self.counters else {
            return;
        };
        // Only atomic accounting under authority/SQL custody. Serialization and
        // trace output happen in Service after the blocking Store call returns.
        for (counter, elapsed) in counters.elapsed_us.iter().zip(self.elapsed) {
            add(counter, elapsed);
        }
        if self.admitted {
            add(&counters.admitted, 1);
        } else if let Some(stage) = self.failed_stage {
            add(&counters.failed_stage[stage], 1);
            add(&counters.refusals[self.refusal], 1);
        } else {
            add(&counters.incomplete, 1);
        }
        add(&counters.attempts, 1);
    }
}

fn replacement_capture_wait() -> crate::AuthorityWait {
    crate::authority_admission::passive_projection_wait()
}

impl Store {
    /// Conformance composition uses the exact production admission budget and
    /// cadence. This supplies no scope, reader, writer or authorization grant.
    #[cfg(feature = "test-support")]
    pub fn replacement_projection_wait_for_test() -> crate::AuthorityWait {
        replacement_capture_wait()
    }
    /// No native IO belongs here. Derive the existing complete footprint,
    /// release SQL before a short fair authority drain, then rederive under
    /// the same zero-busy-timeout SQL guard returned to the transaction.
    /// The five-millisecond acquisition budget cannot become DEFAULT_WAIT's
    /// two-second writer barrier, or repeatedly miss a short packet reader.
    /// Refusal retains neither partial authority nor a SQL mutex. The owner
    /// retries only its immutable captured metadata, never the physical effect.
    pub(super) fn try_replacement_capture_projection<'a>(
        &'a self,
        counters: Option<&AdmissionCounters>,
        derive: impl Fn(&Connection) -> Result<Vec<AuthorityScope>>,
    ) -> Result<(Arc<AuthorityGuardSet>, ConnectionGuard<'a>)> {
        let mut timing = AdmissionTiming {
            counters,
            elapsed: [0; 4],
            failed_stage: None,
            refusal: 0,
            admitted: false,
        };
        // Use the same five-millisecond admission deadline for initial SQL
        // exclusion and authority acquisition. No authority is held during this
        // short SQL wait. An immediate miss here otherwise forces the owner
        // through its twenty-millisecond retry cadence, even for a tiny clash.
        let wait = replacement_capture_wait();
        let scopes = timing.step(0, || {
            let c = self.try_connection_with_wait(&wait)?;
            derive(&c)
        })?;
        let held = timing.step(1, || self.acquire_authority(&scopes, &wait))?;
        #[cfg(test)]
        crate::authority_admission::observe_passive_admission(true);
        let c = timing.step(2, || self.try_connection())?;
        let current = timing.step(3, || derive(&c))?;
        if !self.authority_covers(&held, &current) {
            timing.failed_stage = Some(4);
            timing.refusal = 0;
            return Err(Error::new(
                "authority-busy",
                "Replacement capture projection scope changed",
            ));
        }
        c.bind_available_mutation_authority(self, &held)?;
        timing.admitted = true;
        Ok((held, c))
    }
    /// Final replacement publication (ready or stopped) keeps its SQL Unit through immediate
    /// full Writer admission. There is no primary-SQL reacquire gap after the
    /// WriterNotice. Kernel/file admission NEVER waits while SQL is owned;
    /// registry metadata mutexes and filesystem validation remain synchronous.
    /// A miss releases the entire Unit/proof before Service's same-capture retry.
    pub(super) fn try_replacement_publication_projection<'a>(
        &'a self,
        counters: Option<&AdmissionCounters>,
        derive: impl Fn(&Connection) -> Result<Vec<AuthorityScope>>,
    ) -> Result<(Arc<AuthorityGuardSet>, ConnectionGuard<'a>)> {
        let mut timing = AdmissionTiming {
            counters,
            elapsed: [0; 4],
            failed_stage: None,
            refusal: 0,
            admitted: false,
        };
        // Preserve the existing five-millisecond SQL admission bound. This
        // proof owns no file Writer during SQL admission/footprint derivation.
        let wait = replacement_capture_wait();
        let (c, scopes) = timing.step(0, || {
            let c = self.try_connection_with_wait(&wait)?;
            let scopes = derive(&c)?;
            Ok((c, scopes))
        })?;
        // None in Manager means immediate kernel/resource admission, NOT a
        // promise that internal metadata mutexes/filesystem calls cannot delay.
        let held = timing.step(1, || {
            wait.check()?;
            let held = self.try_acquire_authority(&scopes)?;
            wait.check()?;
            Ok(held)
        })?;
        #[cfg(test)]
        crate::authority_admission::observe_passive_admission(true);
        let current = timing.step(3, || derive(&c))?;
        if !self.authority_covers(&held, &current) {
            timing.failed_stage = Some(4);
            timing.refusal = 0;
            return Err(Error::new(
                "authority-busy",
                "Replacement publication projection scope changed",
            ));
        }
        c.bind_available_mutation_authority(self, &held)?;
        timing.admitted = true;
        Ok((held, c))
    }
}
