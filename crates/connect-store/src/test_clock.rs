//! Scoped policy seconds for synchronous Store regressions only.
//! This does not replace wall/monotonic clocks, affect other test threads, or
//! exist in production builds. Each scoped worker must install its own clock.
use std::{cell::Cell, marker::PhantomData, rc::Rc};
thread_local! {
    static SECOND: Cell<Option<i64>> = const { Cell::new(None) };
}
pub(crate) fn current() -> Option<i64> {
    SECOND.with(Cell::get)
}
pub(crate) struct Clock {
    previous: Option<i64>,
    _thread: PhantomData<Rc<()>>,
}
impl Clock {
    pub(crate) fn at(second: i64) -> Self {
        Self {
            previous: SECOND.with(|value| value.replace(Some(second))),
            _thread: PhantomData,
        }
    }
    pub(crate) fn set(&self, second: i64) {
        SECOND.with(|value| value.set(Some(second)));
    }
}
impl Drop for Clock {
    fn drop(&mut self) {
        SECOND.with(|value| value.set(self.previous));
    }
}
