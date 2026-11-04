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

// Declared before partial file guards, so failure logging runs after they drop.
pub(super) struct Admission {
    began: Option<Instant>,
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
            began: (enabled() && wait.is_some_and(|w| w.projection_diagnostic)).then(Instant::now),
            stage: "initial",
            kind: "none",
            write: false,
            completed: 0,
            total: 0,
            passed: false,
        }
    }
    pub fn scope(&mut self, kind: Kind, mode: AuthorityMode) {
        self.kind = match kind {
            Kind::Identity => "identity",
            Kind::Conversation => "conversation",
            Kind::Record => "record",
        };
        self.write = mode == AuthorityMode::Write;
        self.stage = "scope-deadline";
    }
}
impl Drop for Admission {
    fn drop(&mut self) {
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
