//! Weak allocation identity only, never an IO or authorization capability.
//!
//! Erase route metadata's owner type so its Send/Sync proof need not repeatedly
//! expand the cross-Service connection/policy graph. The cold constructor still
//! requires the actual concrete owner to satisfy both traits. No owner is kept
//! alive; the Weak allocation prevents address reuse until this record drops.
use std::{
    any::Any,
    sync::{Arc, Weak},
};

pub(crate) struct WeakIdentity(Weak<dyn Any + Send + Sync>);
impl WeakIdentity {
    pub(crate) fn new<T: Any + Send + Sync>(owner: &Arc<T>) -> Self {
        let weak = Arc::downgrade(owner);
        let erased: Weak<dyn Any + Send + Sync> = weak;
        Self(erased)
    }
    pub(crate) fn matches<T: ?Sized>(&self, current: &Arc<T>) -> bool {
        // The actual caller Arc proves liveness. The stored Weak prevents ABA;
        // address equality therefore means the same allocation without a
        // per-packet Weak upgrade or temporary strong/erased Arc.
        std::ptr::addr_eq(self.0.as_ptr(), Arc::as_ptr(current))
    }
    pub(crate) fn live(&self) -> bool {
        self.0.strong_count() > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Owner {
        drops: Arc<AtomicUsize>,
        generation: u64,
    }
    impl Drop for Owner {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::AcqRel);
        }
    }
    #[test]
    fn erased_identity_requires_same_allocation_even_for_equal_metadata() {
        let drops = Arc::new(AtomicUsize::new(0));
        let owner = Arc::new(Owner {
            drops: drops.clone(),
            generation: 7,
        });
        let equal = Arc::new(Owner {
            drops: drops.clone(),
            generation: 7,
        });
        assert_eq!(owner.generation, equal.generation);
        let weak = WeakIdentity::new(&owner);
        let clone = owner.clone();
        let erased: Arc<dyn Any + Send + Sync> = owner.clone();
        assert!(weak.matches(&owner) && weak.matches(&clone) && weak.matches(&erased));
        assert!(!weak.matches(&equal));
        drop(erased);
        drop(clone);
        assert!(weak.live());
    }
    #[test]
    fn erased_identity_never_retains_owner_or_accepts_successor_after_drop() {
        let drops = Arc::new(AtomicUsize::new(0));
        let owner = Arc::new(Owner {
            drops: drops.clone(),
            generation: 3,
        });
        let weak = WeakIdentity::new(&owner);
        assert_eq!(Arc::strong_count(&owner), 1);
        assert_eq!(Arc::weak_count(&owner), 1);
        drop(owner);
        assert_eq!(drops.load(Ordering::Acquire), 1);
        assert!(!weak.live());
        let successor = Arc::new(Owner {
            drops: drops.clone(),
            generation: 3,
        });
        assert!(!weak.matches(&successor));
        drop(weak);
        assert_eq!(drops.load(Ordering::Acquire), 1);
        drop(successor);
        assert_eq!(drops.load(Ordering::Acquire), 2);
    }
}
