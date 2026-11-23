//! Invocation-local observations of the real prepared Native commit boundary.
//! Compiled only for tests/test-support; retains no authority or media owner.
use std::cell::RefCell;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

/// Exact counters from one original preparation, copied after final aliases drop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativePreparedBeginSnapshotForTest {
    /// Actual prepared Store entries, including combined scene/begin admission.
    pub calls: usize,
    /// Actual try_connection authority-busy branches, before SQLite BEGIN.
    pub pre_entry_busy: usize,
    /// Successful real SQLite BEGINs.
    pub sql_begins: usize,
    /// Actual operation_effects INSERT attempts after all original checks.
    pub before_dml: usize,
    /// Errors from that exact INSERT.
    pub insert_errors: usize,
    /// Those INSERT errors matching SQLite 1811 AND the controlled trigger text.
    pub controlled_trigger_errors: usize,
    /// Actual typed Ok(Some(_)) returns.
    pub some: usize,
    /// Actual typed Ok(None) returns.
    pub none: usize,
    /// Scoped typed Err(_) returns; setup refusals are separately counted.
    pub errors: usize,
    /// Real raw connection mutex exclusions acquired by this fixture.
    pub exclusion_holds: usize,
    /// Actual Store-entry or retained-uncertainty tuple mismatches.
    pub binding_mismatches: usize,
    /// Actual same-attempt unconfirmed custody observations.
    pub unconfirmed: usize,
    /// Holder setup cutoff/poison refusals before the original method is invoked.
    pub scope_refusals: usize,
    /// Combined calls that reached the actual durable submission marker.
    pub issued: usize,
    /// Combined-call errors before that marker; no BEGIN or DML was entered.
    pub pre_entry_refusals: usize,
}
/// Numeric observation only, plus immutable original IDs. Never a grant.
pub struct NativePreparedBeginObservationForTest {
    binding: (String, String, String),
    counts: [AtomicUsize; 15],
    exclude_once: AtomicBool,
}
struct Active {
    observation: Arc<NativePreparedBeginObservationForTest>,
    admission: bool,
}
thread_local! {
    static ACTIVE: RefCell<Option<Active>> = const { RefCell::new(None) };
}
impl NativePreparedBeginObservationForTest {
    /// Create before final admission using the actual cold prepared tuple.
    pub fn new(binding: (&str, &str, &str)) -> Self {
        Self {
            binding: (binding.0.into(), binding.1.into(), binding.2.into()),
            counts: std::array::from_fn(|_| AtomicUsize::new(0)),
            exclude_once: AtomicBool::new(false),
        }
    }
    /// Request one real mutex exclusion; no result is supplied or rewritten.
    pub fn exclude_next_entry(&self) {
        self.exclude_once.store(true, Ordering::Release);
    }
    /// Exact original IDs, metadata only.
    pub fn binding(&self) -> (&str, &str, &str) {
        (&self.binding.0, &self.binding.1, &self.binding.2)
    }
    fn increment(&self, index: usize) {
        self.counts[index].fetch_add(1, Ordering::AcqRel);
    }
    fn check_binding(&self, binding: (&str, &str, &str)) {
        if binding != self.binding() {
            self.increment(10);
        }
    }
    /// Observe the actual tuple installed in retained uncertain custody.
    pub fn note_unconfirmed(&self, binding: (&str, &str, &str)) {
        self.check_binding(binding);
        self.increment(11);
    }
    /// Copy counters only; Acquire pairs with observations before uncertainty.
    pub fn snapshot(&self) -> NativePreparedBeginSnapshotForTest {
        let c = self.counts.each_ref().map(|n| n.load(Ordering::Acquire));
        NativePreparedBeginSnapshotForTest {
            calls: c[0],
            pre_entry_busy: c[1],
            sql_begins: c[2],
            before_dml: c[3],
            insert_errors: c[4],
            controlled_trigger_errors: c[5],
            some: c[6],
            none: c[7],
            errors: c[8],
            exclusion_holds: c[9],
            binding_mismatches: c[10],
            unconfirmed: c[11],
            scope_refusals: c[12],
            issued: c[13],
            pre_entry_refusals: c[14],
        }
    }
}
fn with_active(call: impl FnOnce(&NativePreparedBeginObservationForTest)) {
    ACTIVE.with(|slot| {
        if let Some(value) = slot.borrow().as_ref() {
            call(&value.observation);
        }
    });
}
pub(crate) fn entry(binding: (&str, &str, &str)) {
    with_active(|o| {
        o.check_binding(binding);
        o.increment(0);
    });
}
pub(crate) fn pre_entry_busy() {
    with_active(|o| o.increment(1));
}
pub(crate) fn issued() {
    with_active(|o| o.increment(13));
}
pub(crate) fn sql_begin() {
    with_active(|o| o.increment(2));
}
pub(crate) fn before_dml() {
    with_active(|o| o.increment(3));
}
pub(crate) fn insert_error(error: &rusqlite::Error) {
    with_active(|o| {
        o.increment(4);
        if matches!(error, rusqlite::Error::SqliteFailure(code, Some(message))
            if code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_TRIGGER
            && message == "controlled real post-BEGIN final begin failure")
        {
            o.increment(5);
        }
    });
}
impl crate::Store {
    /// Scope observation to THIS synchronous Store call on the blocking thread.
    /// The optional holder uses the actual raw mutex and is dropped before the
    /// result reaches the async retry loop. This legacy begin-only wrapper
    /// keeps its original entry exclusion. Neither panic nor return leaves TLS.
    pub fn with_native_prepared_begin_observation_for_test<T>(
        &self,
        observation: Arc<NativePreparedBeginObservationForTest>,
        wait: &crate::AuthorityWait,
        call: impl FnOnce() -> crate::Result<Option<T>>,
    ) -> crate::Result<Option<T>> {
        self.native_prepared_observation_scope(observation, wait, false, call)
    }
    /// Observe combined admission. The real optional mutex exclusion is taken
    /// after provisional readers drop, before SQL reservation/WriterNotice.
    pub fn with_native_prepared_admission_observation_for_test<T>(
        &self,
        observation: Arc<NativePreparedBeginObservationForTest>,
        wait: &crate::AuthorityWait,
        call: impl FnOnce() -> crate::Result<Option<T>>,
    ) -> crate::Result<Option<T>> {
        self.native_prepared_observation_scope(observation, wait, true, call)
    }
    fn native_prepared_observation_scope<T>(
        &self,
        observation: Arc<NativePreparedBeginObservationForTest>,
        wait: &crate::AuthorityWait,
        admission: bool,
        call: impl FnOnce() -> crate::Result<Option<T>>,
    ) -> crate::Result<Option<T>> {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                ACTIVE.with(|slot| {
                    slot.borrow_mut().take();
                });
            }
        }
        ACTIVE.with(|slot| {
            let mut slot = slot.borrow_mut();
            assert!(slot.is_none(), "nested Native prepared begin observation");
            *slot = Some(Active {
                observation: observation.clone(),
                admission,
            });
        });
        let _reset = Reset;
        let _holder = if !admission {
            match self.native_prepared_exclusion_for_test(&observation, wait) {
                Ok(holder) => holder,
                Err(error) => {
                    observation.increment(8);
                    return Err(error);
                }
            }
        } else {
            None
        };
        let before_issued = observation.counts[13].load(Ordering::Acquire);
        let result = call();
        observation.increment(match &result {
            Ok(Some(_)) => 6,
            Ok(None) => 7,
            Err(_)
                if admission && observation.counts[13].load(Ordering::Acquire) == before_issued =>
            {
                14
            }
            Err(_) => 8,
        });
        result
    }
    pub(crate) fn native_prepared_admission_exclusion_for_test(
        &self,
        wait: &crate::AuthorityWait,
    ) -> crate::Result<Option<std::sync::MutexGuard<'_, rusqlite::Connection>>> {
        let observation = ACTIVE.with(|slot| {
            slot.borrow()
                .as_ref()
                .filter(|active| active.admission)
                .map(|active| active.observation.clone())
        });
        match observation {
            Some(observation) => self.native_prepared_exclusion_for_test(&observation, wait),
            None => Ok(None),
        }
    }
    fn native_prepared_exclusion_for_test(
        &self,
        observation: &NativePreparedBeginObservationForTest,
        wait: &crate::AuthorityWait,
    ) -> crate::Result<Option<std::sync::MutexGuard<'_, rusqlite::Connection>>> {
        let held = if observation.exclude_once.load(Ordering::Acquire) {
            loop {
                // The fixture must actually own the exclusion before invoking
                // the original method; another holder releasing cannot win a
                // gap between a failed try-lock and the real Store call.
                if let Err(error) = wait.check() {
                    observation.increment(12);
                    return Err(error);
                }
                match self.connection.try_lock() {
                    Ok(guard) => {
                        observation.exclude_once.store(false, Ordering::Release);
                        observation.increment(9);
                        break Some(guard);
                    }
                    Err(std::sync::TryLockError::WouldBlock) => std::thread::yield_now(),
                    Err(std::sync::TryLockError::Poisoned(_)) => {
                        observation.increment(12);
                        return Err(crate::Error::storage());
                    }
                }
            }
        } else {
            None
        };
        Ok(held)
    }
}
