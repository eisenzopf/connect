//! Brief fair admission for captured facts; no long writer parking or SQL wait.
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
        serde_json::json!({
            "attempts":self.attempts.load(Relaxed), "admitted":self.admitted.load(Relaxed),
            "incomplete":self.incomplete.load(Relaxed),
            "derive_before_us":times[0], "authority_wait_us":times[1],
            "sql_reacquire_us":times[2], "derive_after_us":times[3],
            "failed_derive_before":failures[0], "failed_authority_wait":failures[1],
            "failed_sql_reacquire":failures[2], "failed_derive_after":failures[3],
            "failed_scope_revalidation":failures[4],
            "authority_busy":refusals[0], "storage_unavailable":refusals[1],
            "permanent_refusal":refusals[2]
        })
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

impl Store {
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
        let scopes = timing.step(0, || {
            let c = self.try_connection()?;
            derive(&c)
        })?;
        let wait = crate::AuthorityWait::new(std::time::Duration::from_millis(5))
            .poll_every(std::time::Duration::from_micros(100));
        let held = timing.step(1, || self.acquire_authority(&scopes, &wait))?;
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
        timing.admitted = true;
        Ok((held, c))
    }
}
