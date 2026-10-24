//! Original private SIP candidate job. Watch values retain metadata/owned Core
//! identity only, never the command's authority reader or a reusable IO grant.
use super::*;
use connect_store::{NativeConsultationOrigin, PrivateInitialTelephonePhase};
use tokio::sync::watch;

pub(super) type Outcome = Option<std::result::Result<Arc<policy::Leg>, String>>;

fn retryable_metadata(error: &Error) -> bool {
    matches!(
        error.code.as_str(),
        "authority-busy" | "storage-unavailable"
    )
}
fn current_private_candidate(leg: &Arc<policy::Leg>) -> Result<()> {
    let runtime = leg.runtime().map_err(|_| Error::forbidden())?;
    let origin = leg.origin().map_err(|_| Error::forbidden())?;
    let phase = runtime
        .media
        .store
        .private_initial_telephone_phase(&origin)?;
    let ready = policy::Ready { leg: leg.clone() };
    match phase {
        PrivateInitialTelephonePhase::Quarantined => {
            connect_store::PrivateInitialTelephoneRuntimeObservation::observe_private(&ready)?;
            let read = leg.current_media_read()?;
            if !read.is_private_admission() || !read.is_connected() {
                return Err(Error::forbidden());
            }
        }
        PrivateInitialTelephonePhase::Closed => return Err(Error::forbidden()),
        _ => ready.observe_private_lifetime()?,
    }
    Ok(())
}

impl Service {
    /// Protected coordinator composition over an already admitted public
    /// consultation. None means no configured SIP target (or a native candidate).
    /// A returned ID is the SAME reserved private Connection, not completion of
    /// the consultation or permission to route its audio.
    pub async fn prepare_private_sip_consultation_candidate(
        &self,
        origin: NativeConsultationOrigin,
    ) -> Result<Option<String>> {
        let runtime = self.canonical_runtime.get().ok_or_else(Error::forbidden)?;
        let Some(host) = runtime.initial_telephone_host.get().and_then(Weak::upgrade) else {
            return Ok(None);
        };
        host.prepare_private(origin).await
    }
}

impl Host {
    pub(crate) async fn prepare_private(
        self: &Arc<Self>,
        origin: NativeConsultationOrigin,
    ) -> Result<Option<String>> {
        let runtime = self.runtime.upgrade().ok_or_else(Error::forbidden)?;
        let store = runtime.media.store.clone();
        let target_origin = origin.clone();
        let target = crate::canonical_consultation::store_call(move || {
            store.native_consultation_target(&target_origin)
        })
        .await?;
        if target.candidate_kind.as_deref() == Some("native") {
            return Ok(None);
        }
        let Some(profile) = self
            .profiles
            .iter()
            .find(|p| p.identity == target.identity_id)
        else {
            return Ok(None);
        };
        let route = profile.route.clone();
        let key = format!("private:{}", origin.record_id());
        let existing = {
            self.state
                .lock()
                .map_err(|_| Error::storage())?
                .private
                .get(&key)
                .cloned()
        };
        let mut receiver = if let Some(existing) = existing {
            existing
        } else {
            let store = runtime.media.store.clone();
            let parent = origin.clone();
            let policy = route.policy.clone();
            let initial = crate::canonical_consultation::store_call(move || {
                let reservation = store.reserve_sip_consultation_leg(&parent, policy)?;
                store.prepare_private_initial_telephone(&reservation)
            })
            .await?;
            let mut state = self.state.lock().map_err(|_| Error::storage())?;
            if state.closed || self.stop.is_cancelled() {
                return Err(Error::forbidden());
            }
            while let Some(result) = state.jobs.try_join_next() {
                state.unconfirmed |= !matches!(result, Ok(Ok(())));
            }
            if let Some(existing) = state.private.get(&key) {
                existing.clone()
            } else {
                // Completed private jobs have sent an error/absence outcome. A
                // retried operation still meets Store's immutable once-only intent.
                let active = state
                    .active
                    .keys()
                    .cloned()
                    .collect::<std::collections::BTreeSet<_>>();
                state.private.retain(|key, rx| {
                    active.contains(key) || !matches!(rx.borrow().as_ref(), Some(Err(_)))
                });
                if state.active.len() >= self.capacity || state.private.len() >= self.capacity {
                    return Err(Error::new(
                        "capacity-exceeded",
                        "Private SIP job capacity exhausted",
                    ));
                }
                let cancel = self.stop.child_token();
                let (sender, receiver) = watch::channel(None);
                state.active.insert(
                    key.clone(),
                    (initial.source().session_id.clone(), cancel.clone()),
                );
                state.private.insert(key.clone(), receiver.clone());
                let active = Active {
                    host: Arc::downgrade(self),
                    key: key.clone(),
                };
                let runner = Runner {
                    runtime: self.runtime.clone(),
                    adapter: self.adapter.clone(),
                };
                let runtime = runtime.clone();
                let parent = origin.clone();
                state.jobs.spawn(async move {
                    let _active = active;
                    let result = runner
                        .run_allocated(
                            &runtime,
                            initial.clone(),
                            route,
                            cancel,
                            Some(sender.clone()),
                        )
                        .await;
                    if result.is_err() {
                        let store = runtime.media.store.clone();
                        let initial = initial.clone();
                        let _ = crate::canonical_consultation::store_call(move || {
                            store.abandon_initial_telephone_unstarted(&initial)
                        })
                        .await;
                    }
                    // Possible submission keeps the root unsettled. Captured
                    // cleanup may settle initialization failure, never invent it.
                    let code = result
                        .as_ref()
                        .err()
                        .map(|e| e.code.clone())
                        .unwrap_or_else(|| "candidate-ended".into());
                    let store = runtime.media.store.clone();
                    let failure = code.clone();
                    let _ = crate::canonical_consultation::store_call(move || {
                        store.fail_native_consultation_initialization(&parent, &failure)
                    })
                    .await;
                    sender.send_replace(Some(Err(code)));
                    result
                });
                receiver
            }
        };
        let deadline =
            tokio::time::Instant::now()
                + Duration::from_secs(u64::from(profile.route.policy.timeout_seconds).min(
                    (origin.request().expires_at - chrono::Utc::now().timestamp()).max(0) as u64,
                ));
        loop {
            let outcome = receiver.borrow().clone();
            if let Some(outcome) = outcome {
                let leg = outcome.map_err(|code| {
                    Error::new(&code, "Original private SIP candidate is unavailable")
                })?;
                if leg.signal.origin().source().parent.record_id() != origin.record_id() {
                    return Err(Error::forbidden());
                }
                match current_private_candidate(&leg) {
                    Ok(()) => return Ok(Some(leg.signal.origin().connection_id().into())),
                    Err(error) if retryable_metadata(&error) => {}
                    Err(error) => return Err(error),
                }
            }
            tokio::select! {biased;
                _=self.stop.cancelled()=>return Err(Error::forbidden()),
                _=tokio::time::sleep_until(deadline)=>{
                    if let Some((_,cancel))=self.state.lock().map_err(|_|Error::storage())?.active.get(&key) {cancel.cancel();}
                    return Err(Error::new("deadline-exceeded","Original private SIP admission expired"));
                },
                changed=receiver.changed()=>if changed.is_err(){return Err(Error::new("reconciliation-required","Original private SIP job ended without capture"));},
                _=tokio::time::sleep(Duration::from_millis(10))=>{},
            }
        }
    }
}

pub(super) async fn wait_private(
    runtime: &Arc<Runtime>,
    adapter: &Arc<SipAdapter>,
    leg: &Arc<policy::Leg>,
) -> Result<()> {
    let origin = leg.origin().map_err(|_| Error::forbidden())?;
    let mut received = None;
    loop {
        if !adapter.is_connection_live(&leg.connection()) {
            return Ok(());
        }
        // No reader is retained while idle. Phases only keep the exact owned
        // call; every packet/signal still obtains its own purpose-bound grant.
        let phase = match runtime.media.store.private_initial_telephone_phase(&origin) {
            Ok(PrivateInitialTelephonePhase::Closed) => return Ok(()),
            Ok(phase) => {
                let ready = policy::Ready { leg: leg.clone() };
                let observed = if phase == PrivateInitialTelephonePhase::Quarantined {
                    connect_store::PrivateInitialTelephoneRuntimeObservation::observe_private(
                        &ready,
                    )
                    .map(|_| ())
                } else {
                    ready.observe_private_lifetime()
                };
                match observed {
                    Ok(()) => Some(phase),
                    Err(error) if retryable_metadata(&error) => None,
                    Err(error) => return Err(error),
                }
            }
            // Busy metadata is not an end/withdrawal observation. Keep only
            // this already closed original call; every packet guard still
            // refuses IO until a fresh interval can be obtained.
            Err(error) if retryable_metadata(&error) => None,
            Err(error) => return Err(error),
        };
        if matches!(
            phase,
            Some(PrivateInitialTelephonePhase::Applied | PrivateInitialTelephonePhase::Established)
        ) {
            leg.dtmf_registration
                .get_or_init(|| format!("dtmfsip_{}", uuid::Uuid::new_v4().simple()));
            if received.is_none() {
                if let Some(controller) = leg.controller.get().and_then(Weak::upgrade) {
                    if let Ok(subscription) = adapter.subscribe_managed_dtmf_source(
                        &runtime.core,
                        leg.connection(),
                        &controller,
                        leg.target.principal.clone(),
                    ) {
                        received = Some(subscription);
                    }
                }
            }
        }
        if let Some(subscription) = received.as_mut() {
            tokio::select! {biased;
                _=runtime.stop.cancelled()=>return Ok(()),
                _=leg.cancel.cancelled()=>return Ok(()),
                input=subscription.recv()=>match input {
                    Some(observed)=>if let Err(error)=runtime.enqueue_received_dtmf(crate::canonical_sip_signal::SipLeg::Initial(leg.clone()), observed) {
                        tracing::debug!(code=%error.code,"Received private SIP signal refused before journal admission");
                    },
                    None=>received=None,
                },
                _=tokio::time::sleep(Duration::from_millis(100))=>{},
            }
            continue;
        }
        tokio::select! {biased;
            _=runtime.stop.cancelled()=>return Ok(()),
            _=leg.cancel.cancelled()=>return Ok(()),
            _=tokio::time::sleep(Duration::from_millis(100))=>{},
        }
    }
}
