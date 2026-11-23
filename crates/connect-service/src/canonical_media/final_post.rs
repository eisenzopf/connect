//! Original admitted operation owns terminal cleanup even on cancellation/unwind.
//! No Reader, Writer, role, SIP fence or packet/control authority is stored here.
use crate::canonical_runtime::Runtime;
use rvoip_core::orchestrator::participant_control::{
    ManagedCutPostCompletion, ManagedCutPostFinishError, ManagedCutPostPublication,
};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc, Mutex,
    },
};

pub(crate) struct FailedPost {
    record: String,
    attempt: String,
    effect: String,
    post: ManagedCutPostPublication,
}
pub(crate) enum ObservedCut {
    Replacement {
        outcome: rvoip_core::staged_streams::device_replacement::AudioDeviceReplacementOutcome,
        handle: rvoip_core::staged_streams::device_replacement::AudioDeviceReplacementHandle,
    },
    Handoff {
        outcome: rvoip_core::staged_streams::handoff::AudioHandoffOutcome,
        handle: rvoip_core::staged_streams::handoff::AudioHandoffHandle,
    },
}
pub(crate) struct UnsettledObservation {
    binding: (String, String, String),
    actual: ObservedCut,
}
pub(crate) struct UnconfirmedBegin {
    record: String,
    attempt: String,
    effect: String,
}
impl UnconfirmedBegin {
    pub(crate) fn report_and_release(self) {
        tracing::error!(target:"connect_service::canonical_runtime_drain",record=%self.record,attempt=%self.attempt,effect=%self.effect,"Original Store begin returned uncertain; same IDs retained and never resubmitted");
    }
}
#[derive(Default)]
pub(crate) struct OperationEntry {
    final_owners: std::sync::atomic::AtomicUsize,
    job_finished: std::sync::atomic::AtomicBool,
    unconfirmed_begin: Mutex<Option<UnconfirmedBegin>>,
    final_cutoff: std::sync::OnceLock<tokio::time::Instant>,
    failed: Mutex<Vec<FailedPost>>,
    observed: Mutex<Option<UnsettledObservation>>,
}
pub(crate) struct ObservationCustody {
    runtime: std::sync::Weak<Runtime>,
    entry: Arc<OperationEntry>,
    binding: (String, String, String),
}
impl ObservationCustody {
    /// Only call AFTER exact Known/Applied custody is installed by the caller.
    pub(crate) fn recorded(self) {
        let observed = {
            let mut slot = self
                .entry
                .observed
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if slot.as_ref().is_some_and(|o| o.binding == self.binding) {
                slot.take()
            } else {
                None
            }
        };
        drop(observed);
        if let Some(runtime) = self.runtime.upgrade() {
            runtime
                .consultation_jobs
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .prune(&self.binding.0);
        }
    }
}
impl UnsettledObservation {
    pub(crate) fn report_and_release(self) {
        tracing::error!(target:"connect_service::canonical_runtime_drain",record=%self.binding.0,
            attempt=%self.binding.1,effect=%self.binding.2,"Actual managed publication retained without completed durable capture");
        drop(self.actual);
    }
}
/// SAME admitted-job container/capacity, with an owned terminal payload per entry.
#[derive(Default)]
pub(crate) struct OperationJobs {
    entries: BTreeMap<String, Arc<OperationEntry>>,
}
impl OperationJobs {
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
    pub(crate) fn insert(&mut self, record: String) -> bool {
        if self.entries.contains_key(&record) {
            return false;
        }
        self.entries
            .insert(record, Arc::new(OperationEntry::default()));
        true
    }
    pub(crate) fn remove(&mut self, record: &str) {
        if let Some(entry) = self.entries.get(record) {
            entry.job_finished.store(true, Ordering::Release);
        }
        self.prune(record);
    }
    fn prune(&mut self, record: &str) {
        let removable = self.entries.get(record).is_some_and(|entry| {
            entry.job_finished.load(Ordering::Acquire)
                && entry.final_owners.load(Ordering::Acquire) == 0
                && entry
                    .failed
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .is_empty()
                && entry
                    .observed
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .is_none()
                && entry
                    .unconfirmed_begin
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .is_none()
        });
        if removable {
            self.entries.remove(record);
        }
    }
    pub(crate) fn take_unconfirmed_begin(&mut self) -> Vec<UnconfirmedBegin> {
        let mut outcomes = Vec::new();
        for entry in self.entries.values() {
            if let Some(outcome) = entry
                .unconfirmed_begin
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take()
            {
                outcomes.push(outcome);
            }
        }
        outcomes
    }
    pub(crate) fn get(&self, record: &str) -> Option<Arc<OperationEntry>> {
        self.entries.get(record).cloned()
    }
    pub(crate) fn take_observed(&mut self) -> Vec<UnsettledObservation> {
        let mut observed = Vec::new();
        for entry in self.entries.values() {
            if let Some(value) = entry
                .observed
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take()
            {
                observed.push(value);
            }
        }
        observed
    }
    pub(crate) fn take_failed(&mut self) -> Vec<FailedPost> {
        let mut failed = Vec::new();
        for entry in self.entries.values() {
            failed.extend(std::mem::take(
                &mut *entry.failed.lock().unwrap_or_else(|p| p.into_inner()),
            ));
        }
        failed
    }
}
impl FailedPost {
    pub(crate) fn report_and_release(self) {
        // Caller already moved ownership OUT of every operation container.
        tracing::error!(target:"connect_service::canonical_runtime_drain",record=%self.record,
            attempt=%self.attempt,effect=%self.effect,"Original managed cleanup submission failed; published outcome remains uncertain");
        drop(self.post);
    }
}
const BEGIN_IDLE: u8 = 0;
const BEGIN_ISSUED: u8 = 1;
const BEGIN_CLOSED: u8 = 2;

/// One dispatch marker shared with the blocking Store call. If the outer
/// owner has closed, a late worker cannot begin a durable effect.
pub(crate) struct BeginDispatch {
    state: Arc<AtomicU8>,
    #[cfg(feature = "test-support")]
    probe: Option<Arc<super::preparation_custody_fixture::Probe>>,
}
impl BeginDispatch {
    pub(crate) fn issue(&self) -> connect_protocol::Result<()> {
        self.state
            .compare_exchange(
                BEGIN_IDLE,
                BEGIN_ISSUED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|_| connect_protocol::Error::forbidden())?;
        #[cfg(feature = "test-support")]
        if let Some(probe) = &self.probe {
            probe.note_begin_submission();
        }
        Ok(())
    }
}

/// Declare BEFORE the inner final frame. That frame (including actual SIP
/// guards and ALL read/Writer/Invocation/role aliases) must end before this owner.
pub(crate) struct PostCustody<'a> {
    runtime: &'a Arc<Runtime>,
    entry: Arc<OperationEntry>,
    binding: (String, String, String),
    post: Option<ManagedCutPostPublication>,
    cleanup_observer_until: Option<tokio::time::Instant>,
    observation: Option<UnsettledObservation>,
    begin_state: Arc<AtomicU8>,
    #[cfg(feature = "test-support")]
    probe: Option<Arc<super::preparation_custody_fixture::Probe>>,
}
impl<'a> PostCustody<'a> {
    pub(crate) fn new(
        runtime: &'a Arc<Runtime>,
        binding: (&str, &str, &str),
    ) -> connect_protocol::Result<Self> {
        let entry = {
            let jobs = runtime
                .consultation_jobs
                .lock()
                .map_err(|_| connect_protocol::Error::storage())?;
            let entry = jobs
                .get(binding.0)
                .ok_or_else(connect_protocol::Error::forbidden)?;
            // Pin while SAME admitted-map lock still excludes watcher removal.
            entry.final_owners.fetch_add(1, Ordering::AcqRel);
            entry
        };
        Ok(Self {
            runtime,
            entry,
            binding: (binding.0.into(), binding.1.into(), binding.2.into()),
            post: None,
            cleanup_observer_until: None,
            observation: None,
            begin_state: Arc::new(AtomicU8::new(BEGIN_IDLE)),
            #[cfg(feature = "test-support")]
            probe: None,
        })
    }
    pub(crate) fn begin_dispatch(&self) -> BeginDispatch {
        BeginDispatch {
            state: self.begin_state.clone(),
            #[cfg(feature = "test-support")]
            probe: self.probe.clone(),
        }
    }
    pub(crate) fn begin_issued(&mut self) {
        self.begin_dispatch()
            .issue()
            .expect("one original begin dispatch");
    }
    /// Only typed Some custody or definite pre-DML None clears dispatch state.
    pub(crate) fn begin_returned_definite(&mut self) {
        self.begin_state.store(BEGIN_IDLE, Ordering::Release);
    }
    pub(crate) fn begin_is_unconfirmed(&self) -> bool {
        self.begin_state.load(Ordering::Acquire) == BEGIN_ISSUED
    }
    #[cfg(feature = "test-support")]
    pub(crate) fn observe_for_test(
        &mut self,
        probe: Option<Arc<super::preparation_custody_fixture::Probe>>,
    ) {
        self.probe = probe;
    }
    pub(crate) fn observe(&mut self, actual: ObservedCut) -> ObservationCustody {
        // Only local stack ownership here: final SIP/Writer guards may still
        // be held. The admitted-entry mutex is touched AFTER that frame ends.
        assert!(
            self.observation.is_none(),
            "linear managed token published more than once"
        );
        self.observation = Some(UnsettledObservation {
            binding: self.binding.clone(),
            actual,
        });
        ObservationCustody {
            runtime: Arc::downgrade(self.runtime),
            entry: self.entry.clone(),
            binding: self.binding.clone(),
        }
    }
    fn flush_observation(&mut self) {
        let Some(observed) = self.observation.take() else {
            return;
        };
        let mut slot = self
            .entry
            .observed
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        assert!(
            slot.is_none(),
            "same admitted operation already retains a published observation"
        );
        *slot = Some(observed);
    }
    pub(crate) fn install(&mut self, post: ManagedCutPostPublication) {
        // Exactly one terminal return exists for this linear prepared token.
        assert!(
            self.post.is_none(),
            "managed cut returned more than one terminal post"
        );
        self.cleanup_observer_until =
            Some(tokio::time::Instant::now() + std::time::Duration::from_secs(3));
        self.post = Some(post);
    }
    fn submit(&mut self) -> connect_protocol::Result<Option<ManagedCutPostCompletion>> {
        // Caller or unwind has released every inner alias before this point.
        self.flush_observation();
        if self.begin_state.swap(BEGIN_CLOSED, Ordering::AcqRel) == BEGIN_ISSUED {
            *self
                .entry
                .unconfirmed_begin
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = Some(UnconfirmedBegin {
                record: self.binding.0.clone(),
                attempt: self.binding.1.clone(),
                effect: self.binding.2.clone(),
            });
            self.runtime
                .retirement_failed
                .store(true, Ordering::Release);
            #[cfg(feature = "test-support")]
            if let Some(probe) = &self.probe {
                probe.note_begin_unconfirmed((&self.binding.0, &self.binding.1, &self.binding.2));
            }
        }
        let Some(post) = self.post.take() else {
            return Ok(None);
        };
        #[cfg(feature = "test-support")]
        if let Some(probe) = &self.probe {
            probe.post_start();
        }
        match self.runtime.core.start_managed_cut_publication(post) {
            Ok(completion) => {
                #[cfg(feature = "test-support")]
                if let Some(probe) = &self.probe {
                    probe.post_accepted();
                }
                Ok(Some(completion))
            }
            Err(ManagedCutPostFinishError::SubmissionFailed { error, post }) => {
                self.runtime
                    .retirement_failed
                    .store(true, Ordering::Release);
                self.runtime.stop.cancel();
                self.entry
                    .failed
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push(FailedPost {
                        record: self.binding.0.clone(),
                        attempt: self.binding.1.clone(),
                        effect: self.binding.2.clone(),
                        post,
                    });
                tracing::error!(target:"connect_service::canonical_runtime",?error,record=%self.binding.0,
                    "Original managed cleanup worker refused; exact payload retained by admitted operation");
                Err(connect_protocol::Error::new(
                    "reconciliation-required",
                    "Published managed cleanup remains unconfirmed",
                ))
            }
            Err(ManagedCutPostFinishError::AcceptedUnconfirmed { error }) => {
                // No returned post and no admission retry after accepted transfer.
                self.runtime
                    .retirement_failed
                    .store(true, Ordering::Release);
                self.runtime.stop.cancel();
                tracing::error!(target:"connect_service::canonical_runtime",?error,"Managed cleanup already accepted but remains unconfirmed");
                Err(connect_protocol::Error::new(
                    "reconciliation-required",
                    "Accepted managed cleanup remains unconfirmed",
                ))
            }
        }
    }
    /// Synchronous send only. Caller must drop the outer transient resource
    /// bank/SIP owners BEFORE awaiting the returned observation.
    pub(crate) fn release(mut self) -> PostObservation {
        let completion = self.submit();
        PostObservation {
            runtime: self.runtime.clone(),
            completion,
            until: self.cleanup_observer_until,
            #[cfg(feature = "test-support")]
            probe: self.probe.clone(),
        }
    }
}
impl Drop for PostCustody<'_> {
    fn drop(&mut self) {
        // Synchronous send to PRE-ADMITTED terminal owner only. No task spawn,
        // admission, timer, await, new effect or Core retry is performed here.
        let _ = self.submit();
        let previous = self.entry.final_owners.fetch_sub(1, Ordering::AcqRel);
        assert!(previous > 0, "original final owner pin underflow");
        if previous == 1 {
            self.runtime
                .consultation_jobs
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .prune(&self.binding.0);
        }
    }
}

pub(crate) struct PostObservation {
    runtime: Arc<Runtime>,
    completion: connect_protocol::Result<Option<ManagedCutPostCompletion>>,
    until: Option<tokio::time::Instant>,
    #[cfg(feature = "test-support")]
    probe: Option<Arc<super::preparation_custody_fixture::Probe>>,
}
impl PostObservation {
    pub(crate) async fn wait(self) -> connect_protocol::Result<()> {
        let Some(completion) = self.completion? else {
            return Ok(());
        };
        // Existing cleanup/drain observer interval: existing Service
        // handoff cleanup and Runtime drain use 3s. This starts ONCE at the terminal return and
        // never changes the original publication cutoff or accepted work.
        let until = self.until.expect("installed terminal post");
        #[cfg(feature = "test-support")]
        if let Some(probe) = &self.probe {
            probe.post_observer_wait();
        }
        let observed = tokio::select! {biased;
            _=self.runtime.stop.cancelled()=>None,
            _=tokio::time::sleep_until(until)=>None,
            result=completion.wait()=>Some(result),
        };
        if !matches!(observed, Some(Ok(()))) {
            let error = observed.and_then(|result| result.err());
            self.runtime
                .retirement_failed
                .store(true, Ordering::Release);
            self.runtime.stop.cancel();
            tracing::error!(target:"connect_service::canonical_runtime",?error,"Accepted original managed cleanup remains unconfirmed");
            return Err(connect_protocol::Error::new(
                "reconciliation-required",
                "Accepted managed cleanup remains unconfirmed",
            ));
        }
        Ok(())
    }
}

impl Runtime {
    /// Same admitted operation's once-chosen final phase interval. A planning
    /// retry cannot renew it after the immutable plan/IDs were selected.
    pub(crate) fn managed_publication_cutoff(
        &self,
        record: &str,
        proposed: tokio::time::Instant,
    ) -> connect_protocol::Result<tokio::time::Instant> {
        let entry = self
            .consultation_jobs
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?
            .get(record)
            .ok_or_else(connect_protocol::Error::forbidden)?;
        Ok((*entry.final_cutoff.get_or_init(|| proposed)).min(proposed))
    }
}

#[cfg(test)]
mod tests;
