//! Protected provider event ownership. Activity checks never reenter Store.
use crate::{
    assistant_context::AssistantProviderContextRead, canonical_media::Connection,
    canonical_runtime::Runtime, Service,
};
use connect_protocol::{
    events::{ObservationCapabilities, ObservationDraft},
    Error, Result,
};
use connect_store::{
    AssistantProviderApplied, AssistantProviderContext, PrivateProviderApplied,
    PrivateProviderContext, PrivateProviderMediaRead, SessionObservationActivity,
    SessionObservationSource,
};
use rvoip_core::ids::ConnectionId;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Weak,
};

struct Activity {
    closed: AtomicBool,
    runtime: Weak<Runtime>,
    entry: Arc<Connection>,
}
impl SessionObservationActivity for Activity {
    fn check(&self) -> Result<()> {
        let runtime = self.runtime.upgrade().ok_or_else(Error::forbidden)?;
        if self.closed.load(Ordering::Acquire) || runtime.stop.is_cancelled() {
            return Err(Error::forbidden());
        }
        let current = runtime
            .media
            .assistant_source(
                self.entry.origin.connection_id(),
                self.entry.origin.generation(),
            )
            .map_err(|error| {
                if error.code == "not-ready" {
                    Error::forbidden()
                } else {
                    error
                }
            })?;
        if !Arc::ptr_eq(&current, &self.entry) {
            return Err(Error::forbidden());
        }
        // The adapter's bool also covers internal contention. Do not convert an
        // ambiguous check into permanent removal/cursor advancement. The owning
        // job explicitly closes before asynchronous provider cleanup.
        if !runtime.native_connection_live(&ConnectionId::from_string(
            self.entry.origin.runtime_connection_id().to_owned(),
        )) {
            return Err(Error::new(
                "authority-busy",
                "Provider native source is unavailable",
            ));
        }
        Ok(())
    }
}
struct Owner(Arc<Activity>);
impl Drop for Owner {
    fn drop(&mut self) {
        self.0.closed.store(true, Ordering::Release);
    }
}
/// Retain in the provider job. Closing/dropping synchronously removes sensitive
/// buffered content from subsequent reads without depending on cleanup SQL.
pub struct ProviderObservationSource {
    source: SessionObservationSource,
    owner: Owner,
}
impl std::fmt::Debug for ProviderObservationSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProviderObservationSource(redacted)")
    }
}
impl ProviderObservationSource {
    pub fn close(&self) {
        self.owner.0.closed.store(true, Ordering::Release);
    }
}
impl Service {
    pub fn bind_assistant_observation_source(
        &self,
        publisher: &str,
        context: &AssistantProviderContext,
        applied: Arc<AssistantProviderApplied>,
        capabilities: ObservationCapabilities,
    ) -> Result<ProviderObservationSource> {
        self.bind_assistant_observation_source_with_work(
            publisher,
            context,
            applied,
            capabilities,
            None,
        )
    }
    pub fn bind_assistant_observation_source_with_work(
        &self,
        publisher: &str,
        context: &AssistantProviderContext,
        applied: Arc<AssistantProviderApplied>,
        capabilities: ObservationCapabilities,
        work: Option<connect_store::TaskObservationBinding>,
    ) -> Result<ProviderObservationSource> {
        let read = self.try_assistant_memory_read(context, &applied)?;
        let owner = Owner(Arc::new(Activity {
            closed: AtomicBool::new(false),
            runtime: read.runtime.clone(),
            entry: read.entry.clone(),
        }));
        let source = self
            .store
            .bind_native_assistant_observation_source_with_work(
                publisher,
                context,
                applied,
                capabilities,
                owner.0.clone(),
                work,
            )?;
        Ok(ProviderObservationSource { source, owner })
    }
    pub fn bind_private_observation_source(
        &self,
        publisher: &str,
        context: &PrivateProviderContext,
        applied: Arc<PrivateProviderApplied>,
        capabilities: ObservationCapabilities,
    ) -> Result<ProviderObservationSource> {
        self.bind_private_observation_source_with_work(
            publisher,
            context,
            applied,
            capabilities,
            None,
        )
    }
    pub fn bind_private_observation_source_with_work(
        &self,
        publisher: &str,
        context: &PrivateProviderContext,
        applied: Arc<PrivateProviderApplied>,
        capabilities: ObservationCapabilities,
        work: Option<connect_store::TaskObservationBinding>,
    ) -> Result<ProviderObservationSource> {
        let read = self
            .store
            .try_private_provider_media_read(context, &applied)?;
        let runtime = self
            .canonical_runtime
            .get()
            .ok_or_else(|| Error::new("not-ready", "Canonical runtime is not installed"))?;
        let entry = runtime
            .media
            .assistant_source(read.connection_id(), read.generation())?;
        let owner = Owner(Arc::new(Activity {
            closed: AtomicBool::new(false),
            runtime: Arc::downgrade(runtime),
            entry,
        }));
        owner.0.check()?;
        let source = self
            .store
            .bind_private_provider_observation_source_with_work(
                publisher,
                context,
                applied,
                capabilities,
                owner.0.clone(),
                work,
            )?;
        Ok(ProviderObservationSource { source, owner })
    }
    pub fn publish_assistant_observation(
        &self,
        source: &ProviderObservationSource,
        read: &AssistantProviderContextRead,
        event: &str,
        data: ObservationDraft,
    ) -> Result<String> {
        source.owner.0.check()?;
        if !read.current() {
            return Err(Error::forbidden());
        }
        let id = self.store.publish_native_assistant_observation(
            &source.source,
            &read.inner,
            event,
            data,
        )?;
        self.notify_changed();
        Ok(id)
    }
    pub fn publish_private_observation(
        &self,
        source: &ProviderObservationSource,
        read: &PrivateProviderMediaRead,
        event: &str,
        data: ObservationDraft,
    ) -> Result<String> {
        source.owner.0.check()?;
        let id =
            self.store
                .publish_private_provider_observation(&source.source, read, event, data)?;
        self.notify_changed();
        Ok(id)
    }
}
