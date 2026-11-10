//! One original subscription attempt across resource-only preparation and activation.
use super::*;
use crate::canonical_media::streams::StreamInvocation;
use rvoip_uctp::v2::native_subscriptions::NativeSubscriptionPlan;

impl Runtime {
    pub(super) async fn subscription_resource_worker(
        self: Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        physical: Arc<Physical>,
        credential: String,
        id: ConnectionId,
        plan: NativeSubscriptionPlan,
        invocation: Arc<StreamInvocation>,
        lease: connect_store::NativeStreamLease,
        started: connect_protocol::transitions::OperationRecord,
        deadline: Instant,
        tx: tokio::sync::oneshot::Sender<Result<crate::canonical_native::Reply>>,
    ) {
        let work = async {
            let native = self.native_adapter(physical.binding.transport())?;
            let staged = tokio::select! { biased;
                _ = self.stop.cancelled() => return Err(Error::forbidden()),
                result = native.stage_native_subscription_resources(
                    &self.core, &id, &a, invocation.clone(), plan.clone(), deadline,
                ) => result.map_err(resource_error)?,
            };
            drop(invocation);
            let original = Arc::try_unwrap(a).map_err(|_| {
                Error::new(
                    "conflict",
                    "Original subscription planning authority remains retained",
                )
            })?;
            // This consumes the exact original command, drops its shared reply
            // reader and performs only its bounded parameter/codec exchange.
            let prepared = tokio::select! { biased;
                _ = self.stop.cancelled() => return Err(Error::forbidden()),
                result = staged.prepare(original) => result.map_err(resource_error)?,
            };
            let resource = prepared.authorized();
            // Join this original bounded policy acquisition even if its waiter
            // deadline/cancellation expires. Dropping it could detach the host's
            // blocking admission. It may never activate after the original cap.
            let resumed = resource
                .reauthorize_resource_preparation()
                .await
                .map_err(|_| Error::forbidden())?;
            let authority = resumed
                .authority::<StoreInvocationAuthority>()
                .ok_or_else(Error::forbidden)?;
            physical.validate(&resource, authority)?;
            if physical.lifetime.is_cancelled()
                || self.stop.is_cancelled()
                || Instant::now() >= deadline
            {
                return Err(Error::forbidden());
            }
            let bound = native
                .physical_source(&id)
                .map_err(|_| Error::forbidden())?;
            if !Arc::ptr_eq(&bound, &physical.binding) {
                return Err(Error::forbidden());
            }
            let fresh = self.media.stream_invocation_with_activation(
                resource.clone(),
                Plan::Subscriptions(plan),
                &lease,
                deadline,
                Some(resumed.clone()),
            )?;
            // Same original selected resources and routes; fresh permission and
            // expected revision are checked at the existing atomic Core commit.
            let outcome = tokio::select! { biased;
                _ = self.stop.cancelled() => return Err(Error::forbidden()),
                result = prepared.activate_with_held(fresh.clone()) => result.map_err(resource_error)?,
            };
            fresh.remember_confirmed_selection();
            // Do not retain activation authority in the outer reply slot while
            // the original historical outcome needs its short metadata writer.
            drop(fresh);
            drop(resumed);
            Ok((outcome, resource))
        };
        let result = work.await;
        let (captured, resource, original_error) = match result {
            Ok((outcome, resource)) => match lease.committed(
                outcome.revision,
                outcome.resources_changed,
                captured_projection(&self, &id, outcome.revision),
            ) {
                Ok(receipt) => (
                    CapturedStreamOutcome::Committed(receipt),
                    Some(resource),
                    None,
                ),
                Err(error) => {
                    let _ = tx.send(Err(error));
                    return;
                }
            },
            Err(error) => {
                tracing::debug!(target: "connect_service::native_stream_preparation", code=%error.code,
                    "Original resource subscription retained uncertain outcome");
                (CapturedStreamOutcome::Uncertain(lease), None, Some(error))
            }
        };
        // No activation reader is published into reply custody yet. Record the
        // original outcome first; restoring the reply reader before this point
        // would contend with our own Conversation writer.
        drop(physical);
        drop(credential);
        let (record, unsettled) = match captured.record(&self.media.store) {
            Ok(record) => (record, false),
            Err(_) => (started, true),
        };
        let response = if let Some(error) = original_error {
            Err(error)
        } else if let Some(resource) = resource {
            // Current permission to disclose this original result is separate
            // from its completed effect. Reauthorize the SAME signed command,
            // never the adapter/operation, and retain its original cutoff.
            let current = resource.reauthorize_resource_preparation().await;
            match current
                .and_then(|authority| resource.install_resource_reply_authority(&authority))
            {
                Ok(()) => Ok(crate::canonical_native::Reply {
                    record,
                    answered: false,
                }),
                Err(_) => Err(Error::forbidden()),
            }
        } else {
            Err(Error::forbidden())
        };
        let _ = tx.send(response);
        if unsettled {
            // Only an independently busy metadata writer reaches this path on
            // success. The ordinary response releases its fresh reader, while
            // this SAME worker retains and settles the original captured receipt.
            captured.settle(&self.media.store, &self.stop).await;
        }
    }
}
fn resource_error(error: rvoip_uctp::v2::native_negotiation::NativeOfferError) -> Error {
    tracing::debug!(target: "connect_service::native_stream_preparation", ?error,
        "Original subscription resource or activation stage refused");
    Error::new(
        "reconciliation-required",
        "Original subscription resource outcome requires reconciliation",
    )
}
