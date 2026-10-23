//! Current authority is retained through one bounded INVITE write, never ringing.
use connect_store::{InitialTelephoneMediaOrigin, InitialTelephoneSignaling};
use rvoip_sip_core::{Message, Method};
use rvoip_sip_dialog::transaction::egress::{
    ClientRequestWriteAuthority, ClientRequestWriteFuture, ClientRequestWriteScope,
};
use std::sync::{Arc, OnceLock, Weak};
use tokio_util::sync::CancellationToken;
pub(crate) struct InviteAuthority {
    pub runtime: Weak<crate::canonical_runtime::Runtime>,
    pub signal: OnceLock<Arc<InitialTelephoneSignaling>>,
    pub media: OnceLock<Arc<InitialTelephoneMediaOrigin>>,
    pub cancel: CancellationToken,
    pub deadline: tokio::time::Instant,
}
fn denied() -> rvoip_sip_transport::Error {
    rvoip_sip_transport::Error::InvalidState("initial telephone write authority refused".into())
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
        let runtime = self.runtime.upgrade().ok_or_else(denied)?;
        let signal = self.signal.get().cloned().ok_or_else(denied)?;
        if request.method() != Method::Invite
            || runtime.stop.is_cancelled()
            || self.cancel.is_cancelled()
            || request.call_id().map(|v| v.value()).as_deref() != Some(signal.sip_call_id())
        {
            return Err(denied());
        }
        let expected = signal
            .origin()
            .source()
            .destination
            .parse::<rvoip_sip_core::Uri>()
            .map_err(|_| denied())?;
        if request.uri != expected {
            return Err(denied());
        }
        if let Some(origin) = self.media.get().cloned() {
            let read = runtime
                .media
                .store
                .try_initial_telephone_media_read(origin)
                .map_err(|_| denied())?;
            if !read.is_connected() {
                return Err(denied());
            }
            let until = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
            return tokio::select! {biased;
                _=self.cancel.cancelled()=>Err(denied()),
                _=tokio::time::sleep_until(until)=>Err(denied()),
                result=std::future::poll_fn(|cx|if read.current(){io.as_mut().poll(cx)}else{std::task::Poll::Ready(Err(denied()))})=>result,
            };
        }
        if tokio::time::Instant::now() >= self.deadline {
            return Err(denied());
        }
        let store = runtime.media.store.clone();
        let selected = signal.clone();
        let read =
            tokio::task::spawn_blocking(move || store.initial_telephone_signaling_read(&selected))
                .await
                .map_err(|_| denied())?
                .map_err(|_| denied())?;
        if read.sip_call_id() != Some(signal.sip_call_id())
            || read.runtime_connection_id() != Some(signal.runtime_connection_id())
        {
            return Err(denied());
        }
        let until =
            (tokio::time::Instant::now() + std::time::Duration::from_secs(2)).min(self.deadline);
        tokio::select! {biased;
            _=self.cancel.cancelled()=>Err(denied()),
            _=runtime.stop.cancelled()=>Err(denied()),
            _=tokio::time::sleep_until(until)=>Err(denied()),
            result=std::future::poll_fn(|cx|if read.current(){io.as_mut().poll(cx)}else{std::task::Poll::Ready(Err(denied()))})=>result,
        }
    }
}
