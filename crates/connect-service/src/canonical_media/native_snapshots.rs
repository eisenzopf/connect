//! Per-route immutable authorization. Registration is a bounded route-change
//! operation; established delivery only loads a published handle and atomics.
use super::public_continuation::Continuation;
use super::*;
use connect_store::{NativeMediaSnapshot, NativeReplacementActivation};

// A phase is an exact object identity, not its equal logical operation IDs.
// Stored weak owners prevent address reuse from borrowing a previous phase.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct PhaseKey {
    continuation: usize,
    activation: usize,
}
fn phase_key<C, A>(continuation: &Arc<C>, activation: &Arc<A>) -> PhaseKey {
    PhaseKey {
        continuation: Arc::as_ptr(continuation) as usize,
        activation: Arc::as_ptr(activation) as usize,
    }
}
// Connection origins are immutable. Exact Arc identity plus generation/epoch
// therefore already pins its runtime ID. The retained Weak below both prevents
// address reuse and verifies the actual owner; no per-packet String is needed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SourceKey {
    generation: u64,
    credential_epoch: u64,
    owner: usize,
}
fn source_key<T>(source: &Arc<T>, generation: u64, credential_epoch: u64) -> SourceKey {
    SourceKey {
        generation,
        credential_epoch,
        owner: Arc::as_ptr(source) as usize,
    }
}
fn same_source_owner<T>(registered: Option<&Weak<T>>, current: Option<&Arc<T>>) -> bool {
    match (registered, current) {
        (None, None) => true,
        (Some(registered), Some(current)) => registered
            .upgrade()
            .is_some_and(|registered| Arc::ptr_eq(&registered, current)),
        _ => false,
    }
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct RouteKey {
    source: Option<SourceKey>,
    target_phase: Option<PhaseKey>,
    source_phase: Option<PhaseKey>,
}
const MAX_ROUTE_SNAPSHOTS: usize = 1024;
#[derive(Clone)]
struct PhaseBinding {
    continuation: Weak<Continuation>,
    activation: Weak<NativeReplacementActivation>,
}
impl PhaseBinding {
    fn new(current: &Arc<Continuation>, entry: &Arc<Connection>) -> connect_protocol::Result<Self> {
        if !Arc::ptr_eq(&current.entry, entry)
            || !Arc::ptr_eq(current.metadata.origin(), &entry.origin)
            || current.ordinary()
        {
            return Err(connect_protocol::Error::forbidden());
        }
        Ok(Self {
            continuation: Arc::downgrade(current),
            activation: Arc::downgrade(&current.metadata),
        })
    }
    fn upgrade(&self, entry: &Arc<Connection>) -> connect_protocol::Result<Arc<Continuation>> {
        let current = self
            .continuation
            .upgrade()
            .ok_or_else(connect_protocol::Error::forbidden)?;
        let activation = self
            .activation
            .upgrade()
            .ok_or_else(connect_protocol::Error::forbidden)?;
        if !Arc::ptr_eq(&activation, &current.metadata) {
            return Err(connect_protocol::Error::forbidden());
        }
        Self::new(&current, entry)?;
        Ok(current)
    }
    fn matches(&self, current: &Arc<Continuation>, entry: &Arc<Connection>) -> bool {
        self.upgrade(entry)
            .is_ok_and(|registered| Arc::ptr_eq(&registered, current))
    }
    fn live(&self) -> bool {
        self.continuation
            .upgrade()
            .is_some_and(|current| !current.ordinary())
            && self.activation.strong_count() > 0
    }
}
#[derive(Clone)]
pub(super) struct RouteSnapshot {
    source: Option<Weak<Connection>>,
    target_phase: Option<PhaseBinding>,
    source_phase: Option<PhaseBinding>,
    snapshot: NativeMediaSnapshot,
}
impl RouteSnapshot {
    fn live(&self) -> bool {
        self.source
            .as_ref()
            .is_none_or(|source| source.strong_count() > 0)
            && self.target_phase.as_ref().is_none_or(PhaseBinding::live)
            && self.source_phase.as_ref().is_none_or(PhaseBinding::live)
    }
}
pub(super) type Routes = BTreeMap<RouteKey, RouteSnapshot>;
fn key(
    source: Option<&Arc<Connection>>,
    target_phase: Option<&Arc<Continuation>>,
    source_phase: Option<&Arc<Continuation>>,
) -> RouteKey {
    RouteKey {
        source: source.map(|source| {
            source_key(
                source,
                source.origin.generation(),
                source.origin.credential_epoch(),
            )
        }),
        target_phase: target_phase.map(|current| phase_key(current, &current.metadata)),
        source_phase: source_phase.map(|current| phase_key(current, &current.metadata)),
    }
}
fn busy() -> connect_protocol::Error {
    connect_protocol::Error::new(
        "authority-busy",
        "Media route authorization is being prepared",
    )
}
pub(super) fn phase_error(error: StreamControlError) -> connect_protocol::Error {
    match error {
        StreamControlError::MetadataBusy | StreamControlError::NotReady => busy(),
        _ => connect_protocol::Error::forbidden(),
    }
}
fn phase_matches(
    registered: Option<&PhaseBinding>,
    current: Option<&Arc<Continuation>>,
    entry: &Arc<Connection>,
) -> bool {
    match (registered, current) {
        (None, None) => true,
        (Some(registered), Some(current)) => registered.matches(current, entry),
        _ => false,
    }
}
// Classification only: metadata.target is never used as an IO permission. The
// actual Core activation/current managed route and delivery ticket must agree
// before enrollment. Release that local ticket before worker SQL acquisition.
fn require_current_phase(
    current: &Arc<Continuation>,
    entry: &Arc<Connection>,
) -> connect_protocol::Result<()> {
    PhaseBinding::new(current, entry)?;
    let policy = entry
        .domain
        .upgrade()
        .ok_or_else(connect_protocol::Error::forbidden)?;
    let target = policy
        .replacement_target(&ConnectionId::from_string(
            entry.origin.runtime_connection_id(),
        ))
        .map_err(phase_error)?
        .ok_or_else(connect_protocol::Error::forbidden)?;
    let installed = target
        .current_continuation()
        .map_err(phase_error)?
        .ok_or_else(busy)?;
    if !Arc::ptr_eq(&installed, current) {
        return Err(connect_protocol::Error::forbidden());
    }
    let owner = entry
        .owner(current.metadata.target(), &entry.principal, false)
        .map_err(phase_error)?;
    let owner = entry
        .activation()
        .map_err(phase_error)?
        .ok_or_else(busy)?
        .current(owner)
        .map_err(phase_error)?;
    drop(current.current(&owner).map_err(phase_error)?);
    Ok(())
}
fn defer_eager_enrollment(result: connect_protocol::Result<()>) -> connect_protocol::Result<()> {
    match result {
        Err(error) if error.code == "authority-busy" => Ok(()),
        result => result,
    }
}
impl Connection {
    /// Setup retains its original control owner when effect-free enrollment is
    /// busy. A later packet miss remains a refusal until a real snapshot exists.
    pub(super) fn prepare_snapshot_route(
        self: &Arc<Self>,
        store: &Arc<Store>,
        source: Option<&Arc<Connection>>,
    ) -> connect_protocol::Result<()> {
        defer_eager_enrollment(self.prepare_lifetime_snapshot(store))?;
        defer_eager_enrollment(self.register_snapshot_route(store, source))
    }
    pub(super) fn register_snapshot_route(
        self: &Arc<Self>,
        store: &Arc<Store>,
        source: Option<&Arc<Connection>>,
    ) -> connect_protocol::Result<()> {
        self.register_snapshot_route_for(store, source, None, None)
    }
    fn register_snapshot_route_for(
        self: &Arc<Self>,
        store: &Arc<Store>,
        source: Option<&Arc<Connection>>,
        target_continuation: Option<&Arc<Continuation>>,
        source_continuation: Option<&Arc<Continuation>>,
    ) -> connect_protocol::Result<()> {
        let target_phase = target_continuation
            .map(|current| PhaseBinding::new(current, self))
            .transpose()?;
        let source_phase = source_continuation
            .map(|current| {
                PhaseBinding::new(
                    current,
                    source.ok_or_else(connect_protocol::Error::forbidden)?,
                )
            })
            .transpose()?;
        // Actual committed Core continuation, not merely preallocated metadata.
        // No SQL and no ticket is retained beyond this classification check.
        if let Some(current) = target_continuation {
            require_current_phase(current, self)?;
        }
        if let Some(current) = source_continuation {
            require_current_phase(
                current,
                source.ok_or_else(connect_protocol::Error::forbidden)?,
            )?;
        }
        let mut routes = self.snapshots.try_lock().map_err(|error| match error {
            std::sync::TryLockError::WouldBlock => busy(),
            std::sync::TryLockError::Poisoned(_) => connect_protocol::Error::forbidden(),
        })?;
        let route = key(source, target_continuation, source_continuation);
        if routes.get(&route).is_some_and(RouteSnapshot::live) {
            return Ok(());
        }
        // Inspect capacity without pruning: any subsequent authority-busy is
        // definite no effect, including no withdrawal of an existing epoch.
        if routes.values().filter(|route| route.live()).count() >= MAX_ROUTE_SNAPSHOTS {
            return Err(connect_protocol::Error::new(
                "capacity-exceeded",
                "Media authorization route capacity exceeded",
            ));
        }
        let weak_store = Arc::downgrade(store);
        let weak_target = Arc::downgrade(self);
        let weak_source = source.map(Arc::downgrade);
        let captured_source = weak_source.clone();
        let captured_target_phase = target_phase.clone();
        let captured_source_phase = source_phase.clone();
        let snapshot = store.try_register_native_media_snapshot_for(
            &self.origin,
            source.map(|source| source.origin.as_ref()),
            move || {
                let store = weak_store
                    .upgrade()
                    .ok_or_else(connect_protocol::Error::forbidden)?;
                let target = weak_target
                    .upgrade()
                    .ok_or_else(connect_protocol::Error::forbidden)?;
                let source = captured_source
                    .as_ref()
                    .map(|source| {
                        source
                            .upgrade()
                            .ok_or_else(connect_protocol::Error::forbidden)
                    })
                    .transpose()?;
                let target_phase = captured_target_phase
                    .as_ref()
                    .map(|phase| phase.upgrade(&target))
                    .transpose()?;
                let source_phase = captured_source_phase
                    .as_ref()
                    .map(|phase| {
                        phase.upgrade(
                            source
                                .as_ref()
                                .ok_or_else(connect_protocol::Error::forbidden)?,
                        )
                    })
                    .transpose()?;
                if let Some(current) = &target_phase {
                    require_current_phase(current, &target)?;
                }
                if let Some(current) = &source_phase {
                    require_current_phase(
                        current,
                        source
                            .as_ref()
                            .ok_or_else(connect_protocol::Error::forbidden)?,
                    )?;
                }
                // SQL exists only in this bounded owned worker callback. No
                // ordinary fallback is allowed after an exact pending refusal.
                let read = if target_phase.is_some() || source_phase.is_some() {
                    store.try_native_replacement_continuation_media_read(
                        &target.origin,
                        target_phase.as_ref().map(|phase| phase.metadata.as_ref()),
                        source.as_ref().map(|source| {
                            (
                                source.origin.as_ref(),
                                source_phase.as_ref().map(|phase| phase.metadata.as_ref()),
                            )
                        }),
                        None,
                    )?
                } else {
                    store.try_native_media_read(
                        &target.origin,
                        source.as_ref().map(|source| source.origin.as_ref()),
                    )?
                };
                // The Read interval now exists. Recheck physical continuation
                // after SQL, without retaining any Core mutex while acquiring it.
                if let Some(current) = &target_phase {
                    let owner = target
                        .owner(read.target(), &target.principal, source.is_some())
                        .map_err(phase_error)?;
                    let owner = target
                        .activation()
                        .map_err(phase_error)?
                        .ok_or_else(busy)?
                        .current(owner)
                        .map_err(phase_error)?;
                    drop(current.current(&owner).map_err(phase_error)?);
                }
                if let Some(current) = &source_phase {
                    let source = source
                        .as_ref()
                        .ok_or_else(connect_protocol::Error::forbidden)?;
                    let owner = source
                        .owner(
                            read.source()
                                .ok_or_else(connect_protocol::Error::forbidden)?,
                            &source.principal,
                            false,
                        )
                        .map_err(phase_error)?;
                    let owner = source
                        .activation()
                        .map_err(phase_error)?
                        .ok_or_else(busy)?
                        .current(owner)
                        .map_err(phase_error)?;
                    drop(current.current(&owner).map_err(phase_error)?);
                }
                Ok(read)
            },
        )?;
        // No fallible operation follows publication. Only after successful
        // enrollment may old dead/ordinary phase handles be withdrawn.
        routes.retain(|_, registered| {
            let live = registered.live();
            if !live {
                tracing::debug!(target: "connect_service::packet_refusal",
                    connection=self.origin.connection_id(),
                    old_target_phase=registered.target_phase.is_some(),
                    old_source_phase=registered.source_phase.is_some(),
                    new_target_phase=target_phase.is_some(), new_source_phase=source_phase.is_some(),
                    unary=registered.source.is_none(),
                    "Retired phase snapshot handle withdrawn at route registration");
            }
            live
        });
        routes.insert(
            route,
            RouteSnapshot {
                source: weak_source,
                target_phase,
                source_phase,
                snapshot,
            },
        );
        Ok(())
    }
    /// Control-plane preparation only. All route-map guards are gone before
    /// the Store factory runs. The packet path continues using snapshot_read_for.
    pub(super) fn prepare_snapshot_route_current(
        self: &Arc<Self>,
        store: &Arc<Store>,
        source: Option<&Arc<Connection>>,
        target_continuation: Option<&Arc<Continuation>>,
        source_continuation: Option<&Arc<Continuation>>,
    ) -> connect_protocol::Result<()> {
        self.register_snapshot_route_for(store, source, target_continuation, source_continuation)?;
        let route = key(source, target_continuation, source_continuation);
        let snapshot = {
            let routes = self
                .snapshots
                .snapshot()
                .map_err(|_| connect_protocol::Error::forbidden())?;
            let registered = routes.get(&route).ok_or_else(busy)?;
            if !registered.live() || !same_source_owner(registered.source.as_ref(), source) {
                return Err(connect_protocol::Error::forbidden());
            }
            registered.snapshot.clone()
        };
        store.try_prepare_native_media_snapshot(&snapshot)?;
        if let Some(current) = target_continuation {
            require_current_phase(current, self)?;
        }
        if let Some(current) = source_continuation {
            require_current_phase(
                current,
                source.ok_or_else(connect_protocol::Error::forbidden)?,
            )?;
        }
        Ok(())
    }

    #[cfg(feature = "test-support")]
    pub(super) fn withdraw_unary_route_for_test(
        &self,
        phase: Option<&Arc<Continuation>>,
    ) -> connect_protocol::Result<()> {
        let mut routes = self.snapshots.try_lock().map_err(|_| busy())?;
        routes.remove(&key(None, phase, None));
        Ok(())
    }

    pub(super) fn snapshot_read_for(
        self: &Arc<Self>,
        store: &Arc<Store>,
        source: Option<&Arc<Connection>>,
        original: Option<&NativeMediaRead>,
        target_continuation: Option<&Arc<Continuation>>,
        source_continuation: Option<&Arc<Continuation>>,
    ) -> connect_protocol::Result<NativeMediaRead> {
        let route = key(source, target_continuation, source_continuation);
        let routes = self
            .snapshots
            .snapshot()
            .map_err(|_| connect_protocol::Error::forbidden())?;
        let Some(published) = routes.get(&route) else {
            drop(routes);
            #[cfg(feature = "test-support")]
            if source.is_none() {
                if let Some(policy) = self.domain.upgrade() {
                    policy.observe_cold_unary_miss_for_test(self.origin.runtime_connection_id());
                }
            }
            self.register_snapshot_route_for(
                store,
                source,
                target_continuation,
                source_continuation,
            )?;
            return Err(busy());
        };
        if !published.live() {
            drop(routes);
            self.register_snapshot_route_for(
                store,
                source,
                target_continuation,
                source_continuation,
            )?;
            return Err(busy());
        }
        if !same_source_owner(published.source.as_ref(), source) {
            return Err(connect_protocol::Error::forbidden());
        }
        if !phase_matches(published.target_phase.as_ref(), target_continuation, self)
            || match source {
                Some(source) => {
                    !phase_matches(published.source_phase.as_ref(), source_continuation, source)
                }
                None => source_continuation.is_some() || published.source_phase.is_some(),
            }
        {
            return Err(connect_protocol::Error::forbidden());
        }
        let read = published.snapshot.admit()?;
        match original {
            Some(original) => read.bound_to_original_source(
                original,
                source.map_or(self.origin.as_ref(), |source| source.origin.as_ref()),
            ),
            None => Ok(read),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn route_source_key_requires_exact_owner_generation_and_credential_epoch() {
        let source = Arc::new("same-runtime-id");
        let other = Arc::new("same-runtime-id");
        let weak = Arc::downgrade(&source);
        let original = source_key(&source, 2, 3);
        assert_eq!(original, source_key(&source.clone(), 2, 3));
        assert_ne!(original, source_key(&other, 2, 3));
        assert_ne!(original, source_key(&source, 4, 3));
        assert_ne!(original, source_key(&source, 2, 4));
        assert!(same_source_owner(Some(&weak), Some(&source)));
        assert!(!same_source_owner(Some(&weak), Some(&other)));
        assert!(!same_source_owner(Some(&weak), None));
        assert!(!same_source_owner(None, Some(&source)));
        assert!(same_source_owner::<&str>(None, None));
        assert_eq!(
            Arc::strong_count(&source),
            1,
            "key/validation must not pin the physical owner"
        );
    }
    #[test]
    fn dead_route_source_cannot_be_resurrected_by_equal_runtime_metadata() {
        let source = Arc::new("same-runtime-id");
        let weak = Arc::downgrade(&source);
        let key = source_key(&source, 2, 3);
        drop(source);
        let successor = Arc::new("same-runtime-id");
        assert!(weak.upgrade().is_none());
        assert!(!same_source_owner(Some(&weak), Some(&successor)));
        assert_ne!(
            key,
            source_key(&successor, 2, 3),
            "retained Weak prevents allocation address reuse"
        );
        assert_eq!(Arc::strong_count(&successor), 1);
    }
    #[test]
    fn eager_enrollment_defers_only_effect_free_authority_busy() {
        assert!(defer_eager_enrollment(Ok(())).is_ok());
        assert!(defer_eager_enrollment(Err(busy())).is_ok());
        for code in [
            "forbidden",
            "storage-unavailable",
            "capacity-exceeded",
            "cancelled",
            "deadline-exceeded",
        ] {
            let error =
                defer_eager_enrollment(Err(connect_protocol::Error::new(code, "original refusal")))
                    .unwrap_err();
            assert_eq!(error.code, code);
            assert_eq!(error.message, "original refusal");
        }
        // Only the eager wrapper tolerates this. The actual route-miss result
        // itself remains a refusal and never claims a published admission.
        assert_eq!(busy().code, "authority-busy");
    }
    #[test]
    fn continuation_phase_key_never_borrows_ordinary_or_another_activation() {
        let continuation = Arc::new(());
        let activation = Arc::new(());
        let other_continuation = Arc::new(());
        let other_activation = Arc::new(());
        let phase = phase_key(&continuation, &activation);
        assert!(phase == phase_key(&continuation.clone(), &activation.clone()));
        assert!(phase != phase_key(&other_continuation, &activation));
        assert!(phase != phase_key(&continuation, &other_activation));
        let ordinary = RouteKey {
            source: None,
            target_phase: None,
            source_phase: None,
        };
        let target = RouteKey {
            target_phase: Some(phase),
            ..ordinary.clone()
        };
        let source = RouteKey {
            source_phase: Some(phase),
            ..ordinary.clone()
        };
        let both = RouteKey {
            source_phase: Some(phase),
            ..target.clone()
        };
        let set = BTreeSet::from([ordinary, target, source, both]);
        assert_eq!(
            set.len(),
            4,
            "target/source pending facts are separate from ordinary facts"
        );
    }
}
