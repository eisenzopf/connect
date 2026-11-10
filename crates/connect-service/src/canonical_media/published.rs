//! Immutable routing metadata for media readers. Only state-change writers
//! enter the mutex. Publication happens before the writer releases its guard.
//! A routing snapshot is never an authorization grant.
use arc_swap::ArcSwap;
use std::ops::{Deref, DerefMut};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, LockResult, Mutex, MutexGuard, PoisonError, TryLockError, TryLockResult,
};

pub(super) struct Published<T: Clone> {
    value: ArcSwap<T>,
    writer: Mutex<T>,
    poisoned: AtomicBool,
}

impl<T: Clone + Default> Default for Published<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T: Clone> Published<T> {
    pub(super) fn new(value: T) -> Self {
        Self {
            value: ArcSwap::from_pointee(value.clone()),
            writer: Mutex::new(value),
            poisoned: AtomicBool::new(false),
        }
    }

    pub(super) fn snapshot(&self) -> Result<Arc<T>, ()> {
        if self.poisoned.load(Ordering::Acquire) {
            return Err(());
        }
        let value = self.value.load_full();
        if self.poisoned.load(Ordering::Acquire) {
            return Err(());
        }
        Ok(value)
    }

    fn guard<'a>(&'a self, value: MutexGuard<'a, T>) -> Write<'a, T> {
        Write { owner: self, value }
    }

    pub(super) fn lock(&self) -> LockResult<Write<'_, T>> {
        self.writer
            .lock()
            .map(|value| self.guard(value))
            .map_err(|error| PoisonError::new(self.guard(error.into_inner())))
    }

    pub(super) fn try_lock(&self) -> TryLockResult<Write<'_, T>> {
        self.writer
            .try_lock()
            .map(|value| self.guard(value))
            .map_err(|error| match error {
                TryLockError::WouldBlock => TryLockError::WouldBlock,
                TryLockError::Poisoned(error) => {
                    TryLockError::Poisoned(PoisonError::new(self.guard(error.into_inner())))
                }
            })
    }
}

pub(super) struct Write<'a, T: Clone> {
    owner: &'a Published<T>,
    value: MutexGuard<'a, T>,
}
impl<T: Clone> Deref for Write<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}
impl<T: Clone> DerefMut for Write<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.value
    }
}
impl<T: Clone> Drop for Write<'_, T> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.owner.poisoned.store(true, Ordering::Release);
        } else if !self.owner.poisoned.load(Ordering::Acquire) {
            self.owner.value.store(Arc::new(self.value.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, sync::mpsc, thread};

    #[test]
    fn readers_use_complete_published_map_while_writer_prepares_next_state() {
        let registry = Arc::new(Published::new(BTreeMap::from([(1, 7)])));
        let other = registry.clone();
        let (held_tx, held_rx) = mpsc::sync_channel(0);
        let (release_tx, release_rx) = mpsc::sync_channel(0);
        let writer = thread::spawn(move || {
            let mut next = other.lock().unwrap();
            next.remove(&1);
            next.insert(2, 9);
            held_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        });
        held_rx.recv().unwrap();
        let previous = registry.snapshot().unwrap();
        assert_eq!(*previous, BTreeMap::from([(1, 7)]));
        assert!(matches!(registry.try_lock(), Err(TryLockError::WouldBlock)));
        release_tx.send(()).unwrap();
        writer.join().unwrap();
        assert_eq!(*registry.snapshot().unwrap(), BTreeMap::from([(2, 9)]));
        assert_eq!(*previous, BTreeMap::from([(1, 7)]));
    }

    #[test]
    fn writer_poison_never_publishes_partial_state_or_serves_old_metadata() {
        let registry = Arc::new(Published::new(vec![1]));
        let other = registry.clone();
        assert!(thread::spawn(move || {
            let mut next = other.lock().unwrap();
            next.push(2);
            panic!("controlled publication failure");
        })
        .join()
        .is_err());
        assert!(registry.snapshot().is_err());
        assert!(registry.lock().is_err());
    }
}
