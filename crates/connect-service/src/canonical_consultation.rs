//! One current native audience application and observed native human completion.
//! AI provider context readiness remains a separate initialization boundary.
use crate::{
    canonical_media::consultation::{AudienceInvocation, AudienceRead},
    canonical_runtime::Runtime,
};
use connect_protocol::{transitions::TransitionCommand, Error, Result};
use connect_store::ConsultationMediaOrigin;
use rvoip_core::{
    ids::SessionId,
    staged_streams::{
        audio_subscriptions::AudioSubscription,
        consultation::{PrivateConsultationHandle, PrivateConsultationRequest},
    },
};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

pub(crate) struct AppliedAudience {
    pub session: String,
    pub handle: PrivateConsultationHandle,
    pub receipt: Arc<connect_store::NativeConsultationApplied>,
    confirmation_carried: bool,
    source_expires_at: i64,
    pub original_receivers: std::collections::BTreeSet<String>,
}
fn denied<T>(_: T) -> Error {
    Error::forbidden()
}
struct Job {
    runtime: std::sync::Weak<Runtime>,
    record: String,
}
impl Drop for Job {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.upgrade() {
            runtime
                .consultation_jobs
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&self.record);
        }
    }
}

impl Runtime {
    pub(crate) fn carry_audio_audience_commit(
        &self,
        commit: &rvoip_core::staged_streams::audience::AudioAudienceCommit,
        invocation: &AudienceInvocation,
    ) -> bool {
        let mut carried = true;
        for connection in commit.connections() {
            carried &= match self
                .media
                .observe_sip_audio_audience_commit(connection, commit, invocation)
            {
                Ok(Some(())) => true,
                Ok(None) => self
                    .native
                    .observe_audio_audience_commit(connection, commit)
                    .is_ok(),
                Err(_) => false,
            };
        }
        carried
    }
    /// The queue retains IDs and immutable intent only, never the original
    /// HTTP/signed command readers. Every worker obtains its own current source.
    pub(crate) fn queue_consultation(
        self: &Arc<Self>,
        credential: String,
        command: TransitionCommand,
        record: String,
    ) {
        let Ok(mut queued) = self.consultation_jobs.lock() else {
            return;
        };
        if queued.len() >= self.media.store.limits().connections || !queued.insert(record.clone()) {
            return;
        }
        let runtime = self.clone();
        let Ok(mut work) = self.work.lock() else {
            queued.remove(&record);
            return;
        };
        let job = Job {
            runtime: Arc::downgrade(self),
            record: record.clone(),
        };
        work.spawn(async move {
            let _job=job;
            let result=runtime.apply_consultation(credential.clone(),command.clone(),record.clone()).await;
            if let Err(error)=result {
                if error.code=="unsupported-media-profile" {
                    let recorded=retry_metadata(&runtime.stop,|| {
                        let store=runtime.media.store.clone();
                        let credential=credential.clone();let command=command.clone();
                        store_call(move||store.fail_native_consultation_preflight(&credential,command))
                    }).await;
                    if let Err(refusal)=recorded {tracing::debug!(target:"connect_service::canonical_consultation",code=%refusal.code,"Native consultation refusal recording stopped");}
                }
                tracing::debug!(target:"connect_service::canonical_consultation",code=%error.code,"Native consultation did not apply");
            }
        });
    }
    fn plan_consultation(
        &self,
        origin: &connect_store::NativeConsultationOrigin,
    ) -> Result<
        Option<(
            connect_store::NativeConsultationRead,
            rvoip_core::staged_streams::consultation::PrivateConsultationPlan,
        )>,
    > {
        let all = self
            .media
            .scene_origins(&origin.request().session_id)
            .map_err(denied)?;
        let primary_id = origin.request().connection_id.as_deref().ok_or_else(|| {
            Error::new(
                "unsupported-media-profile",
                "Consultation requires an explicit primary",
            )
        })?;
        let retained = origin.request().intent.retained();
        let mut candidate = None;
        for entry in &all {
            if self
                .media
                .store
                .native_consultation_candidate_matches(origin, entry)?
            {
                candidate = Some(entry.clone());
            }
        }
        let Some(candidate) = candidate else {
            return Ok(None);
        };
        let sips = self
            .media
            .scene_sip_origins(&origin.request().session_id)
            .map_err(denied)?;
        let mut sources = all
            .into_iter()
            .map(ConsultationMediaOrigin::Native)
            .chain(sips.into_iter().map(ConsultationMediaOrigin::Sip))
            .collect::<Vec<_>>();
        let mut take = |id: &str| -> Result<ConsultationMediaOrigin> {
            let index = sources
                .iter()
                .position(|s| s.connection_id() == id)
                .ok_or_else(Error::forbidden)?;
            Ok(sources.remove(index))
        };
        let primary = take(primary_id)?;
        let remote = take(retained)?;
        let consultant = take(candidate.connection_id())?;
        let selected = vec![primary.clone(), consultant];
        let mut outside = vec![];
        let mut outside_sip = vec![];
        for source in sources {
            match source {
                ConsultationMediaOrigin::Native(native) => outside.push(native),
                ConsultationMediaOrigin::Sip(sip) => outside_sip.push(sip),
            }
        }
        let read = self.media.store.try_native_consultation_scene_read(
            origin,
            &remote,
            &selected,
            &outside,
            &outside_sip,
        )?;
        for entry in &selected {
            let audio_only = match entry {
                ConsultationMediaOrigin::Native(_) => {
                    read.media(entry.connection_id(), None)
                        .ok_or_else(Error::forbidden)?
                        .target()
                        .session
                        .session
                        .media
                        == ["audio"]
                }
                ConsultationMediaOrigin::Sip(_) => {
                    read.sip(entry.connection_id())
                        .ok_or_else(Error::forbidden)?
                        .session()
                        .media
                        == ["audio"]
                }
            };
            if !audio_only {
                return Err(Error::new(
                    "unsupported-media-profile",
                    "Private audience requires an audio-only Session",
                ));
            }
        }
        let sid = SessionId::from_string(origin.request().session_id.clone());
        let plan = self
            .core
            .plan_private_consultation(
                &sid,
                PrivateConsultationRequest {
                    operation_id: origin.record_id().into(),
                    expected_revision: self.core.audio_audience_revision(&sid).map_err(denied)?,
                    primary: rvoip_core::ids::ConnectionId::from_string(
                        primary.runtime_connection_id().to_owned(),
                    ),
                    remote: rvoip_core::ids::ConnectionId::from_string(
                        remote.runtime_connection_id().to_owned(),
                    ),
                    consultant: rvoip_core::ids::ConnectionId::from_string(
                        candidate.runtime_connection_id().to_owned(),
                    ),
                    hold_source: None,
                },
            )
            .map_err(denied)?;
        Ok(Some((read, plan)))
    }
    async fn apply_consultation(
        self: &Arc<Self>,
        credential: String,
        command: TransitionCommand,
        record: String,
    ) -> Result<()> {
        if self.stop.is_cancelled() {
            return Err(Error::forbidden());
        }
        if self
            .consultation_audiences
            .lock()
            .map_err(|_| Error::storage())?
            .contains_key(&record)
        {
            return self.settle_consultation(&record).await;
        }
        let origin = retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let credential = credential.clone();
            let command = command.clone();
            store_call(move || store.native_consultation_origin(&credential, command))
        })
        .await?;
        if let connect_protocol::transitions::TransitionIntent::Consult {
            hold_treatment: connect_protocol::transitions::HoldTreatment::ConfiguredMedia,
            ..
        } = origin.request().intent
        {
            return Err(Error::new(
                "unsupported-media-profile",
                "Configured consultation hold source is not installed",
            ));
        }
        let sip_candidate = if let Some(host) = self
            .initial_telephone_host
            .get()
            .and_then(std::sync::Weak::upgrade)
        {
            host.prepare_private(origin.clone()).await?.is_some()
        } else {
            false
        };
        if let Some(initializer) = self
            .consultation_initializer
            .get()
            .filter(|_| !sip_candidate)
        {
            let result = initializer
                .initialize(crate::ConsultationInitialization {
                    credential_id: credential.clone(),
                    command: command.clone(),
                    origin: origin.clone(),
                })
                .await;
            if let Err(error) = result {
                let cleanup = tokio_util::sync::CancellationToken::new();
                let code = error.code.clone();
                let recorded = retry_metadata(&cleanup, || {
                    let store = self.media.store.clone();
                    let origin = origin.clone();
                    let code = code.clone();
                    store_call(move || {
                        store.fail_native_consultation_initialization(&origin, &code)
                    })
                })
                .await;
                if let Err(refusal) = recorded {
                    tracing::debug!(target:"connect_service::canonical_consultation",code=%refusal.code,"Consultation initialization failure recording deferred");
                }
                return Err(error);
            }
        }
        let Some((read, plan)) =
            retry_metadata(&self.stop, || async { self.plan_consultation(&origin) }).await?
        else {
            return Ok(());
        };
        let sid = SessionId::from_string(origin.request().session_id.clone());
        let descriptor = descriptor(plan.audience())?;
        // Consumes/drops planning readers before its metadata writer. The
        // private primary content floor is installed in this transaction.
        let store = self.media.store.clone();
        let Some(lease) =
            store_call(move || store.prepare_native_consultation(read, descriptor)).await?
        else {
            return Ok(());
        };
        // Preparing persisted an immutable plan but performed no IO. A status
        // poll or native lifetime writer can briefly own its authority fence;
        // retry only this fresh read, never begin or the Core submission.
        let preparation = Arc::new(lease.preparation());
        let started = async {
        let read_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let read = loop {
            match self.media.store.native_consultation_attempt_read(&lease) {
                Ok(read) => break read,
                Err(error)
                    if matches!(
                        error.code.as_str(),
                        "authority-busy" | "storage-unavailable"
                    ) && tokio::time::Instant::now() < read_deadline =>
                {
                    tokio::select! {
                        _ = self.stop.cancelled() => return Err(Error::forbidden()),
                        _ = tokio::time::sleep(Duration::from_millis(20)) => {}
                    }
                }
                Err(error) => {
                    tracing::debug!(target:"connect_service::canonical_consultation",code=%error.code,"Native consultation attempt read refused");
                    return Err(error);
                }
            }
        };
        let store = self.media.store.clone();
        store_call(move || store.begin_native_consultation(lease, read)).await
        }.await;
        let (attempt, read) = match started {
            Ok(value) => value,
            Err(error) => {
                // No new IO is authorized here. This receipt can cancel only
                // a durable started=0 preparation, even if the begin's commit
                // outcome was uncertain or its execution grant is now gone.
                let cleanup = tokio_util::sync::CancellationToken::new();
                let settled = retry_metadata(&cleanup, || {
                    let store = self.media.store.clone();
                    let proof = preparation.clone();
                    store_call(move || store.settle_native_consultation_preparation(&proof))
                })
                .await;
                if let Err(refusal) = settled {
                    tracing::debug!(target:"connect_service::canonical_consultation",code=%refusal.code,"Unstarted native consultation settlement deferred");
                }
                return Err(error);
            }
        };
        let remaining = (read.expires_at() - chrono::Utc::now().timestamp()).max(0) as u64;
        let invocation = AudienceInvocation {
            policy: self.media.clone(),
            plan: plan.audience().clone(),
            read: AudienceRead::Consult(Arc::new(read)),
            cancel: self.stop.child_token(),
        };
        let original_receivers = plan
            .audience()
            .request()
            .receivers
            .keys()
            .map(|id| id.as_str().to_owned())
            .collect();
        let applied = self
            .core
            .start_private_consultation_with_held(
                &sid,
                plan,
                tokio::time::Instant::now() + Duration::from_secs(remaining),
                invocation.cancel.clone(),
                &invocation,
            )
            .await;
        let applied = match applied {
            Ok(applied) => applied,
            Err(error) => {
                drop(invocation);
                let store = self.media.store.clone();
                store_call(move || store.native_consultation_uncertain(attempt)).await?;
                return Err(denied(error));
            }
        };
        let (outcome, handle) = applied;
        let observation = serde_json::json!({"operation_id":outcome.operation_id,"audience_revision":outcome.audience_revision,"receiver_revisions":outcome.receiver_revisions.iter().map(|(k,v)|(k.as_str().to_owned(),*v)).collect::<BTreeMap<_,_>>(),"restored":outcome.restored,"departed_connections":outcome.departed_connections.iter().map(|c|c.as_str()).collect::<Vec<_>>(),"participant_handoff":null,"participant_replacement":null});
        let receipt = Arc::new(attempt.applied(observation)?);
        self.consultation_audiences
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                origin.record_id().into(),
                AppliedAudience {
                    session: sid.as_str().into(),
                    handle: handle.clone(),
                    receipt: receipt.clone(),
                    original_receivers,
                    source_expires_at: origin.request().expires_at,
                    confirmation_carried: false,
                },
            );

        // The sealed Core commit pins the exact unchanged native ports. This
        // carries confirmation; it is not another negotiation/readiness claim.
        let carry_failed =
            !self.carry_audio_audience_commit(handle.audience().commit(), &invocation);
        if let Some(applied) = self
            .consultation_audiences
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(origin.record_id())
        {
            applied.confirmation_carried = !carry_failed;
        }
        drop(invocation);
        self.settle_consultation(origin.record_id()).await
    }
    async fn settle_consultation(&self, record: &str) -> Result<()> {
        let (session, handle, receipt, carried, expires) = {
            let saved = self
                .consultation_audiences
                .lock()
                .map_err(|_| Error::storage())?;
            let a = saved.get(record).ok_or_else(Error::storage)?;
            (
                SessionId::from_string(a.session.clone()),
                a.handle.clone(),
                a.receipt.clone(),
                a.confirmation_carried,
                a.source_expires_at,
            )
        };
        // A recording retry never performs another Core call or reacquires an
        // obsolete command grant. Readers were dropped above.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let store = self.media.store.clone();
            let proof = receipt.clone();
            match store_call(move || store.record_native_consultation_applied(&proof)).await {
                Ok(_) => break,
                Err(e)
                    if matches!(e.code.as_str(), "authority-busy" | "storage-unavailable")
                        && tokio::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(Duration::from_millis(10)).await
                }
                Err(e) => return Err(e),
            }
        }
        if !carried {
            let store = self.media.store.clone();
            let proof = receipt.clone();
            store_call(move || {
                store.record_native_consultation_interrupted(&proof, "native-confirmation-lost")
            })
            .await?;
            return Err(Error::new(
                "native-confirmation-lost",
                "Native route confirmation could not follow the actual audience commit",
            ));
        }
        let remaining = (expires - chrono::Utc::now().timestamp()).clamp(0, 30) as u64;
        let outcome = self
            .core
            .wait_private_consultation_audio(
                &session,
                &handle,
                tokio::time::Instant::now() + Duration::from_secs(remaining),
                self.stop.child_token(),
            )
            .await
            .map_err(denied)?;
        use rvoip_core::staged_streams::consultation::PrivateConsultationMediaOutcome as Media;
        let failure = match outcome {
            Media::DecodedAudioObserved => None,
            Media::MediaTimeout => Some("timeout"),
            Media::Cancelled => Some("cancelled"),
            Media::PrimaryEnded => Some("primary-ended"),
            Media::RemoteEnded => Some("remote-ended"),
            Media::ConsultantEnded => Some("consultant-ended"),
            Media::AlreadyRestored => Some("already-restored"),
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let store = self.media.store.clone();
            let proof = receipt.clone();
            let result = store_call(move || match failure {
                Some(reason) => store.record_native_consultation_interrupted(&proof, reason),
                None => {
                    store.record_native_consultation_decoded(&proof)?;
                    store.complete_native_consultation(&proof).map(|_| ())
                }
            })
            .await;
            match result {
                Ok(()) => break,
                Err(e)
                    if matches!(e.code.as_str(), "authority-busy" | "storage-unavailable")
                        && tokio::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(Duration::from_millis(10)).await
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}
pub(super) fn descriptor(
    plan: &rvoip_core::staged_streams::audience::AudioAudiencePlan,
) -> Result<serde_json::Value> {
    let request = plan.request();
    let receivers = request
        .receivers
        .iter()
        .map(|(id, sources)| {
            let sources = sources
                .iter()
                .map(|s| match s {
                    AudioSubscription::Connection(id) => Ok(id.as_str()),
                    _ => Err(Error::forbidden()),
                })
                .collect::<Result<Vec<_>>>()?;
            Ok((id.as_str().to_owned(), sources))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    Ok(
        serde_json::json!({"operation_id":request.operation_id,"audience_revision":request.expected_revision,"private_connections":request.private_connections.iter().map(|c|c.as_str()).collect::<Vec<_>>(),"receivers":receivers,"activate_muted":request.activate_muted.as_ref().map(|c|c.as_str()),"expected_receiver_revisions":plan.expected_receiver_revisions().iter().map(|(id,r)|(id.as_str().to_owned(),*r)).collect::<BTreeMap<_,_>>()}),
    )
}

pub(super) async fn store_call<T: Send + 'static>(
    f: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| Error::storage())?
}

// Only pre-submission planning or idempotent refusal recording belongs here.
// Begin and Core submission must never be wrapped in this retry helper.
pub(super) async fn retry_metadata<T, F, Fut>(
    stop: &tokio_util::sync::CancellationToken,
    mut call: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if stop.is_cancelled() {
            return Err(Error::forbidden());
        }
        match call().await {
            Ok(value) => return Ok(value),
            Err(error)
                if matches!(
                    error.code.as_str(),
                    "authority-busy" | "storage-unavailable"
                ) && tokio::time::Instant::now() < deadline =>
            {
                tokio::select! {_=stop.cancelled()=>return Err(Error::forbidden()),_=tokio::time::sleep(Duration::from_millis(20))=>{}}
            }
            Err(error) => return Err(error),
        }
    }
}

impl crate::Service {
    /// Server-owned provider composition over this runtime's actual native scene.
    /// Both named credentials are independently checked by the Store. This is
    /// not a wire command, a model tool, or an unsigned physical invitation.
    pub fn prepare_private_provider_context(
        &self,
        credential: &str,
        command: TransitionCommand,
        context_credential: &str,
        endpoint: &str,
    ) -> Result<connect_store::PrivateProviderContext> {
        let runtime = self
            .canonical_runtime
            .get()
            .ok_or_else(|| Error::new("not-ready", "Canonical native runtime is not installed"))?;
        let origin = self.store.native_consultation_origin(credential, command)?;
        let (read, _) = runtime.plan_consultation(&origin)?.ok_or_else(|| {
            Error::new("not-ready", "Fresh private native candidate is not ready")
        })?;
        self.store
            .prepare_private_provider_context(read, context_credential, endpoint)
    }
}

impl crate::Service {
    /// Scheduling after captured provider initialization; current origin and
    /// native custody still decide whether any audience action may begin.
    pub fn resume_private_consultation(
        &self,
        credential: &str,
        command: TransitionCommand,
    ) -> Result<()> {
        let runtime = self.canonical_runtime.get().ok_or_else(Error::forbidden)?;
        let origin = self
            .store
            .native_consultation_origin(credential, command.clone())?;
        runtime.queue_consultation(credential.into(), command, origin.record_id().into());
        Ok(())
    }
}
