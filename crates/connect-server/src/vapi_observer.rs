//! Source-attributed Vapi observations with bounded, owned teardown.

use connect_store::Store;
use rvoip_vapi::VapiEvent;
use std::{sync::Arc, time::Duration};
use tokio::sync::broadcast;

pub(crate) struct Observer {
    task: Option<tokio::task::JoinHandle<bool>>,
    store: Arc<Store>,
    session: String,
    provider: String,
}

impl Observer {
    pub(crate) fn start(
        store: Arc<Store>,
        session: String,
        provider: String,
        mut events: broadcast::Receiver<VapiEvent>,
    ) -> Self {
        let writer = Arc::clone(&store);
        let session_ref = session.clone();
        let provider_ref = provider.clone();
        let task = tokio::spawn(async move {
            let mut sequence = 0_u64;
            let mut last_final: Option<(String, String, String)> = None;
            loop {
                match events.recv().await {
                    Ok(VapiEvent::Transcript {
                        role: Some(role),
                        transcript_type: Some(kind),
                        transcript: Some(body),
                    }) if kind.eq_ignore_ascii_case("final") => {
                        // Vapi has no utterance ID. Immediate duplicate events
                        // reuse their reference; distinct utterances do not.
                        let source = match &last_final {
                            Some((last_role, last_body, source))
                                if last_role == &role && last_body == &body =>
                            {
                                source.clone()
                            }
                            _ => {
                                let source = format!("{provider_ref}:transcript:{sequence}");
                                sequence = sequence.saturating_add(1);
                                last_final = Some((role.clone(), body.clone(), source.clone()));
                                source
                            }
                        };
                        if let Err(error) = writer.record_session_transcript(
                            &session_ref,
                            &source,
                            &role,
                            &kind,
                            &body,
                        ) {
                            tracing::warn!(code = error.code, "could not persist Vapi transcript");
                        }
                    }
                    Ok(VapiEvent::EndOfCallReport { report }) => {
                        return match writer.record_session_outcome(
                            &session_ref,
                            &provider_ref,
                            &report,
                        ) {
                            Ok(_) => true,
                            Err(error) => {
                                tracing::warn!(
                                    code = error.code,
                                    "could not persist Vapi call outcome"
                                );
                                false
                            }
                        };
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let _ = writer.record_session_context_gap(
                            &session_ref,
                            &provider_ref,
                            "transcript_events",
                        );
                        tracing::warn!(
                            "Vapi transcript observer lagged; context may be incomplete"
                        );
                    }
                    Err(broadcast::error::RecvError::Closed) => return false,
                }
            }
        });
        Self {
            task: Some(task),
            store,
            session,
            provider,
        }
    }

    pub(crate) async fn finish(mut self) {
        let Some(task) = self.task.as_mut() else {
            return;
        };
        let reported = match tokio::time::timeout(Duration::from_secs(2), &mut *task).await {
            Ok(Ok(reported)) => reported,
            Ok(Err(_)) => false,
            Err(_) => {
                task.abort();
                let _ = (&mut *task).await;
                false
            }
        };
        self.task.take();
        if !reported {
            if let Err(error) =
                self.store
                    .record_session_context_gap(&self.session, &self.provider, "end_report")
            {
                tracing::warn!(code = error.code, "could not persist missing Vapi outcome");
            }
        }
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[cfg(test)]
#[path = "vapi_observer_tests.rs"]
mod tests;
