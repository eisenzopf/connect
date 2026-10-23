//! Protected host context for an exact registered native assistant incarnation.
use crate::{canonical_media::Connection, canonical_runtime::Runtime, Service};
use connect_protocol::{Error, Result};
use connect_store::{NativeAssistantContextRead, Store};
use rvoip_core::{ids::ConnectionId, ConnectionAdapter};
use std::sync::{Arc, Weak};

/// This bounded reader owns current Store authority and checks the actual
/// adapter/activation on every use. The host must bind its exact provider IO,
/// retain the read through each poll, and separately record observed delivery.
pub struct AssistantContextRead {
    inner: NativeAssistantContextRead,
    runtime: Weak<Runtime>,
    entry: Arc<Connection>,
}
impl std::fmt::Debug for AssistantContextRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AssistantContextRead(redacted)")
    }
}
impl AssistantContextRead {
    pub fn current(&self) -> bool {
        self.inner.current() && source_current(&self.runtime, &self.entry, self.inner.target())
    }

    pub fn expires_at(&self) -> i64 {
        self.inner.expires_at()
    }
    pub fn target(&self) -> &connect_store::PeerConnectionProjection {
        self.inner.target()
    }
    pub fn context_json(&self) -> Result<&str> {
        if !self.current() {
            return Err(Error::forbidden());
        }
        self.inner.context_json()
    }
    pub fn provider_endpoint(&self) -> Result<&serde_json::Value> {
        if !self.current() {
            return Err(Error::forbidden());
        }
        self.inner.provider_endpoint()
    }
    pub fn audience(&self) -> &[(String, String, u64)] {
        self.inner.audience()
    }
}
impl Service {
    /// Protected initial provider selection; holds the actual native source
    /// and current endpoint policy together before any provider I/O.
    pub fn try_routed_assistant_context_read(
        &self,
        context_credential: &str,
        connection_id: &str,
        generation: u64,
        provider_endpoint: &str,
        routing: &connect_store::DiscoveryRuntime,
        reachability: connect_protocol::routing::ReachabilityPolicy,
    ) -> Result<AssistantContextRead> {
        let runtime = self
            .canonical_runtime
            .get()
            .ok_or_else(|| Error::new("not-ready", "Canonical runtime is not installed"))?;
        let entry = runtime.media.assistant_source(connection_id, generation)?;
        let inner = self.store.try_routed_native_assistant_context_read(
            &entry.origin,
            context_credential,
            provider_endpoint,
            routing,
            reachability,
        )?;
        let read = AssistantContextRead {
            inner,
            runtime: Arc::downgrade(runtime),
            entry,
        };
        if !read.current() {
            return Err(Error::forbidden());
        }
        Ok(read)
    }
    /// Server composition only. Public Connection ID plus exact generation
    /// select a registered actual source; no Session invitee ordering is used.
    pub fn try_assistant_context_read(
        &self,
        context_credential: &str,
        connection_id: &str,
        generation: u64,
        provider_endpoint: &str,
    ) -> Result<AssistantContextRead> {
        let runtime = self
            .canonical_runtime
            .get()
            .ok_or_else(|| Error::new("not-ready", "Canonical runtime is not installed"))?;
        let entry = runtime.media.assistant_source(connection_id, generation)?;
        let inner = self.store.try_native_assistant_context_read(
            &entry.origin,
            context_credential,
            provider_endpoint,
        )?;
        let read = AssistantContextRead {
            inner,
            runtime: Arc::downgrade(runtime),
            entry,
        };
        if !read.current() {
            return Err(Error::forbidden());
        }
        Ok(read)
    }
}

fn source_current(
    runtime: &Weak<Runtime>,
    entry: &Arc<Connection>,
    target: &connect_store::PeerConnectionProjection,
) -> bool {
    let Some(runtime) = runtime.upgrade() else {
        return false;
    };
    if runtime.stop.is_cancelled()
        || !runtime
            .native
            .is_connection_live(&ConnectionId::from_string(
                entry.origin.runtime_connection_id().to_owned(),
            ))
    {
        return false;
    }
    entry
        .owner(target, &entry.principal, true)
        .and_then(|owner| {
            entry
                .activation()?
                .ok_or(rvoip_core::staged_streams::StreamControlError::NotReady)?
                .current(owner)
        })
        .is_ok()
}

/// Current authority for a captured provider creation/export or its original
/// opaque memory, including the actual registered adapter and activation.
pub struct AssistantProviderContextRead {
    pub(crate) inner: connect_store::AssistantProviderRead,
    pub(crate) runtime: Weak<Runtime>,
    pub(crate) entry: Arc<Connection>,
}
impl std::fmt::Debug for AssistantProviderContextRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AssistantProviderContextRead(redacted)")
    }
}
impl AssistantProviderContextRead {
    pub fn current(&self) -> bool {
        self.inner.current() && source_current(&self.runtime, &self.entry, self.inner.target())
    }
    pub fn expires_at(&self) -> i64 {
        self.inner.expires_at()
    }
    pub fn context_json(&self) -> Result<&str> {
        if !self.current() {
            return Err(Error::forbidden());
        }
        self.inner.context_json()
    }
    pub fn provider_endpoint(&self) -> Result<&serde_json::Value> {
        if !self.current() {
            return Err(Error::forbidden());
        }
        self.inner.provider_endpoint()
    }
    pub fn provider_ref(&self) -> Result<&str> {
        if !self.current() {
            return Err(Error::forbidden());
        }
        self.inner.provider_ref()
    }
}
impl Service {
    pub fn prepare_assistant_provider(
        &self,
        operation_id: &str,
        read: AssistantContextRead,
    ) -> Result<connect_store::AssistantProviderContext> {
        self.prepare_assistant_provider_timed(
            operation_id,
            read,
            connect_store::AssistantProviderTiming::AfterCreation,
        )
    }
    pub fn prepare_assistant_provider_timed(
        &self,
        operation_id: &str,
        read: AssistantContextRead,
        timing: connect_store::AssistantProviderTiming,
    ) -> Result<connect_store::AssistantProviderContext> {
        if !read.current() {
            return Err(Error::forbidden());
        }
        self.store
            .prepare_native_assistant_provider_timed(operation_id, read.inner, timing)
    }
    pub fn try_assistant_provider_read(
        &self,
        context: &connect_store::AssistantProviderContext,
    ) -> Result<AssistantProviderContextRead> {
        self.assistant_provider_read(context, None)
    }
    pub fn try_assistant_memory_read(
        &self,
        context: &connect_store::AssistantProviderContext,
        receipt: &connect_store::AssistantProviderApplied,
    ) -> Result<AssistantProviderContextRead> {
        self.assistant_provider_read(context, Some(receipt))
    }
    fn assistant_provider_read(
        &self,
        context: &connect_store::AssistantProviderContext,
        memory: Option<&connect_store::AssistantProviderApplied>,
    ) -> Result<AssistantProviderContextRead> {
        self.assistant_read_using(context, |store| match memory {
            Some(receipt) => store.try_native_assistant_memory_read(context, receipt),
            None => store.try_native_assistant_provider_read(context),
        })
    }
    fn assistant_read_using(
        &self,
        context: &connect_store::AssistantProviderContext,
        load: impl FnOnce(&Store) -> Result<connect_store::AssistantProviderRead>,
    ) -> Result<AssistantProviderContextRead> {
        let runtime = self
            .canonical_runtime
            .get()
            .ok_or_else(|| Error::new("not-ready", "Canonical runtime is not installed"))?;
        let entry = runtime
            .media
            .assistant_source(context.connection_id(), context.generation())?;
        let inner = load(&self.store)?;
        let read = AssistantProviderContextRead {
            inner,
            runtime: Arc::downgrade(runtime),
            entry,
        };
        if !read.current() {
            return Err(Error::forbidden());
        }
        Ok(read)
    }
    pub fn begin_assistant_provider(
        &self,
        context: connect_store::AssistantProviderContext,
        read: AssistantProviderContextRead,
    ) -> Result<(
        connect_store::AssistantProviderAttempt,
        AssistantProviderContextRead,
    )> {
        if !read.current() {
            return Err(Error::forbidden());
        }
        let (attempt, inner) = self
            .store
            .begin_native_assistant_provider(context, read.inner)?;
        Ok((
            attempt,
            AssistantProviderContextRead {
                inner,
                runtime: read.runtime,
                entry: read.entry,
            },
        ))
    }
    pub fn begin_assistant_export(
        &self,
        context: connect_store::AssistantProviderContext,
        read: AssistantProviderContextRead,
    ) -> Result<(
        connect_store::AssistantProviderExport,
        AssistantProviderContextRead,
    )> {
        if !read.current() {
            return Err(Error::forbidden());
        }
        let (export, inner) = self
            .store
            .begin_native_assistant_export(context, read.inner)?;
        Ok((
            export,
            AssistantProviderContextRead {
                inner,
                runtime: read.runtime,
                entry: read.entry,
            },
        ))
    }
    pub fn begin_assistant_initialization(
        &self,
        context: connect_store::AssistantProviderContext,
        read: AssistantProviderContextRead,
    ) -> Result<(
        connect_store::AssistantProviderInitialization,
        AssistantProviderContextRead,
    )> {
        if !read.current() {
            return Err(Error::forbidden());
        }
        let (attempt, inner) = self
            .store
            .begin_native_assistant_initialization(context, read.inner)?;
        Ok((
            attempt,
            AssistantProviderContextRead {
                inner,
                runtime: read.runtime,
                entry: read.entry,
            },
        ))
    }
    pub fn try_assistant_initialization_read(
        &self,
        context: &connect_store::AssistantProviderContext,
    ) -> Result<AssistantProviderContextRead> {
        self.assistant_read_using(context, |s| {
            s.try_native_assistant_initialization_read(context)
        })
    }
    pub fn prepare_assistant_update(
        &self,
        operation_id: &str,
        context: &connect_store::AssistantProviderContext,
        applied: &connect_store::AssistantProviderApplied,
        read: AssistantProviderContextRead,
    ) -> Result<connect_store::AssistantProviderUpdate> {
        if !read.current() {
            return Err(Error::forbidden());
        }
        self.store
            .prepare_native_assistant_update(operation_id, context, applied, read.inner)
    }
    pub fn try_assistant_update_read(
        &self,
        update: &connect_store::AssistantProviderUpdate,
        applied: &connect_store::AssistantProviderApplied,
    ) -> Result<AssistantProviderContextRead> {
        self.assistant_read_using(update.instance(), |s| {
            s.try_native_assistant_update_read(update, applied)
        })
    }
    pub fn begin_assistant_update(
        &self,
        update: &connect_store::AssistantProviderUpdate,
        read: AssistantProviderContextRead,
    ) -> Result<(
        connect_store::AssistantProviderUpdateExport,
        AssistantProviderContextRead,
    )> {
        if !read.current() {
            return Err(Error::forbidden());
        }
        let (export, inner) = self
            .store
            .begin_native_assistant_update(update, read.inner)?;
        Ok((
            export,
            AssistantProviderContextRead {
                inner,
                runtime: read.runtime,
                entry: read.entry,
            },
        ))
    }
}
