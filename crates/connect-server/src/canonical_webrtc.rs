//! Owned WebRTC transport preparation for one durable Connection attempt.
//! This backend component is not yet a configured canonical signaling/media route.
use connect_protocol::{provisioning::EndpointTransport, Error, Result};
use connect_store::{NativeMediaOrigin, NativeOfferLease, Store};
use futures_util::{future::BoxFuture, FutureExt};
use rvoip_webrtc::{
    peer::{PeerRole, RvoipPeerConnection},
    WebRtcConfig,
};
use std::{collections::BTreeMap, str::FromStr, sync::Arc, time::Duration};
use tokio::{
    sync::{watch, OwnedSemaphorePermit},
    time::Instant,
};
use tokio_util::sync::CancellationToken;
type Completion = futures_util::future::Shared<BoxFuture<'static, Result<()>>>;
#[derive(Clone)]
enum State {
    Preparing,
    Answered(String),
    Connected(String),
    Closed(Result<()>),
}
#[cfg(test)]
#[derive(Default)]
struct CloseGate {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
#[cfg(test)]
type CloseGateSlot = Arc<std::sync::Mutex<Option<Arc<CloseGate>>>>;

/// Owns the non-cloneable Store attempt, actual peer worker and host capacity.
/// No capture, media publication, Core route or remote success is inferred here.
pub struct Peer {
    store: Arc<Store>,
    origin: Arc<NativeMediaOrigin>,
    attempt: Arc<NativeOfferLease>,
    stop: CancellationToken,
    state: watch::Receiver<State>,
    completion: Completion,
    #[cfg(test)]
    close_gate: CloseGateSlot,
}
impl Peer {
    /// Transfers the single Store-issued attempt before any asynchronous work.
    /// Keeping this owner outside answer/connection waits preserves the original
    /// lease even when a wait is cancelled or setup fails.
    pub fn start(
        store: Arc<Store>,
        origin: Arc<NativeMediaOrigin>,
        attempt: NativeOfferLease,
        config: WebRtcConfig,
        physical: &CancellationToken,
        permit: OwnedSemaphorePermit,
        deadline: Instant,
    ) -> Self {
        let stop = physical.child_token();
        let (state_tx, state) = watch::channel(State::Preparing);
        let attempt = Arc::new(attempt);
        #[cfg(test)]
        let close_gate = CloseGateSlot::default();
        let task = tokio::spawn(worker(
            store.clone(),
            origin.clone(),
            attempt.clone(),
            config,
            stop.clone(),
            permit,
            deadline,
            state_tx,
            #[cfg(test)]
            close_gate.clone(),
        ));
        let completion = async move {
            task.await
                .map_err(|_| Error::new("cleanup-unconfirmed", "WebRTC worker join failed"))?
        }
        .boxed()
        .shared();
        Self {
            store,
            origin,
            attempt,
            stop,
            state,
            completion,
            #[cfg(test)]
            close_gate,
        }
    }
    /// Read-only custody for the caller's later attributed readiness/cleanup.
    /// It cannot be moved into a second peer or deserialized as another attempt.
    pub fn original_attempt(&self) -> &NativeOfferLease {
        &self.attempt
    }
    /// Current backend snapshot only. Physical reply delivery still needs its
    /// exact current canonical signaling authority held by the caller.
    pub async fn answer_sdp(&self, deadline: Instant) -> Result<String> {
        let mut state = self.state.clone();
        loop {
            let current = state.borrow().clone();
            match current {
                State::Answered(answer) | State::Connected(answer) => {
                    if self.stop.is_cancelled() {
                        return Err(Error::forbidden());
                    }
                    original_sdp(&self.store, &self.origin)?;
                    return Ok(answer);
                }
                State::Closed(result) => {
                    return result.and_then(|_| {
                        Err(Error::new("closed", "WebRTC peer closed before answer"))
                    })
                }
                _ => {}
            }
            self.changed(&mut state, deadline).await?;
        }
    }
    /// ICE/DTLS connection is separate from successful media delivery.
    pub async fn wait_connected(&self, deadline: Instant) -> Result<()> {
        let mut state = self.state.clone();
        loop {
            let current = state.borrow().clone();
            match current {
                State::Connected(_) => {
                    if self.stop.is_cancelled() {
                        return Err(Error::forbidden());
                    }
                    original_sdp(&self.store, &self.origin)?;
                    return Ok(());
                }
                State::Closed(result) => {
                    return result.and_then(|_| {
                        Err(Error::new("closed", "WebRTC peer closed before connection"))
                    })
                }
                _ => {}
            }
            self.changed(&mut state, deadline).await?;
        }
    }
    async fn changed(&self, state: &mut watch::Receiver<State>, deadline: Instant) -> Result<()> {
        tokio::select! {
            biased;
            _=self.stop.cancelled()=>Err(Error::new("closed","WebRTC peer retired")),
            _=tokio::time::sleep_until(deadline)=>Err(Error::new("timeout","WebRTC observation deadline")),
            changed=state.changed()=>changed.map_err(|_|Error::new("cleanup-unconfirmed","WebRTC worker state unavailable")),
        }
    }
    /// Await the original close worker. Cancellation of one wait cannot take
    /// its join handle or release the capacity permit before actual close.
    pub async fn shutdown(&self) -> Result<()> {
        self.stop.cancel();
        self.completion.clone().await
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

fn original_sdp(store: &Store, origin: &NativeMediaOrigin) -> Result<String> {
    let read = store.try_native_media_read(origin, None)?;
    let target = read.target();
    let offer = &target.offer;
    if !read.current()
        || target.endpoint_transport != EndpointTransport::Webrtc
        || offer["substrate"] != "websocket+webrtc"
        || offer["substrate_setup"]["kind"] != "websocket+webrtc"
    {
        return Err(Error::forbidden());
    }
    let sdp = offer["substrate_setup"]["sdp"]
        .as_str()
        .ok_or_else(Error::forbidden)?;
    let parsed = rvoip_sip_core::types::sdp::SdpSession::from_str(sdp)
        .map_err(|_| Error::new("invalid-request", "WebRTC SDP cannot be parsed"))?;
    let mut offered = BTreeMap::<&str, usize>::new();
    for stream in offer["streams_offered"]
        .as_array()
        .ok_or_else(Error::forbidden)?
    {
        let kind = match stream["kind"].as_str() {
            Some("audio") => "audio",
            Some("video" | "screen") => "video",
            Some("data") => "application",
            _ => return Err(Error::forbidden()),
        };
        *offered.entry(kind).or_default() += 1;
    }
    let mut actual = BTreeMap::<&str, usize>::new();
    for media in &parsed.media_descriptions {
        if media.port == 0 {
            continue;
        }
        let kind = match media.media.as_str() {
            "audio" => "audio",
            "video" => "video",
            "application" => "application",
            _ => {
                return Err(Error::new(
                    "unsupported-media-profile",
                    "WebRTC SDP media kind is unsupported",
                ))
            }
        };
        *actual.entry(kind).or_default() += 1;
    }
    // Multiple logical SCTP channels share one application m-line.
    if let Some(data) = offered.get_mut("application") {
        *data = 1;
    }
    if offered != actual || actual.is_empty() || parsed.media_descriptions.len() > 32 {
        return Err(Error::new(
            "incompatible-capabilities",
            "WebRTC SDP media differs from original stream intent",
        ));
    }
    Ok(sdp.to_owned())
}
fn engine_error() -> Error {
    Error::new("webrtc-failed", "WebRTC peer setup or transport failed")
}
#[allow(clippy::too_many_arguments)]
async fn worker(
    store: Arc<Store>,
    origin: Arc<NativeMediaOrigin>,
    attempt: Arc<NativeOfferLease>,
    config: WebRtcConfig,
    stop: CancellationToken,
    _permit: OwnedSemaphorePermit,
    deadline: Instant,
    state: watch::Sender<State>,
    #[cfg(test)] close_gate: CloseGateSlot,
) -> Result<()> {
    let remaining = attempt.expires_at() - chrono::Utc::now().timestamp();
    let lease_deadline = Instant::now() + Duration::from_secs(remaining.max(0) as u64);
    let original_deadline = deadline;
    let deadline = deadline.min(lease_deadline);
    let mut allocated = None;
    let result: Result<()> = async {
        if origin.offer_record_id()!=attempt.record_id()||origin.connection_id()!=attempt.connection_id()||origin.runtime_connection_id()!=attempt.runtime_connection_id()||origin.generation()!=attempt.generation(){return Err(Error::forbidden());}
        if original_deadline<=Instant::now()||original_deadline>Instant::now()+Duration::from_secs(30){return Err(Error::new("invalid-request","WebRTC preparation requires a bounded original deadline"));}
        if stop.is_cancelled(){return Err(Error::new("cancelled","WebRTC preparation cancelled"));}
        let sdp=original_sdp(&store,&origin)?;
        // Keep allocation owned even when the preparing caller goes away.
        let peer=RvoipPeerConnection::new(&config,PeerRole::Answerer).await.map_err(|_|engine_error())?;allocated=Some(peer.clone());
        if stop.is_cancelled(){return Err(Error::new("cancelled","WebRTC preparation cancelled"));}
        let answer=tokio::select! {
            biased;
            _=stop.cancelled()=>return Err(Error::new("cancelled","WebRTC preparation cancelled")),
            _=tokio::time::sleep_until(deadline)=>return Err(Error::new("timeout","WebRTC original preparation deadline")),
            answer=peer.accept_offer_and_gather(&sdp)=>answer.map_err(|_|engine_error())?,
        };
        if !answer.starts_with("v=0\r\n") || answer.len()>512*1024 || answer.contains('\0') { return Err(engine_error()); }
        original_sdp(&store,&origin)?;
        if stop.is_cancelled(){return Err(Error::new("cancelled","WebRTC preparation cancelled"));}
        state.send_replace(State::Answered(answer.clone()));
        let mut interval=tokio::time::interval(Duration::from_millis(50));interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                _=stop.cancelled()=>return Ok(()),
                _=peer.wait_failed()=>return Err(engine_error()),
                _=interval.tick()=>{
                    // No domain reader is retained across this idle wait.
                    match store.try_native_connection_current(&origin) {
                        Ok(true)=>{},
                        Err(error) if matches!(error.code.as_str(),"authority-busy"|"storage-unavailable")=>continue,
                        _=>return Err(Error::forbidden()),
                    }
                    if peer.is_connected(){state.send_if_modified(|s|{if matches!(s,State::Connected(_)){false}else{*s=State::Connected(answer.clone());true}});}
                }
            }
        }
    }
    .await;
    #[cfg(test)]
    {
        let gate = close_gate.lock().ok().and_then(|gate| gate.clone());
        if let Some(gate) = gate {
            gate.entered.notify_one();
            gate.release.notified().await;
        }
    }
    let cleanup = match allocated {
        Some(peer) => peer
            .close()
            .await
            .map_err(|_| Error::new("cleanup-unconfirmed", "WebRTC peer close failed")),
        None => Ok(()),
    };
    state.send_replace(State::Closed(cleanup.clone().and(result)));
    // Setup/authority refusal and actual close confirmation remain distinct.
    cleanup
}
#[cfg(test)]
mod tests;
