//! Protected host composition for the canonical Core's telephone replacement adapter.
//! Configuration and retained attempts own metadata; physical IO obtains a fresh read.
pub(crate) mod allocation;
pub(crate) mod authority;
pub(crate) mod continuation;
pub(crate) mod control;
mod established;
pub(crate) mod media;
pub(crate) mod movement;
mod preparation;
mod write;
use crate::{canonical_runtime::Runtime, Service};
use connect_protocol::{Error, Result};
use connect_store::TelephoneReplacementPolicy;
use rvoip_core::adapter::ConnectionAdapter;
use rvoip_sip::{SipAdapter, SipOriginateContext};
use std::{collections::BTreeMap, sync::Arc};

/// Administrator-selected account and bounded callback policy. The route key is
/// a Store credential ID, never a caller-supplied bearer or a destination.
#[derive(Clone)]
pub struct TelephoneReplacementRoute {
    pub credential_id: String,
    pub policy: TelephoneReplacementPolicy,
    pub originate: SipOriginateContext,
}
impl std::fmt::Debug for TelephoneReplacementRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TelephoneReplacementRoute(redacted)")
    }
}
pub(crate) struct TelephoneRuntime {
    pub adapter: Arc<SipAdapter>,
    pub routes: BTreeMap<String, Arc<TelephoneReplacementRoute>>,
}
impl Service {
    /// Install the supplied SIP adapter on the SAME canonical Core before intake.
    /// The server owns coordinator/network configuration and shutdown. This hook
    /// grants no operation, allocation or write. Each subsequent replacement
    /// needs independently current operator/source/route authority. Submission
    /// never implies endpoint confirmation or a committed speaking route.
    pub fn install_telephone_replacement_adapter(
        &self,
        adapter: Arc<SipAdapter>,
        routes: Vec<TelephoneReplacementRoute>,
    ) -> Result<()> {
        let runtime = self
            .canonical_runtime
            .get()
            .ok_or_else(|| Error::new("not-ready", "Canonical runtime is not installed"))?;
        let closed = runtime
            .host_configuration_closed
            .lock()
            .map_err(|_| Error::storage())?;
        if *closed {
            return Err(Error::forbidden());
        }
        let mut installed = runtime.telephone.lock().map_err(|_| Error::storage())?;
        if installed.is_some() {
            return Err(Error::new(
                "conflict",
                "Telephone adapter is already installed",
            ));
        }
        if runtime.stop.is_cancelled() {
            return Err(Error::forbidden());
        }
        if routes.is_empty() || routes.len() > self.store.limits().connections {
            return Err(Error::new(
                "invalid-request",
                "Invalid telephone profile capacity",
            ));
        }
        let mut configured = BTreeMap::new();
        for route in routes {
            route.policy.validate()?;
            route
                .originate
                .validate()
                .map_err(|_| Error::new("invalid-request", "Invalid telephone originate policy"))?;
            let actor = self.store.revalidate(&route.credential_id)?;
            actor.require("sip_dial")?;
            if configured
                .insert(route.policy.route_reference.clone(), Arc::new(route))
                .is_some()
            {
                return Err(Error::new(
                    "invalid-request",
                    "Duplicate telephone route reference",
                ));
            }
        }
        if !adapter.supports_device_replacement_preparation()
            || !adapter.lifecycle_capabilities().authoritative_liveness
            || !adapter.lifecycle_capabilities().staged_outbound_activation
        {
            return Err(Error::new(
                "unsupported-media-profile",
                "SIP adapter lacks replacement custody",
            ));
        }
        runtime
            .core
            .register(adapter.clone())
            .map_err(|_| Error::new("conflict", "Canonical SIP adapter registration failed"))?;
        *installed = Some(Arc::new(TelephoneRuntime {
            adapter,
            routes: configured,
        }));
        Ok(())
    }
}
impl Runtime {
    pub(crate) fn telephone_route(
        &self,
        reference: &str,
    ) -> Result<(Arc<SipAdapter>, Arc<TelephoneReplacementRoute>)> {
        let installed = self.telephone.lock().map_err(|_| Error::storage())?;
        let installed = installed.as_ref().ok_or_else(|| {
            Error::new(
                "unsupported-media-profile",
                "Telephone replacement is not configured",
            )
        })?;
        let route = installed
            .routes
            .get(reference)
            .cloned()
            .ok_or_else(Error::forbidden)?;
        Ok((installed.adapter.clone(), route))
    }
}
