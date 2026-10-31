//! Restore only the actual original Core audience handle under fresh return authority.
use crate::{
    canonical_consultation::{descriptor, retry_metadata, store_call},
    canonical_media::consultation::{AudienceInvocation, AudienceRead},
    canonical_runtime::Runtime,
};
use connect_protocol::{transitions::TransitionCommand, Error, Result};
use rvoip_core::ids::SessionId;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};
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
    pub(crate) fn queue_consultation_return(
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
        let Ok(mut work) = self.work.lock() else {
            queued.remove(&record);
            return;
        };
        let runtime = self.clone();
        let job = Job {
            runtime: Arc::downgrade(self),
            record: record.clone(),
        };
        work.spawn(async move {let _job=job;if let Err(error)=runtime.apply_consultation_return(credential,command,record).await{tracing::debug!(target:"connect_service::canonical_consultation",code=%error.code,"Native return did not complete");}});
    }
    async fn apply_consultation_return(
        self: &Arc<Self>,
        credential: String,
        command: TransitionCommand,
        record: String,
    ) -> Result<()> {
        let saved = self
            .consultation_returns
            .lock()
            .map_err(|_| Error::storage())?
            .get(&record)
            .cloned();
        if let Some(receipt) = saved {
            return self.settle_consultation_return(receipt).await;
        }
        let origin = retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let credential = credential.clone();
            let command = command.clone();
            store_call(move || store.native_consultation_return_origin(&credential, command))
        })
        .await.map_err(|error|{tracing::debug!(target:"connect_service::canonical_consultation",stage="return-origin",code=%error.code,"Native return refused");error})?;
        let (session, handle, start_receipt, original_receivers) = {
            let audiences = self
                .consultation_audiences
                .lock()
                .map_err(|_| Error::storage())?;
            let a = audiences
                .get(origin.consultation_record_id())
                .ok_or_else(Error::forbidden)?;
            (
                a.session.clone(),
                a.handle.clone(),
                a.receipt.clone(),
                a.original_receivers.clone(),
            )
        };
        if session != command.session_id
            || handle.audience().operation_id() != origin.consultation_record_id()
        {
            return Err(Error::forbidden());
        }
        tracing::debug!(target:"connect_service::canonical_consultation",stage="return-handle","Native return original handle matched");
        // A retained receipt only records a previous actual Core result. It cannot execute.
        retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let receipt = start_receipt.clone();
            store_call(move || store.record_native_consultation_applied(&receipt))
        })
        .await.map_err(|error|{tracing::debug!(target:"connect_service::canonical_consultation",stage="return-original-observation",code=%error.code,"Native return refused");error})?;
        let sid = SessionId::from_string(session);
        // A refused metadata reservation consumes its planning readers. Retry
        // with current sources and a fresh pure Core plan, never an old lease.
        // Attempt admission and the actual Core change remain outside this loop.
        let Some((lease, plan)) = retry_metadata(&self.stop, || async {
            let sources = self
                .media
                .scene_origins(&command.session_id)
                .map_err(|error|{tracing::debug!(target:"connect_service::canonical_consultation",stage="return-scene",?error,"Native return refused");Error::forbidden()})?;
            let sips = self.media.scene_sip_origins(&command.session_id).map_err(|_|Error::forbidden())?;
            let read = self.media.store.try_native_return_read(&origin, &sources, &sips).map_err(|error| {
                tracing::debug!(target:"connect_service::canonical_consultation",stage="return-read",code=%error.code,"Native return planning refused");error
            })?;
            let revision = self
                .core
                .audio_audience_revision(&sid)
                .map_err(|error| {tracing::debug!(target:"connect_service::canonical_consultation",stage="return-revision",?error,"Native return planning refused");Error::forbidden()})?;
            let plan = self
                .core
                .plan_restore_staged_audio_audience(
                    &sid,
                    handle.audience(),
                    origin.record_id().into(),
                    revision,
                )
                .map_err(|error|{tracing::debug!(target:"connect_service::canonical_consultation",stage="return-plan",?error,"Native return refused");Error::forbidden()})?;
            tracing::debug!(target:"connect_service::canonical_consultation",stage="return-planned",current=read.current(),source_count=sources.len(),receiver_count=plan.expected_receiver_revisions().len(),"Native return planning snapshot");
            // EOF can remove a receiver from Core before its original Store
            // lifetime/registry retirement is observed. This is still pure
            // planning: release every reader and retry fresh metadata rather
            // than reserve a plan against different source incarnations.
            // Store's exact selection/revision checks remain mandatory below.
            let current = sources.iter().map(|source| source.runtime_connection_id())
                .chain(sips.iter().map(|source| source.runtime_connection_id()))
                .collect::<BTreeSet<_>>();
            let planned = plan.expected_receiver_revisions().keys()
                .map(|id| id.as_str()).collect::<BTreeSet<_>>();
            if current != planned {
                tracing::debug!(target:"connect_service::canonical_consultation",stage="return-scene-pending",code="authority-busy",source_count=current.len(),receiver_count=planned.len(),"Native return planning observations differ");
                return Err(Error::new("authority-busy", "Native return scene observations are not yet aligned"));
            }
            let mut intent = descriptor(&plan)?;
            intent["consultation_record_id"] = serde_json::json!(origin.consultation_record_id());
            intent["restore"] = serde_json::json!(true);
            intent["known_departed_connections"] = serde_json::json!(original_receivers
                .iter()
                .filter(|id| !plan
                    .expected_receiver_revisions()
                    .keys()
                    .any(|current| current.as_str() == id.as_str()))
                .collect::<Vec<_>>());
            let store = self.media.store.clone();
            let Some(lease) =
                store_call(move || store.prepare_native_consultation_return(read, intent)).await.map_err(|error|{tracing::debug!(target:"connect_service::canonical_consultation",stage="return-prepare",code=%error.code,reason=%error.message,"Native return preparation refused");error})?
            else {
                return Ok(None);
            };
            Ok(Some((lease, plan)))
        })
        .await? else {
            return Ok(());
        };
        let read = retry_metadata(&self.stop, || async {
            self.media
                .store
                .native_consultation_return_attempt_read(&lease)
        })
        .await?;
        let store = self.media.store.clone();
        let (attempt, read) =
            store_call(move || store.begin_native_consultation_return(lease, read)).await?;
        let remaining = (read.expires_at() - chrono::Utc::now().timestamp()).max(0) as u64;
        let invocation = AudienceInvocation {
            policy: self.media.clone(),
            plan: plan.clone(),
            read: AudienceRead::Return(Arc::new(read)),
            cancel: self.stop.child_token(),
        };
        let actual = self
            .core
            .change_staged_audio_audience_with_held(
                &sid,
                plan,
                tokio::time::Instant::now() + Duration::from_secs(remaining),
                invocation.cancel.clone(),
                &invocation,
            )
            .await;
        let (outcome, handle) = match actual {
            Ok(result) => result,
            Err(error) => {
                tracing::debug!(target:"connect_service::canonical_consultation",stage="return-core",?error,"Actual native return did not complete");
                drop(invocation);
                let store = self.media.store.clone();
                store_call(move || store.native_consultation_return_uncertain(attempt)).await?;
                return Err(Error::new(
                    "reconciliation-required",
                    "Native restoration attempt is uncertain",
                ));
            }
        };
        let observation = serde_json::json!({"operation_id":outcome.operation_id,"audience_revision":outcome.audience_revision,"receiver_revisions":outcome.receiver_revisions.iter().map(|(k,v)|(k.as_str().to_owned(),*v)).collect::<BTreeMap<_,_>>(),"restored":outcome.restored,"departed_connections":outcome.departed_connections.iter().map(|c|c.as_str()).collect::<Vec<_>>(),"participant_handoff":null,"participant_replacement":null});
        let receipt = Arc::new(attempt.applied(
            observation,
            crate::canonical_runtime::capture_audience_metadata(handle.commit())?,
        )?);
        self.consultation_returns
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(origin.record_id().into(), receipt.clone());
        let carry_failed = !self.carry_audio_audience_commit(handle.commit(), &invocation);
        drop(invocation);
        retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let receipt = receipt.clone();
            store_call(move || store.record_native_consultation_return_applied(&receipt))
        })
        .await?;
        if carry_failed {
            return Err(Error::new(
                "native-confirmation-lost",
                "Native return confirmation could not follow the actual audience commit",
            ));
        }
        self.settle_consultation_return(receipt).await
    }
    async fn settle_consultation_return(
        &self,
        receipt: Arc<connect_store::NativeConsultationReturnApplied>,
    ) -> Result<()> {
        retry_metadata(&self.stop, || {
            let store = self.media.store.clone();
            let receipt = receipt.clone();
            store_call(move || store.record_native_consultation_return_applied(&receipt))
        })
        .await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let store = self.media.store.clone();
            let proof = receipt.clone();
            if store_call(move || store.complete_native_consultation_return(&proof))
                .await?
                .is_some()
            {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::new(
                    "reconciliation-required",
                    "Native private retirement is not yet observed",
                ));
            }
            tokio::select! {_=self.stop.cancelled()=>return Err(Error::forbidden()),_=tokio::time::sleep(Duration::from_millis(20))=>{}}
        }
    }
}
