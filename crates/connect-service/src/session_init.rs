//! Protected hosting notifications after committed generic Session admission.
use async_trait::async_trait;
use connect_protocol::{session_control::Request, transitions::OperationState, Error, Result};
use std::sync::Arc;

/// Metadata only. Each host rechecks current authority and its exact target;
/// notification never retains the dispatcher's authorization reader.
#[derive(Clone)]
pub struct SessionInitialization {
    pub credential_id: String,
    pub request: Request,
}
#[async_trait]
pub trait SessionInitializer: Send + Sync {
    /// Admit/queue owned jobs and return; do not perform an inline media lifetime.
    async fn initialize(&self, request: SessionInitialization) -> Result<()>;
    /// The owned provider host may resolve an original private voice candidate.
    /// Default hosts do not consume this administrative operation.
    async fn resolve_incoming_voice(
        &self,
        _credential: String,
        _selection: connect_protocol::incoming_contacts::IncomingContactSelection,
    ) -> Option<Result<connect_protocol::incoming_contacts::IncomingContact>> {
        None
    }
    /// Permanently stop new admissions, including already queued notifications.
    fn cancel(&self);
    async fn drain(&self) -> Result<()>;
    /// A closed host may permit replacement after all transport/result custody
    /// has drained. Unresolved durable effects must be recovered by its successor.
    fn replacement_ready(&self) -> bool {
        false
    }
}
// Jobs may retain Runtime while Runtime owns their host registry. Break that
// ownership cycle when the final Service owner disappears, even without an
// explicit server drain. Cancellation requests cleanup; it is not cleanup proof.
impl Drop for crate::Service {
    fn drop(&mut self) {
        self.cancel_session_initialization();
        if let Some(runtime) = self.canonical_runtime.get() {
            runtime.stop.cancel();
        }
    }
}
const HOST_LIMIT: usize = 16;
#[derive(Default)]
pub(crate) struct Registry {
    closed: bool,
    hosts: std::collections::BTreeMap<String, Arc<dyn SessionInitializer>>,
}
impl crate::Service {
    pub async fn resolve_incoming_voice_contact(
        &self,
        credential: String,
        selection: connect_protocol::incoming_contacts::IncomingContactSelection,
    ) -> Result<connect_protocol::incoming_contacts::IncomingContact> {
        selection.validate()?;
        // Each host still validates the exact original candidate/administrator.
        // Copy registrations before awaiting; never retain the registry lock.
        let runtime = self.canonical_runtime.get().ok_or_else(Error::forbidden)?;
        let hosts = {
            let registry = runtime
                .session_initializers
                .lock()
                .map_err(|_| Error::storage())?;
            if registry.closed || runtime.stop.is_cancelled() {
                return Err(Error::forbidden());
            }
            registry.hosts.values().cloned().collect::<Vec<_>>()
        };
        for host in hosts {
            if let Some(result) = host
                .resolve_incoming_voice(credential.clone(), selection.clone())
                .await
            {
                return result;
            }
        }
        Err(Error::new(
            "provider-unavailable",
            "No owned incoming voice resolver is available",
        ))
    }
    /// Install a distinct protected host. The name is local configuration, not
    /// a wire route or permission. Every host independently filters its targets.
    pub fn install_session_initializer(
        &self,
        name: &str,
        initializer: Arc<dyn SessionInitializer>,
    ) -> Result<()> {
        connect_protocol::transitions::id(name)?;
        let runtime = self
            .canonical_runtime
            .get()
            .ok_or_else(|| Error::new("not-ready", "Canonical runtime is not installed"))?;
        let mut installed = runtime
            .session_initializers
            .lock()
            .map_err(|_| Error::storage())?;
        if runtime.stop.is_cancelled() || installed.closed {
            return Err(Error::forbidden());
        }
        if installed.hosts.contains_key(name) {
            return Err(Error::new("conflict", "Session host is already installed"));
        }
        if installed.hosts.len() >= HOST_LIMIT.min(self.store.limits().connections) {
            return Err(Error::new(
                "capacity-exceeded",
                "Session host capacity exceeded",
            ));
        }
        installed.hosts.insert(name.into(), initializer);
        Ok(())
    }
    /// Replace only a quiescent host. The successor must acquire its exclusive
    /// durable worker role and recover/enumerate original effects first.
    pub fn install_or_replace_session_initializer(
        &self,
        name: &str,
        initializer: Arc<dyn SessionInitializer>,
    ) -> Result<()> {
        connect_protocol::transitions::id(name)?;
        let runtime = self.canonical_runtime.get().ok_or_else(Error::forbidden)?;
        let mut registry = runtime
            .session_initializers
            .lock()
            .map_err(|_| Error::storage())?;
        if registry.closed || runtime.stop.is_cancelled() {
            return Err(Error::forbidden());
        }
        if let Some(old) = registry.hosts.get(name) {
            if !old.replacement_ready() {
                return Err(Error::new(
                    "conflict",
                    "Session host still owns active custody",
                ));
            }
        } else if registry.hosts.len() >= HOST_LIMIT.min(self.store.limits().connections) {
            return Err(Error::new(
                "capacity-exceeded",
                "Session host capacity exceeded",
            ));
        }
        registry.hosts.insert(name.into(), initializer);
        Ok(())
    }
    pub(crate) fn queue_session_initialization(
        &self,
        credential_id: String,
        request: Request,
        state: OperationState,
    ) {
        if state != OperationState::Completed {
            return;
        }
        let Some(runtime) = self.canonical_runtime.get() else {
            return;
        };
        let initializers = runtime.session_initializers.lock().map(|installed| {
            if installed.closed {
                vec![]
            } else {
                installed.hosts.values().cloned().collect::<Vec<_>>()
            }
        });
        let Ok(initializers) = initializers else {
            return;
        };
        if initializers.is_empty() || runtime.stop.is_cancelled() {
            return;
        }
        #[cfg(test)]
        queue_boundary();
        let Ok(mut work) = runtime.work.lock() else {
            return;
        };
        // A snapshot may have waited behind the work lock while shutdown closed
        // admission and took its JoinSet. Never add work after that boundary.
        if runtime.stop.is_cancelled()
            || runtime
                .session_initializers
                .lock()
                .map_or(true, |hosts| hosts.closed)
        {
            return;
        }
        while work.try_join_next().is_some() {}
        if work.len() >= self.store.limits().connections {
            return;
        }
        work.spawn(async move {
            let invocation = SessionInitialization { credential_id, request };
            // One bounded outer notification owns all child handles. A blocked
            // or failed host cannot prevent an independent host's admission.
            let mut notifications = tokio::task::JoinSet::new();
            for initializer in initializers {
                let invocation = invocation.clone();
                notifications.spawn(async move { initializer.initialize(invocation).await });
            }
            while let Some(result) = notifications.join_next().await {
                match result {
                    Ok(Ok(())) => (),
                    Ok(Err(error)) => tracing::debug!(target:"connect_service::session_init",code=%error.code,"Configured Session initialization stopped"),
                    Err(_) => tracing::debug!(target:"connect_service::session_init","Configured Session notification task failed"),
                }
            }
        });
    }
    fn close_session_initializers(&self) -> Vec<Arc<dyn SessionInitializer>> {
        self.canonical_runtime
            .get()
            .map(|runtime| {
                let mut installed = runtime
                    .session_initializers
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                // Serialize closure with registration. Retain every original host
                // for retrying uncertain cleanup, while refusing new ownership.
                installed.closed = true;
                installed.hosts.values().cloned().collect()
            })
            .unwrap_or_default()
    }
    pub fn cancel_session_initialization(&self) {
        for initializer in self.close_session_initializers() {
            initializer.cancel();
        }
    }
    pub async fn drain_session_initialization(&self) -> Result<()> {
        let initializers = self.close_session_initializers();
        for initializer in &initializers {
            initializer.cancel();
        }
        let mut draining = tokio::task::JoinSet::new();
        for initializer in initializers {
            draining.spawn(async move { initializer.drain().await });
        }
        let mut problem = None;
        while let Some(result) = draining.join_next().await {
            match result {
                Ok(Ok(())) => (),
                Ok(Err(error)) => {
                    if problem.is_none() {
                        problem = Some(error);
                    }
                }
                Err(_) => {
                    if problem.is_none() {
                        problem = Some(Error::storage());
                    }
                }
            }
        }
        problem.map_or(Ok(()), Err)
    }
}

#[cfg(test)]
thread_local! {
    static QUEUE_BOUNDARY: std::cell::RefCell<Option<(std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>)>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
fn queue_boundary() {
    if let Some((entered, release)) = QUEUE_BOUNDARY.with(|slot| slot.borrow_mut().take()) {
        entered.send(()).expect("queue boundary entered");
        release
            .recv_timeout(std::time::Duration::from_secs(3))
            .expect("queue boundary released");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use connect_protocol::session_control::Action;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    };
    struct Host {
        calls: Mutex<Vec<(String, String)>>,
        cancels: AtomicUsize,
        drains: AtomicUsize,
        fails: bool,
    }
    impl Host {
        fn new(fails: bool) -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(vec![]),
                cancels: AtomicUsize::new(0),
                drains: AtomicUsize::new(0),
                fails,
            })
        }
    }
    #[async_trait]
    impl SessionInitializer for Host {
        async fn initialize(&self, request: SessionInitialization) -> Result<()> {
            self.calls
                .lock()
                .expect("calls")
                .push((request.credential_id, request.request.operation_id));
            if self.fails {
                Err(Error::forbidden())
            } else {
                Ok(())
            }
        }
        fn cancel(&self) {
            self.cancels.fetch_add(1, Ordering::SeqCst);
        }
        async fn drain(&self) -> Result<()> {
            self.drains.fetch_add(1, Ordering::SeqCst);
            if self.fails {
                Err(Error::new(
                    "cleanup-unconfirmed",
                    "Fixture cleanup remains uncertain",
                ))
            } else {
                Ok(())
            }
        }
    }
    fn fixture(capacity: usize) -> (tempfile::TempDir, Arc<crate::Service>) {
        let dir = tempfile::tempdir().expect("fixture");
        let store = Arc::new(
            connect_store::Store::initialize(
                &dir.path().join("state.sqlite3"),
                connect_protocol::Limits {
                    connections: capacity,
                    ..Default::default()
                },
            )
            .expect("store"),
        );
        let service = crate::Service::new(store);
        service.install_canonical_runtime().expect("runtime");
        (dir, service)
    }
    fn request() -> Request {
        Request {
            operation_id: "ordinary-session-operation".into(),
            conversation_id: "room".into(),
            session_id: "session".into(),
            by_participant: None,
            expected_revision: Some(1),
            action: Action::Accept {
                capabilities: serde_json::json!({"audio":["g.711-mu"]}),
            },
        }
    }
    #[tokio::test]
    async fn dropping_final_service_cancels_host_job_retaining_runtime() {
        struct RetainingHost {
            runtime: std::sync::Weak<crate::canonical_runtime::Runtime>,
            stop: tokio_util::sync::CancellationToken,
            entered: Arc<tokio::sync::Notify>,
            jobs: Mutex<tokio::task::JoinSet<()>>,
        }
        #[async_trait]
        impl SessionInitializer for RetainingHost {
            async fn initialize(&self, _: SessionInitialization) -> Result<()> {
                let runtime = self.runtime.upgrade().ok_or_else(Error::forbidden)?;
                let stop = self.stop.clone();
                let entered = self.entered.clone();
                self.jobs.lock().unwrap().spawn(async move {
                    entered.notify_one();
                    stop.cancelled().await;
                    drop(runtime);
                });
                Ok(())
            }
            fn cancel(&self) {
                self.stop.cancel();
            }
            async fn drain(&self) -> Result<()> {
                self.cancel();
                let mut jobs =
                    std::mem::replace(&mut *self.jobs.lock().unwrap(), tokio::task::JoinSet::new());
                while let Some(result) = jobs.join_next().await {
                    result.map_err(|_| Error::storage())?;
                }
                Ok(())
            }
        }
        let (_dir, service) = fixture(4);
        let runtime = Arc::downgrade(service.canonical_runtime.get().unwrap());
        let stop = service.canonical_runtime.get().unwrap().stop.clone();
        let host = Arc::new(RetainingHost {
            runtime: runtime.clone(),
            stop: stop.child_token(),
            entered: Arc::new(tokio::sync::Notify::new()),
            jobs: Mutex::new(tokio::task::JoinSet::new()),
        });
        service
            .install_session_initializer("retaining-host", host.clone())
            .unwrap();
        host.initialize(SessionInitialization {
            credential_id: "original-credential".into(),
            request: request(),
        })
        .await
        .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), host.entered.notified())
            .await
            .unwrap();
        assert_eq!(
            runtime.strong_count(),
            2,
            "Service and running host own Runtime"
        );
        let another_owner = service.clone();
        drop(service);
        assert!(
            !stop.is_cancelled(),
            "a remaining Service owner keeps the host alive"
        );
        drop(another_owner);
        assert!(stop.is_cancelled());
        assert!(host.stop.is_cancelled());
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while runtime.strong_count() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("host job must release the Runtime ownership cycle");
        host.drain().await.unwrap();
    }
    #[tokio::test]
    async fn independent_hosts_receive_exact_committed_metadata_and_one_failure_does_not_skip_another(
    ) {
        let (_dir, service) = fixture(4);
        let a = Host::new(true);
        let b = Host::new(false);
        service
            .install_session_initializer("ai-provider", a.clone())
            .expect("first");
        service
            .install_session_initializer("telephone", b.clone())
            .expect("second");
        service.queue_session_initialization(
            "original-credential".into(),
            request(),
            OperationState::Accepted,
        );
        tokio::task::yield_now().await;
        assert!(a.calls.lock().expect("calls").is_empty());
        service.queue_session_initialization(
            "original-credential".into(),
            request(),
            OperationState::Completed,
        );
        let until = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        while a.calls.lock().expect("calls").is_empty() || b.calls.lock().expect("calls").is_empty()
        {
            assert!(
                tokio::time::Instant::now() < until,
                "notification did not reach both hosts"
            );
            tokio::task::yield_now().await;
        }
        let expected: Vec<(String, String)> = vec![(
            "original-credential".into(),
            "ordinary-session-operation".into(),
        )];
        assert_eq!(*a.calls.lock().expect("calls"), expected);
        assert_eq!(*b.calls.lock().expect("calls"), expected);
        service.cancel_session_initialization();
        assert_eq!(
            service
                .drain_session_initialization()
                .await
                .expect_err("one cleanup uncertain")
                .code,
            "cleanup-unconfirmed"
        );
        assert_eq!(a.drains.load(Ordering::SeqCst), 1);
        assert_eq!(b.drains.load(Ordering::SeqCst), 1);
        assert!(a.cancels.load(Ordering::SeqCst) > 0 && b.cancels.load(Ordering::SeqCst) > 0);
        service.drain_canonical_runtime().await;
    }
    #[tokio::test]
    async fn host_registry_refuses_duplicate_names_capacity_and_retired_runtime() {
        let (_dir, service) = fixture(2);
        let a = Host::new(false);
        service
            .install_session_initializer("first", a.clone())
            .expect("first");
        assert_eq!(
            service
                .install_session_initializer("first", a.clone())
                .expect_err("duplicate")
                .code,
            "conflict"
        );
        service
            .install_session_initializer("second", a.clone())
            .expect("second");
        assert_eq!(
            service
                .install_session_initializer("third", a.clone())
                .expect_err("capacity")
                .code,
            "capacity-exceeded"
        );
        service.cancel_session_initialization();
        assert_eq!(
            service
                .install_session_initializer("late", a.clone())
                .expect_err("closed hosts")
                .code,
            "forbidden"
        );
        service.queue_session_initialization(
            "original-credential".into(),
            request(),
            OperationState::Completed,
        );
        tokio::task::yield_now().await;
        assert!(a.calls.lock().expect("calls").is_empty());
        service
            .canonical_runtime
            .get()
            .expect("runtime")
            .stop
            .cancel();
        assert_eq!(
            service
                .install_session_initializer("third", a)
                .expect_err("retired")
                .code,
            "forbidden"
        );
        service.drain_canonical_runtime().await;
    }
    #[tokio::test]
    async fn queued_snapshot_cannot_add_work_after_host_closure() {
        let (_dir, service) = fixture(2);
        let host = Host::new(false);
        service
            .install_session_initializer("telephone", host.clone())
            .expect("host");
        let (entered, ready) = std::sync::mpsc::channel();
        let (release, waiting) = std::sync::mpsc::channel();
        let queued = service.clone();
        let handle = tokio::runtime::Handle::current();
        let worker = std::thread::spawn(move || {
            let _runtime = handle.enter();
            QUEUE_BOUNDARY.with(|slot| *slot.borrow_mut() = Some((entered, waiting)));
            queued.queue_session_initialization(
                "original-credential".into(),
                request(),
                OperationState::Completed,
            );
        });
        ready
            .recv_timeout(std::time::Duration::from_secs(3))
            .expect("snapshot captured");
        service.cancel_session_initialization();
        release.send(()).expect("release");
        worker.join().expect("queued caller");
        assert_eq!(
            service
                .canonical_runtime
                .get()
                .expect("runtime")
                .work
                .lock()
                .expect("work")
                .len(),
            0
        );
        assert!(host.calls.lock().expect("calls").is_empty());
        service.drain_canonical_runtime().await;
    }
}
