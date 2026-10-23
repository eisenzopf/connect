//! Actual dormant reservation, pre-recorded Call-ID and once-only SIP submission.
use super::*;
use crate::{
    canonical_consultation::{retry_metadata, store_call},
    canonical_replacement::{observed, PreparedReplacement},
};
use connect_store::{
    TelephoneReplacementAllocated, TelephoneReplacementAttempt, TelephoneReplacementSubmitted,
    TelephoneReplacementUncertain,
};
use rvoip_sip::{SipDeviceReplacementContext, SipDeviceReplacementSignal};
use std::sync::{Mutex, OnceLock};

pub(crate) struct Allocated {
    pub context: Arc<SipDeviceReplacementContext>,
    pub receipt: Arc<TelephoneReplacementAllocated>,
    pub route: Arc<TelephoneReplacementRoute>,
    pub target: Arc<authority::Target>,
    pub writer: Arc<write::InviteAuthority>,
    pub signals: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<SipDeviceReplacementSignal>>,
    pub submission: OnceLock<Arc<TelephoneReplacementAttempt>>,
    pub submitted: Mutex<Option<Arc<TelephoneReplacementSubmitted>>>,
    pub uncertain: Mutex<Option<Arc<TelephoneReplacementUncertain>>>,
    pub resources: Mutex<Option<Arc<super::media::Started>>>,
    pub resource_failure: Mutex<Option<Arc<connect_store::TelephoneReplacementResourcesCapture>>>,
}
impl Runtime {
    pub(crate) async fn allocate_telephone_replacement(
        self: &Arc<Self>,
        root: &Arc<PreparedReplacement>,
        route: Arc<TelephoneReplacementRoute>,
    ) -> Result<Arc<Allocated>> {
        let _roles = self.role_projection.lock().await;
        let previous = { root.telephone.lock().map_err(|_| Error::storage())?.clone() };
        if let Some(previous) = previous {
            self.record_telephone_allocation(&previous).await?;
            return Ok(previous);
        }
        let (adapter, configured) = self.telephone_route(&route.policy.route_reference)?;
        if !Arc::ptr_eq(&route, &configured) {
            return Err(Error::forbidden());
        }
        let read = retry_metadata(&self.stop, || async {
            self.media.store.telephone_replacement_allocation_read(
                &root.receipt,
                &route.credential_id,
                &route.policy,
            )
        })
        .await?;
        let original = self
            .media
            .replacement_source_owner(read.source())
            .map_err(|_| Error::forbidden())?;
        let target = Arc::new(authority::allocated_target(
            self,
            &read,
            &route,
            &original.member,
        )?);
        let writer = Arc::new(write::InviteAuthority {
            runtime: Arc::downgrade(self),
            attempt: OnceLock::new(),
            established: OnceLock::new(),
            initial_from: OnceLock::new(),
            cancelled: root.handle.cancellation(),
            deadline: root.deadline,
        });
        let context = adapter
            .device_replacement_context(
                &self.core,
                &root.handle,
                target.principal.clone(),
                target.endpoint.clone(),
                route
                    .originate
                    .clone()
                    .with_request_write_authority(writer.clone()),
            )
            .map_err(|_| Error::forbidden())?;
        let signals = context.take_signals().map_err(|_| Error::forbidden())?;
        let store = self.media.store.clone();
        let (attempt, read) =
            store_call(move || store.begin_telephone_replacement_allocation(read)).await?;
        let invocation = control::Invocation {
            runtime: self.clone(),
            root: root.clone(),
            route: route.clone(),
            target: target.clone(),
            phase: control::Phase::Allocate(Arc::new(read)),
        };
        let result = self.core.prepare_adapter_device_replacement_with_held(
            &root.handle,
            context.clone(),
            &invocation,
        );
        drop(invocation);
        let capture = result.map_err(|_| Error::forbidden()).and_then(|actual| {
            if actual != root.handle.intent().replacement {
                return Err(Error::forbidden());
            }
            let mut observation = observed(&root.handle, &root.plan)?;
            observation["allocated"] = serde_json::json!(true);
            observation["transport"] = serde_json::json!("sip");
            let reference = context
                .prepared_call_reference()
                .map_err(|_| Error::forbidden())?;
            observation["call_reference"] =
                serde_json::json!({"kind":reference.kind(),"value":reference.expose_secret()});
            attempt.allocated(observation)
        });
        let receipt = match capture {
            Ok(receipt) => Arc::new(receipt),
            Err(error) => {
                root.handle.cancel();
                let unknown = attempt.uncertain();
                let store = self.media.store.clone();
                store_call(move || store.record_telephone_replacement_uncertain(&unknown)).await?;
                return Err(error);
            }
        };
        let allocated = Arc::new(Allocated {
            context,
            receipt,
            route,
            target,
            writer,
            signals: tokio::sync::Mutex::new(signals),
            submission: OnceLock::new(),
            submitted: Mutex::new(None),
            uncertain: Mutex::new(None),
            resources: Mutex::new(None),
            resource_failure: Mutex::new(None),
        });
        *root.telephone.lock().map_err(|_| Error::storage())? = Some(allocated.clone());
        self.record_telephone_allocation(&allocated).await?;
        Ok(allocated)
    }
    async fn record_telephone_allocation(&self, leg: &Allocated) -> Result<()> {
        retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let receipt = leg.receipt.clone();
            store_call(move || store.record_telephone_replacement_allocated(&receipt))
        })
        .await?;
        Ok(())
    }
    pub(crate) async fn submit_telephone_replacement(
        self: &Arc<Self>,
        root: &Arc<PreparedReplacement>,
        leg: &Arc<Allocated>,
    ) -> Result<()> {
        // No role mutex or current-authority reader survives ringing.
        let read = retry_metadata(&root.handle.cancellation(), || async {
            self.media
                .store
                .telephone_replacement_submission_read(leg.receipt.clone())
        })
        .await?;
        let store = self.media.store.clone();
        let (attempt, read) =
            store_call(move || store.begin_telephone_replacement_submission(read)).await?;
        drop(read);
        leg.submission
            .set(attempt.clone())
            .map_err(|_| Error::forbidden())?;
        leg.writer
            .attempt
            .set(attempt.clone())
            .map_err(|_| Error::forbidden())?;
        let invocation = control::Invocation {
            runtime: self.clone(),
            root: root.clone(),
            route: leg.route.clone(),
            target: leg.target.clone(),
            phase: control::Phase::Start(attempt.clone()),
        };
        let result = self
            .core
            .start_adapter_device_replacement_with_held(&root.handle, &invocation)
            .await;
        drop(invocation);
        match result {
            Ok(activation) => {
                let references = activation.external_references();
                if references.len() != 1
                    || references[0].kind() != "sip.call-id"
                    || references[0].expose_secret() != leg.receipt.call_id()
                {
                    return self
                        .uncertain_telephone_submission(root, leg, &attempt)
                        .await;
                }
                let capture = Arc::new(attempt.submitted(serde_json::json!({"submitted":true,"call_reference":{"kind":references[0].kind(),"value":references[0].expose_secret()}}))?);
                *leg.submitted.lock().map_err(|_| Error::storage())? = Some(capture.clone());
                retry_metadata(&self.stop, || {
                    let store = self.media.store.clone();
                    let capture = capture.clone();
                    store_call(move || store.record_telephone_replacement_submitted(&capture))
                })
                .await?;
                let result = self.prepare_telephone_resources(root, leg, capture).await;
                if result.is_err() {
                    root.handle.cancel();
                    let _ = self.abandon_telephone_submission(leg).await;
                }
                result
            }
            Err(error) => {
                tracing::debug!(target:"connect_service::telephone_replacement", ?error, "Actual Core telephone start did not complete");
                self.uncertain_telephone_submission(root, leg, &attempt)
                    .await
            }
        }
    }
    pub(crate) async fn abandon_telephone_submission(&self, leg: &Allocated) -> Result<()> {
        let submitted = { leg.submitted.lock().map_err(|_| Error::storage())?.clone() };
        if let Some(submitted) = submitted {
            retry_metadata(&self.stop, || {
                let store = self.media.store.clone();
                let submitted = submitted.clone();
                store_call(move || store.abandon_telephone_replacement(&submitted))
            })
            .await?;
        }
        Ok(())
    }
    async fn uncertain_telephone_submission(
        &self,
        root: &PreparedReplacement,
        leg: &Allocated,
        attempt: &Arc<TelephoneReplacementAttempt>,
    ) -> Result<()> {
        root.handle.cancel();
        let capture = Arc::new(attempt.uncertain());
        *leg.uncertain.lock().map_err(|_| Error::storage())? = Some(capture.clone());
        retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let capture = capture.clone();
            store_call(move || store.record_telephone_replacement_uncertain(&capture))
        })
        .await?;
        Err(Error::new(
            "reconciliation-required",
            "Telephone submission requires reconciliation",
        ))
    }
}
