//! Exact signed native data publication with one owned attempt and queue receipt.
use crate::canonical_native_streams::CapturedStreamOutcome;
use crate::{
    canonical_runtime::{Physical, Runtime},
    StoreInvocationAuthority,
};
use connect_protocol::{transitions::OperationState, Error, Result};
use rvoip_core::{
    ids::ConnectionId,
    stream_lifecycle::{MediaFormat, Medium},
};
use rvoip_uctp::v2::auth_dispatch::AuthorizedEnvelope;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

/// This retries metadata reads only, before the single Core publication call.
/// Original invocation/physical lifetimes and the original attempt deadline
/// remain mandatory; a failed read lends no grant to the next attempt.
async fn data_metadata<T: Send>(
    until: tokio::time::Instant,
    stop: &CancellationToken,
    invocation: &CancellationToken,
    physical: &CancellationToken,
    mut read: impl FnMut() -> Result<T> + Send,
) -> Result<T> {
    loop {
        if stop.is_cancelled() || invocation.is_cancelled() || physical.is_cancelled() {
            return Err(Error::forbidden());
        }
        if tokio::time::Instant::now() >= until {
            return Err(Error::new(
                "authority-busy",
                "Native data metadata deadline elapsed",
            ));
        }
        match read() {
            Err(error) if error.code == "authority-busy" => {
                tokio::select! { biased;
                    _=stop.cancelled()=>return Err(Error::forbidden()),
                    _=invocation.cancelled()=>return Err(Error::forbidden()),
                    _=physical.cancelled()=>return Err(Error::forbidden()),
                    _=tokio::time::sleep_until(until.min(tokio::time::Instant::now()+Duration::from_millis(1)))=>{}
                }
            }
            result => return result,
        }
    }
}
fn proof(a: &AuthorizedEnvelope) -> Result<&connect_store::PeerAuthorityGuard> {
    a.authority::<StoreInvocationAuthority>()
        .and_then(|h| h.peer_guard())
        .ok_or_else(Error::forbidden)
}
fn reply(record: connect_protocol::transitions::OperationRecord) -> crate::canonical_native::Reply {
    crate::canonical_native::Reply {
        record,
        answered: false,
    }
}
pub(crate) fn failure(stage: &'static str, error: &Error) {
    let origin = match error.message.as_str() {
        "Invalid native application message" => "native-message-validation",
        "Invalid native stream selection intent" => "native-intent-validation",
        "Invalid transition lifecycle action" => "transition-action-validation",
        "Invalid addressed message" => "conversation-message-validation",
        "Logical request ID is required" => "receipt-id-validation",
        "Unsupported operation" => "ordinary-operation-resource-validation",
        _ => "other",
    };
    tracing::debug!(target: "connect_service::canonical_native_data", stage, code=%error.code, origin, "Native data did not complete");
}
pub(crate) fn checkpoint(stage: &'static str) {
    tracing::debug!(target: "connect_service::canonical_native_data", stage, "Native data stage reached");
}
impl Runtime {
    pub(crate) async fn native_data(
        self: &Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        physical: Arc<Physical>,
        credential: String,
    ) -> Result<crate::canonical_native::Reply> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let runtime = self.clone();
        {
            let mut work = self.work.lock().map_err(|_| Error::storage())?;
            while work.try_join_next().is_some() {}
            if self.stop.is_cancelled()
                || work.len() >= self.media.store.limits().connections.saturating_mul(2)
            {
                return Err(Error::new(
                    "busy",
                    "Native publication workers are at capacity",
                ));
            }
            work.spawn(async move {
                runtime.data_worker(a, physical, credential, tx).await;
            });
        }
        rx.await.map_err(|_| Error::storage())?
    }
    fn data_preflight(
        &self,
        a: &AuthorizedEnvelope,
        physical: &Physical,
        credential: &str,
    ) -> Result<ConnectionId> {
        let native = self.native_adapter(physical.binding.transport())?;
        let authority = a
            .authority::<StoreInvocationAuthority>()
            .ok_or_else(Error::forbidden)?;
        physical.validate(a, authority)?;
        if physical.lifetime.is_cancelled() || self.stop.is_cancelled() {
            return Err(Error::forbidden());
        }
        let data = self
            .media
            .store
            .project_native_data_with_peer(credential, proof(a)?)?;
        let id = ConnectionId::from_string(data.source.connection.runtime_connection_id);
        let bound = native
            .physical_source(&id)
            .map_err(|_| Error::forbidden())?;
        if !Arc::ptr_eq(&bound, &physical.binding) {
            return Err(Error::forbidden());
        }
        let controller = native
            .media_controller(&id)
            .map_err(|_| Error::forbidden())?;
        let bindings = controller.active_bindings();
        let mut selected = bindings.iter().filter(|b| {
            b.negotiated.requested.medium == Medium::Data
                && b.negotiated.requested.logical_id.as_str() == data.message.label
                && b.source_ready
        });
        let binding = selected.next().ok_or_else(|| {
            Error::new(
                "unsupported-media-profile",
                "Native data source label is not active",
            )
        })?;
        if selected.next().is_some() {
            return Err(Error::forbidden());
        }
        use rvoip_core::capability::DataProtocol;
        let valid = match &binding.negotiated.format {
            MediaFormat::ReliableOrdered(DataProtocol::Text) => {
                std::str::from_utf8(&data.message.bytes).is_ok()
            }
            MediaFormat::ReliableOrdered(DataProtocol::Json) => {
                serde_json::from_slice::<serde_json::Value>(&data.message.bytes).is_ok()
            }
            MediaFormat::ReliableOrdered(DataProtocol::Binary) => true,
            _ => false,
        };
        if !valid {
            return Err(Error::new(
                "unsupported-media-profile",
                "Message bytes do not match the negotiated native data format",
            ));
        }
        Ok(id)
    }
    async fn data_worker(
        self: Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        physical: Arc<Physical>,
        credential: String,
        tx: tokio::sync::oneshot::Sender<Result<crate::canonical_native::Reply>>,
    ) {
        checkpoint("worker-entered");
        let native = match self.native_adapter(physical.binding.transport()) {
            Ok(native) => native,
            Err(error) => {
                failure("native-adapter", &error);
                let _ = tx.send(Err(error));
                return;
            }
        };
        let store = &self.media.store;
        let metadata_deadline = tokio::time::Instant::now() + Duration::from_millis(200);
        let invocation_cancelled = a.invocation_cancellation();
        match data_metadata(
            metadata_deadline,
            &self.stop,
            &invocation_cancelled,
            &physical.lifetime,
            || proof(&a).and_then(|p| store.native_stream_record_with_peer(&credential, p)),
        )
        .await
        {
            Ok(Some(record)) if record.state != OperationState::Accepted => {
                let _ = tx.send(Ok(reply(record)));
                return;
            }
            Ok(_) => (),
            Err(e) => {
                failure("original-record-read", &e);
                let _ = tx.send(Err(e));
                return;
            }
        }
        // Before a new durable Message becomes visible, check the actual
        // physical source and active format. Failure never mutates a replay.
        let id = match data_metadata(
            metadata_deadline,
            &self.stop,
            &invocation_cancelled,
            &physical.lifetime,
            || self.data_preflight(&a, &physical, &credential),
        )
        .await
        {
            Ok(id) => id,
            Err(e) => {
                failure("source-format-preflight", &e);
                let _ = tx.send(Err(e));
                return;
            }
        };
        let accepted = match self.accept_after_managed(&a, &physical, &credential).await {
            Ok(v) => v,
            Err(e) => {
                failure("durable-admission", &e);
                let _ = tx.send(Err(e));
                return;
            }
        };
        if accepted.state != OperationState::Accepted {
            let _ = tx.send(Ok(reply(accepted)));
            return;
        }
        let (lease, started) =
            match proof(&a).and_then(|p| store.begin_native_stream_with_peer(&credential, p)) {
                Ok(Some(v)) => v,
                Ok(None) => {
                    let _ = tx.send(
                        proof(&a)
                            .and_then(|p| store.native_stream_record_with_peer(&credential, p))
                            .and_then(|r| r.ok_or_else(Error::forbidden))
                            .map(reply),
                    );
                    return;
                }
                Err(e) => {
                    failure("attempt-begin", &e);
                    let _ = tx.send(Err(e));
                    return;
                }
            };
        let seconds = (lease.expires_at() - chrono::Utc::now().timestamp()).max(0) as u64;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
        let captured = 'captured: {
            let invocation = match data_metadata(
                deadline.min(tokio::time::Instant::now() + Duration::from_millis(200)),
                &self.stop,
                &invocation_cancelled,
                &physical.lifetime,
                // This performs the complete exact attempt/source/message capture
                // under the original signed command's current interval. Initial
                // physical/format preflight preceded acceptance; the single actual
                // adapter call below rechecks its current physical peer and binding.
                || self.media.data_invocation(a.clone(), &lease, deadline),
            )
            .await
            {
                Ok(v) => v,
                Err(e) => {
                    failure("data-invocation", &e);
                    break 'captured CapturedStreamOutcome::NoEffect(lease.without_effect(e));
                }
            };
            // One Core call only. Its success is queue admission, not a writer ACK.
            checkpoint("core-publication-started");
            let outcome = native.publish_data(&id, &a);
            drop(invocation);
            if let Err(error) = outcome {
                tracing::debug!(target: "connect_service::canonical_native_data", stage="core-publication", code="adapter-error", origin="native-adapter", "Native data did not complete");
                tracing::debug!(?error, "Native data queue admission remains uncertain");
                break 'captured CapturedStreamOutcome::Uncertain(lease);
            }
            checkpoint("core-publication-admitted");
            let receipt = match lease.queued() {
                Ok(v) => v,
                Err(e) => {
                    failure("queue-receipt", &e);
                    let _ = tx.send(Err(e));
                    return;
                }
            };
            CapturedStreamOutcome::Committed(receipt)
        };
        match captured.record(store) {
            Ok(record) => {
                checkpoint("receipt-recorded");
                let _ = tx.send(Ok(reply(record)));
                return;
            }
            Err(error) => {
                failure("receipt-record", &error);
                tracing::debug!(code=%error.code,"Native data queue receipt awaits recording");
                let _ = tx.send(Ok(reply(started)));
            }
        }
        drop(a);
        drop(physical);
        drop(credential);
        captured.settle(store, &self.stop).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn data_metadata_rechecks_busy_without_reexecuting_a_mutation() {
        let stop = CancellationToken::new();
        let mut calls = 0;
        let value = data_metadata(
            tokio::time::Instant::now() + Duration::from_secs(1),
            &stop,
            &stop,
            &stop,
            || {
                calls += 1;
                if calls == 1 {
                    Err(Error::new("authority-busy", "busy"))
                } else {
                    Ok("original-operation")
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(value, "original-operation");
        assert_eq!(calls, 2);
        let mut calls = 0;
        let error = data_metadata(
            tokio::time::Instant::now() + Duration::from_secs(1),
            &stop,
            &stop,
            &stop,
            || {
                calls += 1;
                Err::<(), _>(Error::forbidden())
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "forbidden");
        assert_eq!(calls, 1);
    }
    #[tokio::test]
    async fn cancelled_or_expired_data_metadata_never_reaches_publication() {
        let stop = CancellationToken::new();
        let invocation = CancellationToken::new();
        let mut calls = 0;
        let error = data_metadata(
            tokio::time::Instant::now() + Duration::from_secs(1),
            &stop,
            &invocation,
            &stop,
            || {
                calls += 1;
                invocation.cancel();
                Err::<(), _>(Error::new("authority-busy", "busy"))
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "forbidden");
        assert_eq!(calls, 1);
        let mut calls = 0;
        let error = data_metadata(tokio::time::Instant::now(), &stop, &stop, &stop, || {
            calls += 1;
            Ok(())
        })
        .await
        .unwrap_err();
        assert_eq!(error.code, "authority-busy");
        assert_eq!(calls, 0);
    }
}
