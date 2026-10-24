//! Original private SIP candidate job. Watch values retain metadata/owned Core
//! identity only, never the command's authority reader or a reusable IO grant.
use super::*;
use connect_store::NativeConsultationOrigin;
use tokio::sync::watch;

pub(super) type Outcome = Option<std::result::Result<Arc<policy::Leg>, String>>;

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
                connect_store::PrivateInitialTelephoneRuntimeObservation::observe_private(
                    &policy::Ready { leg: leg.clone() },
                )?;
                let read = leg.current_media_read()?;
                if !read.is_connected() || !read.is_private_admission() {
                    return Err(Error::forbidden());
                }
                return Ok(Some(leg.signal.origin().connection_id().into()));
            }
            tokio::select! {biased;
                _=self.stop.cancelled()=>return Err(Error::forbidden()),
                _=tokio::time::sleep_until(deadline)=>{
                    if let Some((_,cancel))=self.state.lock().map_err(|_|Error::storage())?.active.get(&key) {cancel.cancel();}
                    return Err(Error::new("deadline-exceeded","Original private SIP admission expired"));
                },
                changed=receiver.changed()=>if changed.is_err(){return Err(Error::new("reconciliation-required","Original private SIP job ended without capture"));},
            }
        }
    }
}

pub(super) async fn wait_quarantined(
    runtime: &Arc<Runtime>,
    adapter: &SipAdapter,
    leg: &Arc<policy::Leg>,
    deadline: tokio::time::Instant,
) -> Result<()> {
    loop {
        if !adapter.is_connection_live(&leg.connection()) {
            return Ok(());
        }
        connect_store::PrivateInitialTelephoneRuntimeObservation::observe_private(
            &policy::Ready { leg: leg.clone() },
        )?;
        // No public subscriptions, implicit receive programme or DTMF listener
        // is installed. This phase owns only the original closed quarantine.
        let read = leg.current_media_read()?;
        if !read.is_private_admission() || !read.is_connected() {
            return Err(Error::forbidden());
        }
        drop(read);
        tokio::select! {biased;
            _=runtime.stop.cancelled()=>return Ok(()),
            _=leg.cancel.cancelled()=>return Ok(()),
            _=tokio::time::sleep_until(deadline)=>return Ok(()),
            _=tokio::time::sleep(Duration::from_millis(100))=>{},
        }
    }
}
