//! Unit-only invocation-local Ready/closed-permit phase timing. No SQL, IO,
//! mutex, authority, retry, payload or shipping code. Formatting is caller-only.
use std::{cell::RefCell, time::Instant};
#[derive(Clone, Debug)]
pub(crate) struct Phase {
    pub label: &'static str,
    pub delta_us: u64,
    pub elapsed_us: u64,
}
struct Capture {
    began: Instant,
    last: Instant,
    phases: Vec<Phase>,
    overflow: u64,
}
thread_local! { static ACTIVE: RefCell<Option<Capture>> = const { RefCell::new(None) }; }
pub(crate) fn mark(label: &'static str) {
    ACTIVE.with(|active| {
        let mut active = active.borrow_mut();
        let Some(active) = active.as_mut() else {
            return;
        };
        let now = Instant::now();
        if active.phases.len() < 64 {
            active.phases.push(Phase {
                label,
                delta_us: now
                    .duration_since(active.last)
                    .as_micros()
                    .try_into()
                    .unwrap_or(u64::MAX),
                elapsed_us: now
                    .duration_since(active.began)
                    .as_micros()
                    .try_into()
                    .unwrap_or(u64::MAX),
            });
        } else {
            active.overflow = active.overflow.saturating_add(1);
        }
        active.last = now;
    });
}
struct Reset;
impl Drop for Reset {
    fn drop(&mut self) {
        ACTIVE.with(|a| {
            a.borrow_mut().take();
        });
    }
}
pub(crate) fn collect<T>(call: impl FnOnce() -> T) -> (T, (Vec<Phase>, u64)) {
    ACTIVE.with(|a| {
        let mut a = a.borrow_mut();
        assert!(a.is_none(), "phase timing cannot nest");
        let now = Instant::now();
        *a = Some(Capture {
            began: now,
            last: now,
            phases: Vec::with_capacity(64),
            overflow: 0,
        });
    });
    let reset = Reset;
    let result = call();
    mark("call-returned");
    let result_phases = ACTIVE.with(|a| {
        let c = a.borrow_mut().take().unwrap();
        (c.phases, c.overflow)
    });
    drop(reset);
    (result, result_phases)
}
