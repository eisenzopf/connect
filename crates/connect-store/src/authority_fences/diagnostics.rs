//! Explicit bounded metadata diagnostics. Never own a file or authority grant.
use super::*;
use std::sync::atomic::AtomicUsize;

fn enabled() -> bool {
    tracing::enabled!(target: "connect_store::authority_summary", tracing::Level::DEBUG)
}
fn reserve(counter: &AtomicUsize) -> bool {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
            (n < 256).then_some(n + 1)
        })
        .is_ok()
}

#[derive(Clone, Copy)]
pub(super) enum ImmediateBusy {
    IdentityLookup,
    IdentityPublication,
    KernelAdmission,
    KernelResource,
    ReaderRegistry,
}
impl ImmediateBusy {
    fn name(self) -> &'static str {
        match self {
            Self::IdentityLookup => "file-identity-lookup",
            Self::IdentityPublication => "file-identity-publication",
            Self::KernelAdmission => "kernel-admission-lock",
            Self::KernelResource => "kernel-resource-lock",
            Self::ReaderRegistry => "reader-registry",
        }
    }
    fn sample(self) -> Option<usize> {
        static COUNTS: [AtomicUsize; 5] = [const { AtomicUsize::new(0) }; 5];
        // Powers of two preserve sparse visibility across a busy programme.
        // Each cause emits at most 25 records per process.
        COUNTS[self as usize]
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                (n < 1 << 24).then_some(n + 1)
            })
            .ok()
            .map(|n| n + 1)
            .filter(|n| n.is_power_of_two())
    }
}

// Declared before partial file guards, so failure logging runs after they drop.
pub(super) struct Admission {
    began: Option<Instant>,
    immediate: bool,
    immediate_busy: Option<ImmediateBusy>,
    pub stage: &'static str,
    pub kind: &'static str,
    pub write: bool,
    pub completed: usize,
    pub total: usize,
    pub passed: bool,
}
impl Admission {
    pub fn new(wait: Option<&AuthorityWait>) -> Self {
        Self {
            began: (wait.is_some_and(|w| w.projection_diagnostic) && enabled()).then(Instant::now),
            immediate: wait.is_none(),
            immediate_busy: None,
            stage: "initial",
            kind: "none",
            write: false,
            completed: 0,
            total: 0,
            passed: false,
        }
    }
    pub fn busy(&mut self, cause: ImmediateBusy, error: &Error) {
        if self.immediate && error.code == "authority-busy" {
            self.immediate_busy.get_or_insert(cause);
        }
    }
    // Always captured at the actual failure leaf, even when tracing/sampling
    // is disabled. Only this typed cause distinguishes exclusion intent from
    // unavailable local metadata; public acquisition errors stay unchanged.
    pub(super) fn immediate_cause(&self) -> Option<ImmediateBusy> {
        self.immediate_busy
    }
    pub fn scope(&mut self, kind: Kind, mode: AuthorityMode) {
        self.kind = match kind {
            Kind::Identity => "identity",
            Kind::Conversation => "conversation",
            Kind::Record => "record",
            Kind::Connection => "connection",
        };
        self.write = mode == AuthorityMode::Write;
        self.stage = "scope-deadline";
    }
}
impl Drop for Admission {
    fn drop(&mut self) {
        if let Some(cause) = self.immediate_busy {
            if enabled() {
                if let Some(occurrence) = cause.sample() {
                    tracing::debug!(target: "connect_store::authority_summary",
                        cause=cause.name(), occurrence, stage=self.stage, kind=self.kind, write=self.write,
                        completed_scopes=self.completed, total_scopes=self.total,
                        "Immediate authority admission deferred");
                }
            }
            return;
        }
        static RECORDS: AtomicUsize = AtomicUsize::new(0);
        let Some(began) = self.began else { return };
        if self.passed || !reserve(&RECORDS) {
            return;
        }
        tracing::debug!(target: "connect_store::authority_summary",
            stage=self.stage, kind=self.kind, write=self.write,
            completed_scopes=self.completed, total_scopes=self.total,
            elapsed_us=began.elapsed().as_micros() as u64,
            "Replacement projection authority acquisition refused");
    }
}

pub(super) fn enabled_hold() -> bool {
    enabled()
}
pub(super) fn released(started: Instant, caller: &'static std::panic::Location<'static>) {
    static RECORDS: AtomicUsize = AtomicUsize::new(0);
    if !enabled() || started.elapsed() < Duration::from_millis(20) || !reserve(&RECORDS) {
        return;
    }
    tracing::debug!(target: "connect_store::authority_summary",
        elapsed_us=started.elapsed().as_micros() as u64,
        file=caller.file(), line=caller.line(),
        "Authority interval exceeding one audio quantum released");
}
