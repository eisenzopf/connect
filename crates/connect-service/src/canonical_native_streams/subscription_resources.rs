//! One original subscription attempt across resource-only preparation and activation.
use super::*;
use crate::canonical_media::streams::StreamInvocation;
use rvoip_uctp::v2::native_subscriptions::NativeSubscriptionPlan;
mod current_scene;

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
        let physical_cancel = physical.lifetime.child_token();
        #[cfg(feature = "test-support")]
        let admission_probe = self
            .media
            .take_preparation_registry_probe_for_test(&format!(
                "subscription-admission:{}",
                a.envelope().scope(Scope::Operation).unwrap_or("")
            ));
        #[cfg(feature = "test-support")]
        let scene_probe = self
            .media
            .take_preparation_registry_probe_for_test(&format!(
                "subscription-scene:{}",
                a.envelope().scope(Scope::Operation).unwrap_or("")
            ));
        let scene_busy_observed = std::sync::atomic::AtomicBool::new(false);
        let mut phase = "stage-resources";
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
            phase = "prepare-parameters";
            let prepared = tokio::select! { biased;
                _ = self.stop.cancelled() => return Err(Error::forbidden()),
                result = staged.prepare(original) => result.map_err(resource_error)?,
            };
            let resource = prepared.authorized();
            #[cfg(feature = "test-support")]
            if let Some(probe) = &admission_probe {
                // Same original worker, after parameter preparation released
                // every domain alias. The fixture may now hold a REAL writer;
                // no permit or error is fabricated by this rendezvous.
                probe.store(1, std::sync::atomic::Ordering::Release);
                let original = resource.invocation_cancellation();
                while probe.load(std::sync::atomic::Ordering::Acquire) == 1 {
                    tokio::select! { biased;
                        _ = self.stop.cancelled() => return Err(Error::forbidden()),
                        _ = physical_cancel.cancelled() => return Err(Error::forbidden()),
                        _ = original.cancelled() => return Err(Error::forbidden()),
                        _ = tokio::time::sleep_until(deadline) => return Err(Error::forbidden()),
                        _ = tokio::time::sleep(Duration::from_millis(1)) => {},
                    }
                }
                if probe.load(std::sync::atomic::Ordering::Acquire) != 2 {
                    return Err(Error::forbidden());
                }
            }
            // Join this original bounded policy acquisition even if its waiter
            // deadline/cancellation expires. Dropping it could detach the host's
            // blocking admission. It may never activate after the original cap.
            phase = "authorize-activation";
            let original = resource.invocation_cancellation();
            let fresh=current_scene::capture(deadline,[&self.stop,&physical_cancel,&original],|| {
            // Type erasure only: this SAME attempt future is awaited and joined
            // by capture; no task, cancellation select or custody is introduced.
            let attempt: std::pin::Pin<Box<dyn std::future::Future<Output = Result<Arc<StreamInvocation>>> + Send + '_>> = Box::pin(async {
            let resumed = resource
                .reauthorize_subscription_resource_preparation_observing(
                    &self.stop,
                    &physical_cancel,
                    deadline,
                    || {
                        #[cfg(feature = "test-support")]
                        if let Some(probe) = &admission_probe {
                            probe.store(3, std::sync::atomic::Ordering::Release);
                        }
                    },
                )
                .await
                .map_err(|_| Error::forbidden())?;
            #[cfg(feature = "test-support")]
            if let Some(probe) = &admission_probe {
                probe.store(4, std::sync::atomic::Ordering::Release);
            }
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
            #[cfg(feature = "test-support")]
            let scope_custody = if scene_probe.is_some() {
                Some(authority.peer_guard().ok_or_else(Error::forbidden)?.retained_custody_probe_for_test())
            } else {None};
            #[cfg(feature = "test-support")]
            if let Some(probe)=&scene_probe {
                if probe.load(std::sync::atomic::Ordering::Acquire)==0 {
                    if !scope_custody.as_ref().is_some_and(|held|held()) {return Err(Error::new("test-custody-unobserved","Original scene command scope set was not live"));}
                    probe.store(1,std::sync::atomic::Ordering::Release);
                    while probe.load(std::sync::atomic::Ordering::Acquire)==1 {
                        tokio::select! {biased;
                            _=self.stop.cancelled()=>return Err(Error::forbidden()),
                            _=physical_cancel.cancelled()=>return Err(Error::forbidden()),
                            _=original.cancelled()=>return Err(Error::forbidden()),
                            _=tokio::time::sleep_until(deadline)=>return Err(Error::forbidden()),
                            _=tokio::time::sleep(Duration::from_millis(1))=>{},
                        }
                    }
                    if probe.load(std::sync::atomic::Ordering::Acquire)!=2 {return Err(Error::forbidden());}
                }
            }
            let fresh = self.media.stream_invocation_with_activation(
                resource.clone(),
                Plan::Subscriptions(plan.clone()),
                &lease,
                deadline,
                Some(resumed.clone()),
            );
            match fresh {
                Ok(fresh)=>{
                    #[cfg(feature = "test-support")]
                    if let Some(probe)=&scene_probe {probe.store(4,std::sync::atomic::Ordering::Release);}
                    Ok(fresh)
                }
                Err(error)=>{
                    // The complete effect-free factory has returned: partial
                    // scene reads have dropped. Release its fresh grant too
                    // before the bounded helper may pause/reacquire authority.
                    drop(resumed);
                    if error.code=="authority-busy" {
                        #[cfg(feature = "test-support")]
                        if let Some(probe)=&scene_probe {
                            if probe.load(std::sync::atomic::Ordering::Acquire)==2 {
                                if scope_custody.as_ref().is_some_and(|held|held()) {
                                    probe.store(5,std::sync::atomic::Ordering::Release);
                                    return Err(Error::new("test-custody-retained","Original scene command scope custody remains after metadata refusal"));
                                }
                                probe.store(3,std::sync::atomic::Ordering::Release);
                            }
                        }
                        if !scene_busy_observed.swap(true,std::sync::atomic::Ordering::AcqRel) {
                            tracing::debug!(target:"connect_service::canonical_replacement::subscription_resources",
                                operation_id=?resource.envelope().scope(Scope::Operation),
                                phase="scene-projection",code=%error.code,
                                "Original prepared subscription awaits fresh complete scene metadata");
                        }
                    }
                    Err(error)
                }
            }

            });
            attempt
            }).await?;
            // Same original selected resources and routes; fresh permission and
            // expected revision are checked at the existing atomic Core commit.
            phase = "activate-once";
            let subscription_observation = native
                .media_controller(&id)
                .ok()
                .and_then(|owner| fresh.begin_subscription_observation(&owner));
            let outcome = tokio::select! { biased;
                _ = self.stop.cancelled() => return Err(Error::forbidden()),
                result = prepared.activate_with_held(fresh.clone()) => result.map_err(resource_error)?,
            };
            let historical = subscription_observation
                .as_ref()
                .and_then(|o| o.unchanged_av(lease.expected_revision(), outcome.revision));
            fresh.remember_confirmed_selection();
            // Do not retain activation authority in the outer reply slot while
            // the original historical outcome needs its short metadata writer.
            drop(fresh);
            Ok((outcome, resource, historical))
        };
        let result = work.await;
        if let Err(error) = &result {
            tracing::debug!(target:"connect_service::canonical_replacement::subscription_resources",
                phase, code=%error.code, "Original subscription resource phase stopped");
        }
        let (captured, resource, original_error) = match result {
            Ok((outcome, resource, observation)) => match {
                let revision = outcome.revision;
                super::subscription_observation::capture(
                    lease,
                    &resource,
                    outcome,
                    observation,
                    || captured_projection(&self, &id, revision),
                )
            } {
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
                tracing::debug!(target: "connect_service::canonical_replacement::subscription_resources", code=%error.code,
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
            let current = resource
                .reauthorize_subscription_resource_preparation(
                    &self.stop,
                    &physical_cancel,
                    deadline,
                )
                .await;
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
    tracing::debug!(target: "connect_service::canonical_replacement::subscription_resources", ?error,
        "Original subscription resource or activation stage refused");
    Error::new(
        "reconciliation-required",
        "Original subscription resource outcome requires reconciliation",
    )
}
