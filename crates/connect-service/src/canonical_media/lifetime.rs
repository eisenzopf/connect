//! Presence lifetime for existing custodians, independent of packet permission.
use super::*;
fn busy() -> connect_protocol::Error {
    connect_protocol::Error::new("authority-busy", "Original lifetime snapshot is refreshing")
}
// The selected Connection may be replaced by a same-physical credential
// successor while its old factory publishes a definitive old-epoch refusal.
// No result (including an early binding error) belongs to the installed owner
// unless the exact selected Arc is still installed at the observation boundary.
fn observe_selected_entry<T, R>(
    selected: &Arc<T>,
    observe: impl FnOnce() -> connect_protocol::Result<R>,
    installed: impl FnOnce() -> Option<Arc<T>>,
) -> connect_protocol::Result<R> {
    let result = observe();
    match installed() {
        Some(current) if Arc::ptr_eq(selected, &current) => result,
        _ => Err(busy()),
    }
}
impl Connection {
    pub(super) fn prepare_lifetime_snapshot(
        &self,
        store: &Arc<Store>,
    ) -> connect_protocol::Result<()> {
        let mut slot = self.lifetime.try_lock().map_err(|error| match error {
            std::sync::TryLockError::WouldBlock => busy(),
            std::sync::TryLockError::Poisoned(_) => connect_protocol::Error::forbidden(),
        })?;
        if slot.is_none() {
            *slot =
                Some(store.try_register_native_connection_lifetime_snapshot(self.origin.clone())?);
        }
        Ok(())
    }
    pub(crate) fn lifetime_binding(&self) -> ControlResult<ConnectionBinding> {
        self.binding
            .snapshot()
            .map_err(denied)?
            .as_ref()
            .clone()
            .ok_or(StreamControlError::NotReady)
    }
}
impl Policy {
    /// No SQL and no IO grant. Exact current physical binding is independently
    /// retained by the original custodian; a credential successor cannot borrow
    /// a different runtime/generation or revive an expired original lifetime.
    pub(crate) fn connection_lifetime_current(
        &self,
        original: &NativeMediaOrigin,
        physical: Option<&ConnectionBinding>,
    ) -> connect_protocol::Result<bool> {
        self.connection_lifetime_generation(original, physical)
            .map(|_| true)
    }
    pub(crate) fn connection_lifetime_generation(
        &self,
        original: &NativeMediaOrigin,
        physical: Option<&ConnectionBinding>,
    ) -> connect_protocol::Result<connect_store::NativeConnectionLifetimeStamp> {
        let entry = self
            .entry(&ConnectionId::from_string(original.runtime_connection_id()))
            .map_err(|_| connect_protocol::Error::forbidden())?;
        observe_selected_entry(
            &entry,
            || {
                if let Some(physical) = physical {
                    let current = entry.lifetime_binding().map_err(|_| busy())?;
                    if !current.same_incarnation(physical) {
                        return Err(connect_protocol::Error::forbidden());
                    }
                }
                let lifetime = entry
                    .lifetime
                    .snapshot()
                    .map_err(|_| connect_protocol::Error::forbidden())?;
                let Some(lifetime) = lifetime.as_ref() else {
                    entry.prepare_lifetime_snapshot(&self.store)?;
                    return Err(busy());
                };
                lifetime.observe_original_generation(original)
            },
            || {
                self.entry(&ConnectionId::from_string(original.runtime_connection_id()))
                    .ok()
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, thread, time::Duration};
    #[test]
    fn replaced_selected_lifetime_refusal_is_retryable_and_next_successor_is_observed() {
        let original = Arc::new(1u64);
        let successor = Arc::new(2u64);
        let published = Arc::new(Published::new(original.clone()));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (resume_tx, resume_rx) = mpsc::channel();
        let reader = published.clone();
        let worker = thread::spawn(move || {
            let selected = reader.snapshot().unwrap().as_ref().clone();
            observe_selected_entry(
                &selected,
                || {
                    entered_tx.send(()).unwrap();
                    resume_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                    Err::<bool, _>(connect_protocol::Error::forbidden())
                },
                || reader.snapshot().ok().map(|entry| entry.as_ref().clone()),
            )
        });
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        *published.lock().unwrap() = successor.clone();
        resume_tx.send(()).unwrap();
        let old_result = worker.join().unwrap();
        assert_eq!(old_result.unwrap_err().code, "authority-busy");
        let selected = published.snapshot().unwrap().as_ref().clone();
        assert!(Arc::ptr_eq(&selected, &successor));
        assert!(observe_selected_entry(
            &selected,
            || Ok(true),
            || published
                .snapshot()
                .ok()
                .map(|entry| entry.as_ref().clone())
        )
        .unwrap());
    }
    #[test]
    fn unchanged_selected_lifetime_preserves_definitive_and_binding_refusals() {
        let selected = Arc::new(1u64);
        let refused = observe_selected_entry(
            &selected,
            || Err::<bool, _>(connect_protocol::Error::forbidden()),
            || Some(selected.clone()),
        );
        assert_eq!(refused.unwrap_err().code, "forbidden");
        let replaced = Arc::new(1u64); // Equal metadata cannot replace object identity.
        let stale_binding = observe_selected_entry(
            &selected,
            || Err::<bool, _>(connect_protocol::Error::forbidden()),
            || Some(replaced),
        );
        assert_eq!(stale_binding.unwrap_err().code, "authority-busy");
    }
}
