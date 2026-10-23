//! One frozen submission attempt, with no retained reader during queue/ring/timers.
use super::*;
use connect_store::TelephoneReplacementAttempt;
use rvoip_sip_core::{Message, Method};
use rvoip_sip_dialog::transaction::egress::{
    ClientRequestWriteAuthority, ClientRequestWriteFuture, ClientRequestWriteScope,
};
use std::sync::{OnceLock, Weak};
use tokio_util::sync::CancellationToken;

pub(crate) struct InviteAuthority {
    pub runtime: Weak<Runtime>,
    pub attempt: OnceLock<Arc<TelephoneReplacementAttempt>>,
    pub established: OnceLock<Arc<connect_store::TelephoneMediaOrigin>>,
    pub initial_from: OnceLock<String>,
    pub cancelled: CancellationToken,
    pub deadline: tokio::time::Instant,
}
fn denied() -> rvoip_sip_transport::Error {
    rvoip_sip_transport::Error::InvalidState("telephone submission authority refused".into())
}
#[async_trait::async_trait]
impl ClientRequestWriteAuthority for InviteAuthority {
    async fn write(
        &self,
        scope: &ClientRequestWriteScope<'_>,
        mut io: ClientRequestWriteFuture<'_>,
    ) -> rvoip_sip_transport::Result<rvoip_sip_transport::TransportRoute> {
        let Message::Request(request) = scope.message else {
            return Err(denied());
        };
        if let Some(origin) = self.established.get().cloned() {
            let runtime = self.runtime.upgrade().ok_or_else(denied)?;
            let saved = origin.snapshot();
            let expected = saved
                .destination
                .parse::<rvoip_sip_core::Uri>()
                .map_err(|_| denied())?;
            if request.method() != Method::Invite
                || request.call_id().map(|id| id.value()).as_deref()
                    != Some(saved.media.call_id.as_str())
                || request.to().and_then(|to| to.tag()).is_none()
                || request
                    .from()
                    .and_then(|from| from.tag())
                    .map(str::to_owned)
                    .as_ref()
                    != self.initial_from.get()
                || request.uri.host_port() != expected.host_port()
                || request.uri.scheme != expected.scheme
                || runtime.stop.is_cancelled()
            {
                return Err(denied());
            }
            let read = crate::canonical_consultation::retry_metadata(&runtime.stop, || async {
                runtime.media.store.try_telephone_media_read(origin.clone())
            })
            .await
            .map_err(|_| denied())?;
            let until = tokio::time::Instant::now()
                + std::time::Duration::from_secs(
                    (read.expires_at() - chrono::Utc::now().timestamp()).max(0) as u64,
                );
            let live = || {
                runtime
                    .telephone_route(&saved.route_reference)
                    .is_ok_and(|(adapter, _)| {
                        adapter.is_connection_live(&rvoip_core::ids::ConnectionId::from_string(
                            saved.runtime_connection_id.clone(),
                        ))
                    })
            };
            if !live() {
                return Err(denied());
            }
            let result = tokio::select! {biased;
                _=runtime.stop.cancelled()=>Err(denied()),
                _=tokio::time::sleep_until(until)=>Err(denied()),
                result=std::future::poll_fn(|cx|{
                    if !read.current() || !live() || runtime.stop.is_cancelled() {return std::task::Poll::Ready(Err(denied()));}
                    io.as_mut().poll(cx)
                })=>result,
            };
            tracing::debug!(target:"connect_service::telephone_replacement",written=result.is_ok(),"Established telephone INVITE write settled");
            return result;
        }
        if request.method() != Method::Invite
            || self.cancelled.is_cancelled()
            || tokio::time::Instant::now() >= self.deadline
        {
            return Err(denied());
        }
        let runtime = self.runtime.upgrade().ok_or_else(denied)?;
        if runtime.stop.is_cancelled() {
            return Err(denied());
        }
        let attempt = self.attempt.get().cloned().ok_or_else(denied)?;
        let read = crate::canonical_consultation::retry_metadata(&self.cancelled, || {
            let store = runtime.media.store.clone();
            let attempt = attempt.clone();
            async move {
                tokio::task::spawn_blocking(move || store.telephone_replacement_submission_write_read(attempt))
                    .await.map_err(|_| Error::storage())?
            }
        }).await.map_err(|error| {
            tracing::debug!(target:"connect_service::telephone_replacement",code=%error.code,"Current INVITE write check refused");
            denied()
        })?;
        let source_current = runtime
            .media
            .replacement_source_owner(read.source())
            .is_ok();
        let current = read.current();
        let call_matches =
            request.call_id().map(|id| id.value()).as_deref() == Some(read.call_id());
        // SIP formatting escapes E.164's leading plus. Compare parsed URI
        // values, including every parameter/header, rather than raw spelling.
        let destination_matches = read
            .destination()
            .map_err(|_| denied())?
            .parse::<rvoip_sip_core::Uri>()
            .is_ok_and(|expected| expected == request.uri);
        if !source_current || !current || !call_matches || !destination_matches {
            tracing::debug!(target:"connect_service::telephone_replacement",source_current,current,call_matches,destination_matches,"Exact INVITE binding refused");
            return Err(denied());
        }
        let from = request
            .from()
            .and_then(|from| from.tag())
            .ok_or_else(denied)?
            .to_owned();
        if let Some(saved) = self.initial_from.get() {
            if saved != &from {
                return Err(denied());
            }
        } else {
            let _ = self.initial_from.set(from);
        }
        let until = self.deadline.min(
            tokio::time::Instant::now()
                + std::time::Duration::from_secs(
                    (read.expires_at() - chrono::Utc::now().timestamp()).max(0) as u64,
                ),
        );
        let result = tokio::select! {
            biased;
            _ = self.cancelled.cancelled() => Err(denied()),
            _ = runtime.stop.cancelled() => Err(denied()),
            _ = tokio::time::sleep_until(until) => Err(denied()),
            result = std::future::poll_fn(|cx| {
                if !read.current() || self.cancelled.is_cancelled() || runtime.stop.is_cancelled()
                    || tokio::time::Instant::now() >= self.deadline
                    || runtime.media.replacement_source_owner(read.source()).is_err() {
                    return std::task::Poll::Ready(Err(denied()));
                }
                // The exact IO future cannot be substituted with another route.
                io.as_mut().poll(cx)
            }) => result,
        };
        drop(read);
        result
    }
}
