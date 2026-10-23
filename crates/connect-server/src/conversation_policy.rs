//! Bounded, supervised local idle policy. Durable journal rows carry the facts.
use connect_service::Service;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
pub(crate) struct Worker {
    stop: CancellationToken,
    task: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
}
impl Worker {
    pub(crate) fn start(service: Arc<Service>) -> Self {
        let stop = CancellationToken::new();
        let cancelled = stop.clone();
        let task = crate::worker::spawn(
            service.readiness.clone(),
            "conversation-policy",
            stop.clone(),
            async move {
                loop {
                    tokio::select! {biased;_=cancelled.cancelled()=>break,_=tokio::time::sleep(Duration::from_secs(1))=>{}}
                    let store = service.store.clone();
                    let count =
                        tokio::task::spawn_blocking(move || store.expire_idle_conversations(32))
                            .await??;
                    if count > 0 {
                        service.notify_changed();
                    }
                }
                Ok(())
            },
        );
        Self {
            stop,
            task: Some(task),
        }
    }
    pub(crate) async fn shutdown(mut self) -> anyhow::Result<()> {
        self.stop.cancel();
        crate::worker::drain(
            self.task.take().into_iter().collect(),
            Duration::from_secs(5),
        )
        .await
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
