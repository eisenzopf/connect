//! Admission counters share the exact semaphore that owns the resource.
//! Observation never transfers protected-response custody.
use crate::readiness::{OperationalReadiness, ResourceObserver};
use connect_protocol::{ResourcePoolKind, ResourceUsage, MAX_OPERATIONAL_COUNTER};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, SemaphorePermit, TryAcquireError};
use tokio_util::sync::CancellationToken;

pub struct ResourceBudget {
    semaphore: Arc<Semaphore>,
    limit: u32,
    accepted: AtomicU64,
    rejected: AtomicU64,
}
impl ResourceBudget {
    pub fn new(limit: usize) -> connect_protocol::Result<Arc<Self>> {
        let limit = u32::try_from(limit)
            .ok()
            .filter(|v| *v > 0 && (*v as usize) <= Semaphore::MAX_PERMITS)
            .ok_or_else(|| {
                connect_protocol::Error::new("invalid-config", "Invalid resource capacity")
            })?;
        Ok(Arc::new(Self {
            semaphore: Arc::new(Semaphore::new(limit as usize)),
            limit,
            accepted: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
        }))
    }
    fn record<T>(&self, result: &Result<T, TryAcquireError>) {
        let counter = if result.is_ok() {
            &self.accepted
        } else {
            &self.rejected
        };
        let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
            Some(v.saturating_add(1).min(MAX_OPERATIONAL_COUNTER))
        });
    }
    pub fn try_acquire(&self) -> Result<SemaphorePermit<'_>, TryAcquireError> {
        let result = self.semaphore.try_acquire();
        self.record(&result);
        result
    }
    pub fn try_acquire_owned(self: Arc<Self>) -> Result<OwnedSemaphorePermit, TryAcquireError> {
        let result = self.semaphore.clone().try_acquire_owned();
        self.record(&result);
        result
    }
    pub fn observe(
        self: &Arc<Self>,
        readiness: &OperationalReadiness,
        kind: ResourcePoolKind,
        stop: CancellationToken,
    ) -> connect_protocol::Result<Arc<ResourceObserver>> {
        let budget = self.clone();
        readiness.register_resource_pool(kind, move || {
            Some(ResourceUsage {
                used: budget
                    .limit
                    .saturating_sub(budget.semaphore.available_permits() as u32),
                limit: budget.limit,
                accepted: budget.accepted.load(Ordering::Relaxed),
                rejected: budget.rejected.load(Ordering::Relaxed),
                draining: stop.is_cancelled(),
            })
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_permits_refusal_drain_and_owner_retirement_are_observed() {
        let readiness = OperationalReadiness::default();
        let stop = CancellationToken::new();
        let budget = ResourceBudget::new(1).unwrap();
        let observer = budget
            .observe(&readiness, ResourcePoolKind::HttpRequests, stop.clone())
            .unwrap();
        let permit = budget.clone().try_acquire_owned().unwrap();
        assert!(budget.try_acquire().is_err());
        let sample = || readiness.resource_pool_snapshot().unwrap().remove(0);
        assert_eq!(
            sample().usage.unwrap(),
            ResourceUsage {
                used: 1,
                limit: 1,
                accepted: 1,
                rejected: 1,
                draining: false,
            }
        );
        drop(permit);
        let borrowed = budget.try_acquire().unwrap();
        assert_eq!(sample().usage.unwrap().accepted, 2);
        drop(borrowed);
        stop.cancel();
        assert_eq!(
            sample().usage.unwrap(),
            ResourceUsage {
                used: 0,
                limit: 1,
                accepted: 2,
                rejected: 1,
                draining: true,
            }
        );
        let weak = Arc::downgrade(&budget);
        drop(budget);
        assert!(weak.upgrade().is_some());
        drop(observer);
        assert!(weak.upgrade().is_none());
        assert_eq!(
            sample().state,
            connect_protocol::ResourceSampleState::Retired
        );
        assert!(sample().usage.is_none());
    }
    #[test]
    fn invalid_bounds_and_saturated_counters_are_explicit() {
        assert!(ResourceBudget::new(0).is_err());
        assert!(ResourceBudget::new(usize::MAX).is_err());
        let budget = ResourceBudget::new(1).unwrap();
        budget
            .accepted
            .store(MAX_OPERATIONAL_COUNTER, Ordering::Relaxed);
        let _permit = budget.try_acquire().unwrap();
        assert_eq!(
            budget.accepted.load(Ordering::Relaxed),
            MAX_OPERATIONAL_COUNTER
        );
        budget
            .rejected
            .store(MAX_OPERATIONAL_COUNTER, Ordering::Relaxed);
        assert!(budget.try_acquire().is_err());
        assert_eq!(
            budget.rejected.load(Ordering::Relaxed),
            MAX_OPERATIONAL_COUNTER
        );
    }
}
