//! Process-local operational state, separate from the authoritative journal.
use connect_protocol::{ComponentReadiness, Readiness, ReadinessState};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, Weak},
};
pub type ResourceObserver = dyn Fn() -> Option<connect_protocol::ResourceUsage> + Send + Sync;
type MediaSnapshot = Arc<dyn Fn() -> connect_protocol::MediaDiagnostics + Send + Sync>;
use tokio_util::sync::CancellationToken;

impl crate::Service {
    pub fn endpoint_observation_epoch(&self) -> &str {
        self.runtime.endpoints.epoch()
    }
    /// Trusted runtime composition only. Public requests cannot publish
    /// reachability or turn a self-reported address into verified ownership.
    pub async fn observe_endpoint(
        &self,
        observation: connect_store::EndpointObservation,
    ) -> connect_protocol::Result<()> {
        let store = self.store.clone();
        let runtime = self.runtime.endpoints.clone();
        tokio::task::spawn_blocking(move || {
            store.record_endpoint_observation(&runtime, observation)
        })
        .await
        .map_err(|_| connect_protocol::Error::storage())?
    }
}

/// Shared by runtime supervisors and authenticated health responses.
#[derive(Default)]
pub struct OperationalReadiness {
    components: Mutex<BTreeMap<&'static str, ComponentReadiness>>,
    fatal: CancellationToken,
    media: Mutex<Option<MediaSnapshot>>,
    resources: Mutex<BTreeMap<connect_protocol::ResourcePoolKind, Weak<ResourceObserver>>>,
}
impl OperationalReadiness {
    /// The caller retains the returned owner. The registry keeps only a Weak
    /// reference, so observing a listener cannot retain its service/worker cycle.
    pub fn register_resource_pool<F>(
        &self,
        kind: connect_protocol::ResourcePoolKind,
        source: F,
    ) -> connect_protocol::Result<Arc<ResourceObserver>>
    where
        F: Fn() -> Option<connect_protocol::ResourceUsage> + Send + Sync + 'static,
    {
        let mut pools = self
            .resources
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?;
        if pools.get(&kind).and_then(Weak::upgrade).is_some() {
            return Err(connect_protocol::Error::new(
                "conflict",
                "Resource owner already registered",
            ));
        }
        if !pools.contains_key(&kind) && pools.len() >= connect_protocol::MAX_OPERATIONAL_POOLS {
            return Err(connect_protocol::Error::new(
                "capacity-exceeded",
                "Resource observation capacity exceeded",
            ));
        }
        let owner: Arc<ResourceObserver> = Arc::new(source);
        pools.insert(kind, Arc::downgrade(&owner));
        Ok(owner)
    }
    /// Never wait for registry contention and never execute probes under its lock.
    /// A retired or busy owner cannot be presented as zero resource usage.
    pub fn resource_pool_snapshot(&self) -> Option<Vec<connect_protocol::ResourcePool>> {
        use connect_protocol::{ResourcePool, ResourceSampleState, MAX_OPERATIONAL_COUNTER};
        let sources: Vec<_> = self
            .resources
            .try_lock()
            .ok()?
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect();
        Some(
            sources
                .into_iter()
                .map(|(kind, source)| {
                    let (state, usage) = match source.upgrade() {
                        None => (ResourceSampleState::Retired, None),
                        Some(source) => match source() {
                            None => (ResourceSampleState::Busy, None),
                            Some(mut value) => {
                                value.accepted = value.accepted.min(MAX_OPERATIONAL_COUNTER);
                                value.rejected = value.rejected.min(MAX_OPERATIONAL_COUNTER);
                                (ResourceSampleState::Available, Some(value))
                            }
                        },
                    };
                    ResourcePool { kind, state, usage }
                })
                .collect(),
        )
    }

    /// Install a process-local bounded numeric provider. Authorization remains
    /// the existing Health operation's responsibility.
    pub fn set_media_snapshot(&self, provider: MediaSnapshot) {
        *self.media.lock().unwrap_or_else(|p| p.into_inner()) = Some(provider);
    }
    /// Publish a bounded, static component/reason pair without private data.
    pub fn set(&self, component: &'static str, state: ReadinessState, reason: &'static str) {
        self.components
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                component,
                ComponentReadiness {
                    component: component.into(),
                    state,
                    reason: reason.into(),
                    observed_at: chrono::Utc::now().timestamp(),
                    configured: true,
                },
            );
    }
    /// An optional component was not enabled. Expose that fact without failing
    /// configured components or claiming a remote worker is running.
    pub fn unconfigured(&self, component: &'static str) {
        self.components
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                component,
                ComponentReadiness {
                    component: component.into(),
                    state: ReadinessState::Unavailable,
                    reason: "not-configured".into(),
                    observed_at: chrono::Utc::now().timestamp(),
                    configured: false,
                },
            );
    }
    /// A worker cannot continue safely. Wake supervision without replaying effects.
    pub fn fail(&self, component: &'static str) {
        self.set(component, ReadinessState::Unavailable, "worker-exited");
        self.fatal.cancel();
    }
    /// Wait without a lost-wakeup race, including failures before subscription.
    pub async fn failed(&self) {
        self.fatal.cancelled().await;
    }

    /// Optional unconfigured components are explicit and do not degrade the
    /// aggregate. No configured observations still means uncertain, not ready.
    pub fn snapshot(&self) -> Readiness {
        let components: Vec<_> = self
            .components
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .cloned()
            .collect();
        let provider = self.media.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let mut readiness = Readiness::from_components(components);
        readiness.media = provider.map(|provider| provider());
        readiness
    }
}
#[cfg(test)]
#[path = "readiness_tests.rs"]
mod tests;
