//! Retry only leaf registry lookups whose NotReady means mutex contention.
//! These lookups clone metadata and hold no Store/policy/IO authority reader.
use rvoip_core::staged_streams::{ControlResult, StreamControlError};
use std::time::Instant;

pub(super) fn read_until<T>(
    until: Instant,
    mut read: impl FnMut() -> ControlResult<T>,
) -> ControlResult<T> {
    loop {
        match read() {
            Err(StreamControlError::NotReady) if Instant::now() < until => {
                std::thread::yield_now();
            }
            result => return result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc, Mutex, TryLockError};
    use std::time::Duration;

    fn concurrent_lookup(after: Option<u32>) -> ControlResult<Option<u32>> {
        let value = Arc::new(Mutex::new(Some(7)));
        let other = value.clone();
        let (held_tx, held_rx) = mpsc::sync_channel(0);
        let (observed_tx, observed_rx) = mpsc::sync_channel(0);
        let holder = std::thread::spawn(move || {
            let mut held = other.lock().unwrap();
            held_tx.send(()).unwrap();
            observed_rx.recv().unwrap();
            *held = after;
        });
        held_rx.recv().unwrap();
        let mut observed = false;
        let result = read_until(Instant::now() + Duration::from_secs(1), || {
            match value.try_lock() {
                Ok(value) => Ok(*value),
                Err(TryLockError::WouldBlock) => {
                    if !observed {
                        observed = true;
                        observed_tx.send(()).unwrap();
                    }
                    Err(StreamControlError::NotReady)
                }
                Err(TryLockError::Poisoned(_)) => Err(StreamControlError::Authority),
            }
        });
        holder.join().unwrap();
        assert!(observed, "actual mutex exclusion must precede release");
        result
    }
    #[test]
    fn busy_lookup_rechecks_the_original_registry_after_release() {
        assert_eq!(concurrent_lookup(Some(42)).unwrap(), Some(42));
    }
    #[test]
    fn withdrawn_metadata_is_not_replaced_with_a_cached_positive() {
        assert_eq!(concurrent_lookup(None).unwrap(), None);
    }
    #[test]
    fn expired_original_budget_does_not_start_another_lookup() {
        let value = Mutex::new(7);
        let held = value.lock().unwrap();
        let mut attempts = 0;
        let result = read_until(Instant::now(), || {
            attempts += 1;
            value
                .try_lock()
                .map(|value| *value)
                .map_err(|_| StreamControlError::NotReady)
        });
        assert!(matches!(result, Err(StreamControlError::NotReady)));
        assert_eq!(attempts, 1);
        drop(held);
    }
    #[test]
    fn poisoned_registry_remains_an_immediate_authority_refusal() {
        let value = Arc::new(Mutex::new(7));
        let other = value.clone();
        assert!(std::thread::spawn(move || {
            let _held = other.lock().unwrap();
            panic!("controlled registry poison");
        })
        .join()
        .is_err());
        let mut attempts = 0;
        let result = read_until(Instant::now() + Duration::from_secs(1), || {
            attempts += 1;
            match value.try_lock() {
                Ok(value) => Ok(*value),
                Err(TryLockError::WouldBlock) => Err(StreamControlError::NotReady),
                Err(TryLockError::Poisoned(_)) => Err(StreamControlError::Authority),
            }
        });
        assert!(matches!(result, Err(StreamControlError::Authority)));
        assert_eq!(attempts, 1);
    }
}
