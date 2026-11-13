// Pure test rendezvous inside the genuine authorized snapshot transaction.
// No callback or extra work exists in ordinary builds.
thread_local! {
    static PROBE: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = Default::default();
}
pub(crate) fn checkpoint() {
    let probe = PROBE.with(|slot| slot.borrow_mut().take());
    if let Some(probe) = probe {
        probe();
    }
}
pub(crate) fn with_checkpoint<T>(probe: impl FnOnce() + 'static, body: impl FnOnce() -> T) -> T {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            PROBE.with(|slot| {
                slot.borrow_mut().take();
            });
        }
    }
    PROBE.with(|slot| {
        let mut slot = slot.borrow_mut();
        assert!(slot.is_none());
        *slot = Some(Box::new(probe));
    });
    let _reset = Reset;
    body()
}
