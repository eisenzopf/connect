//! Lifecycle-owned telephone workers; network effects occur after durable claims.
use connect_connectors::{voice::VoiceConnector, Failure, Vapi};
use connect_service::Service;
use connect_store::VoiceAction;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
pub struct Worker {
    stop: CancellationToken,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Worker {
    pub fn start(service: Arc<Service>) -> anyhow::Result<Self> {
        Self::with_driver(service, Arc::new(Vapi::from_env()?))
    }
    pub fn with_driver(
        service: Arc<Service>,
        driver: Arc<dyn VoiceConnector>,
    ) -> anyhow::Result<Self> {
        service.store.recover_voice_calls()?;
        let stop = CancellationToken::new();
        let discovery = tokio::spawn(discover(service.clone(), driver.clone(), stop.clone()));
        let control = tokio::spawn(control(service, driver, stop.clone()));
        Ok(Self {
            stop,
            tasks: vec![discovery, control],
        })
    }
    pub async fn shutdown(mut self) -> anyhow::Result<()> {
        self.stop.cancel();
        for mut task in self.tasks.drain(..) {
            if tokio::time::timeout(Duration::from_secs(5), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
                anyhow::bail!("Voice worker shutdown timed out; recover uncertain work");
            }
        }
        Ok(())
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.cancel();
        for task in self.tasks.drain(..) {
            task.abort();
        }
    }
}
async fn discover(
    service: Arc<Service>,
    driver: Arc<dyn VoiceConnector>,
    cancelled: CancellationToken,
) {
    loop {
        if cancelled.is_cancelled() {
            break;
        }
        let store = service.store.clone();
        let bindings = match tokio::task::spawn_blocking(move || store.voice_ingresses()).await {
            Ok(Ok(b)) => b,
            _ => {
                tracing::warn!("Inbound route storage unavailable; discovery stopped");
                break;
            }
        };
        for binding in bindings {
            let calls = tokio::select! {_=cancelled.cancelled()=>return,r=tokio::time::timeout(Duration::from_secs(35),driver.inbound(&binding))=>r};
            match calls {
                Ok(Ok(calls))=>{
                    let store=service.store.clone();
                    let saved=tokio::task::spawn_blocking(move|| {
                        for call in calls {
                            match store.import_voice_call(&binding.route.id,&call) {
                                Ok(_)=>{},
                                Err(e) if ["forbidden","unauthorized"].contains(&e.code.as_str())=>{},
                                Err(e)=>return Err(e),
                            }
                        }
                        store.complete_voice_scan(&binding)
                    }).await;
                    if matches!(saved,Ok(Ok(()))) {service.notify_changed();}
                    else {tracing::warn!("Inbound voice batch not fully committed; scan cursor preserved for retry");}
                }
                _=>tracing::warn!("Inbound voice discovery unavailable or scan budget exceeded; cursor preserved for retry"),
            }
        }
        tokio::select! {_=cancelled.cancelled()=>break,_=tokio::time::sleep(Duration::from_secs(5))=>{}}
    }
}
async fn control(
    service: Arc<Service>,
    driver: Arc<dyn VoiceConnector>,
    cancelled: CancellationToken,
) {
    loop {
        if cancelled.is_cancelled() {
            break;
        }
        let store = service.store.clone();
        let job = match tokio::task::spawn_blocking(move || store.claim_voice_job()).await {
            Ok(Ok(Some(j))) => j,
            Ok(Ok(None)) => {
                tokio::select! {_=cancelled.cancelled()=>break,_=tokio::time::sleep(Duration::from_millis(100))=>{}};
                continue;
            }
            _ => {
                tracing::warn!("Voice storage unavailable; control worker stopped");
                break;
            }
        };
        let result = tokio::select! {_=cancelled.cancelled()=>Err(Failure::Uncertain),r=tokio::time::timeout(Duration::from_secs(35),async {
            match job.action {
                VoiceAction::Start=>driver.start(&job).await,
                VoiceAction::Poll=>match job.call.provider_id.as_deref(){Some(id)=>driver.status(id).await,None=>Err(Failure::Rejected)},
                VoiceAction::End=>match job.call.provider_id.as_deref(){Some(id)=>driver.end(id,job.control_url.as_deref()).await,None=>Err(Failure::Rejected)},
            }
        })=>r.unwrap_or(Err(Failure::Uncertain))};
        // Read failures cannot invent a terminal outcome or erase accepted end intent.
        if result.is_err() && job.action == VoiceAction::Poll {
            continue;
        }
        let store = service.store.clone();
        let saved = tokio::task::spawn_blocking(move || match result {
            Ok(remote) => store.finish_voice_job(
                &job,
                &remote.state,
                Some(&remote.id),
                remote.control_url.as_deref(),
                remote.ended_reason.as_deref(),
            ),
            Err(f) => store.finish_voice_job(
                &job,
                if job.action == VoiceAction::Start && matches!(f, Failure::Rejected) {
                    "failed"
                } else {
                    "uncertain"
                },
                None,
                None,
                None,
            ),
        })
        .await;
        if !matches!(saved, Ok(Ok(()))) {
            tracing::warn!("Voice result not committed; control stopped for reconciliation");
            break;
        }
        service.notify_changed();
    }
}
