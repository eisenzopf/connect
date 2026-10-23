//! Exact signed native data publication with one owned attempt and queue receipt.
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
        let bound = self
            .native
            .physical_source(&id)
            .map_err(|_| Error::forbidden())?;
        if !Arc::ptr_eq(&bound, &physical.binding) {
            return Err(Error::forbidden());
        }
        let controller = self
            .native
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
        let store = &self.media.store;
        match proof(&a).and_then(|p| store.native_stream_record_with_peer(&credential, p)) {
            Ok(Some(record)) if record.state != OperationState::Accepted => {
                let _ = tx.send(Ok(reply(record)));
                return;
            }
            Ok(_) => (),
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        }
        // Before a new durable Message becomes visible, check the actual
        // physical source and active format. Failure never mutates a replay.
        let id = match self.data_preflight(&a, &physical, &credential) {
            Ok(id) => id,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        let accepted =
            match proof(&a).and_then(|p| store.accept_native_stream_with_peer(&credential, p)) {
                Ok(v) => v,
                Err(e) => {
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
                    let _ = tx.send(Err(e));
                    return;
                }
            };
        let seconds = (lease.expires_at() - chrono::Utc::now().timestamp()).max(0) as u64;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
        let invocation = match (|| {
            self.data_preflight(&a, &physical, &credential)?;
            store.validate_native_stream_attempt(&credential, proof(&a)?, &lease)?;
            self.media.data_invocation(a.clone(), &lease, deadline)
        })() {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(store.fail_native_stream_without_effect(lease, e).map(reply));
                return;
            }
        };
        // One Core call only. Its success is queue admission, not a writer ACK.
        let outcome = self.native.publish_data(&id, &a);
        drop(invocation);
        if let Err(error) = outcome {
            tracing::debug!(?error, "Native data queue admission remains uncertain");
            let _ = tx.send(store.native_stream_attempt_uncertain(&lease).map(reply));
            return;
        }
        let receipt = match lease.queued() {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        match store.complete_native_stream(&receipt) {
            Ok(record) => {
                let _ = tx.send(Ok(reply(record)));
                return;
            }
            Err(error) => {
                tracing::debug!(code=%error.code,"Native data queue receipt awaits recording");
                let _ = tx.send(Ok(reply(started)));
            }
        }
        drop(a);
        drop(physical);
        drop(credential);
        loop {
            tokio::select! {_=self.stop.cancelled()=>return,_=tokio::time::sleep(Duration::from_millis(100))=>{}}
            match store.complete_native_stream(&receipt) {
                Ok(_) => return,
                Err(e) if e.code == "forbidden" => return,
                Err(_) => (),
            }
        }
    }
}
