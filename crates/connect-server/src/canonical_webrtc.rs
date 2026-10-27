//! Owned WebRTC transport preparation for one durable Connection attempt.
//! Actual RTC allocation requires the exact prior Core reservation.
use connect_protocol::{provisioning::EndpointTransport, Error, Result};
use connect_store::{NativeMediaOrigin, NativeMediaRead, NativeOfferLease, Store};
use futures_util::{future::BoxFuture, FutureExt};
use rvoip_webrtc::{canonical::WebRtcPhysicalBinding, peer::PeerRole, WebRtcConfig};
use std::{
    collections::BTreeMap,
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::watch, time::Instant};
use tokio_util::sync::CancellationToken;
mod signaling;
pub use signaling::SignalingHost;
struct ReadContext {
    initial: Mutex<Option<Arc<NativeMediaRead>>>,
    #[cfg(test)]
    busy_reads: std::sync::atomic::AtomicUsize,
}
impl ReadContext {
    fn new(initial: Option<Arc<NativeMediaRead>>) -> Arc<Self> {
        Arc::new(Self {
            initial: Mutex::new(initial),
            #[cfg(test)]
            busy_reads: std::sync::atomic::AtomicUsize::new(0),
        })
    }
}
type InitialRead = Arc<ReadContext>;
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
    initial: InitialRead,
    #[cfg(test)]
    close_gate: CloseGateSlot,
}
impl Peer {
    #[allow(clippy::too_many_arguments)]
    fn start_scoped(
        store: Arc<Store>,
        origin: Arc<NativeMediaOrigin>,
        attempt: NativeOfferLease,
        config: WebRtcConfig,
        physical: &CancellationToken,
        binding: Arc<WebRtcPhysicalBinding>,
        deadline: Instant,
        initial: InitialRead,
        #[cfg(test)] reservation: Option<tests::reservation::Reservation>,
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
            binding,
            deadline,
            state_tx,
            initial.clone(),
            #[cfg(test)]
            close_gate.clone(),
            #[cfg(test)]
            reservation,
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
            initial,
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
                    bounded_read(&self.stop, deadline, || {
                        original_sdp(&self.store, &self.origin, &self.initial)
                    })
                    .await?;
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
                    bounded_read(&self.stop, deadline, || {
                        original_sdp(&self.store, &self.origin, &self.initial)
                    })
                    .await?;
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
        self.release_invocation();
        self.stop.cancel();
        self.completion.clone().await
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.release_invocation();
        self.stop.cancel();
    }
}

impl Peer {
    fn release_invocation(&self) {
        if let Ok(mut initial) = self.initial.initial.lock() {
            initial.take();
        }
    }
    async fn source_read(&self, deadline: Instant) -> Result<NativeMediaRead> {
        bounded_read(&self.stop, deadline, || {
            source_read(&self.store, &self.origin, &self.initial)
        })
        .await
    }
}
fn source_read(
    store: &Store,
    origin: &NativeMediaOrigin,
    initial: &InitialRead,
) -> Result<NativeMediaRead> {
    #[cfg(test)]
    if initial
        .busy_reads
        .fetch_update(
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
            |n| n.checked_sub(1),
        )
        .is_ok()
    {
        return Err(Error::new(
            "authority-busy",
            "Controlled bounded metadata contention",
        ));
    }
    let held = initial
        .initial
        .lock()
        .map_err(|_| Error::storage())?
        .clone();
    match held {
        Some(held) => store.try_native_media_source_read_under(origin, &held),
        None => store.try_native_media_read(origin, None),
    }
}
fn original_sdp(
    store: &Store,
    origin: &NativeMediaOrigin,
    initial: &InitialRead,
) -> Result<String> {
    let read = source_read(store, origin, initial)?;
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
/// Retry metadata observation only, never peer allocation or SDP generation.
/// No reader survives a retry wait, and unavailable metadata grants no IO.
async fn bounded_read<T>(
    stop: &CancellationToken,
    deadline: Instant,
    mut read: impl FnMut() -> Result<T>,
) -> Result<T> {
    loop {
        if stop.is_cancelled() {
            return Err(Error::new("closed", "WebRTC peer retired"));
        }
        if Instant::now() >= deadline {
            return Err(Error::new("timeout", "WebRTC metadata deadline"));
        }
        match read() {
            Err(error)
                if matches!(
                    error.code.as_str(),
                    "authority-busy" | "storage-unavailable"
                ) =>
            {
                tokio::select! {
                    biased;
                    _ = stop.cancelled() => return Err(Error::new("closed", "WebRTC peer retired")),
                    _ = tokio::time::sleep_until(deadline) => return Err(Error::new("timeout", "WebRTC metadata deadline")),
                    _ = tokio::time::sleep(Duration::from_millis(5)) => {},
                }
            }
            result => return result,
        }
    }
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
    binding: Arc<WebRtcPhysicalBinding>,
    deadline: Instant,
    state: watch::Sender<State>,
    initial: InitialRead,
    #[cfg(test)] close_gate: CloseGateSlot,
    #[cfg(test)] reservation: Option<tests::reservation::Reservation>,
) -> Result<()> {
    let remaining = attempt.expires_at() - chrono::Utc::now().timestamp();
    let lease_deadline = Instant::now() + Duration::from_secs(remaining.max(0) as u64);
    let original_deadline = deadline;
    let deadline = deadline.min(lease_deadline);
    let result: Result<()> = async {
        if origin.offer_record_id()!=attempt.record_id()||origin.connection_id()!=attempt.connection_id()||origin.runtime_connection_id()!=attempt.runtime_connection_id()||origin.generation()!=attempt.generation(){return Err(Error::forbidden());}
        if original_deadline<=Instant::now()||original_deadline>Instant::now()+Duration::from_secs(30){return Err(Error::new("invalid-request","WebRTC preparation requires a bounded original deadline"));}
        if stop.is_cancelled(){return Err(Error::new("cancelled","WebRTC preparation cancelled"));}
        if config.trickle_ice {return Err(Error::new("unsupported-media-profile","Canonical WebRTC preparation requires complete ICE in SDP"));}
        let sdp=bounded_read(&stop,deadline,||original_sdp(&store,&origin,&initial)).await?;
        // Keep allocation owned even when the preparing caller goes away.
        binding.start_peer(config.clone(),PeerRole::Answerer).map_err(|_|engine_error())?;
        let peer=binding.peer(deadline).await.map_err(|_|engine_error())?;
        if stop.is_cancelled(){return Err(Error::new("cancelled","WebRTC preparation cancelled"));}
        let answer=tokio::select! {
            biased;
            _=stop.cancelled()=>return Err(Error::new("cancelled","WebRTC preparation cancelled")),
            _=tokio::time::sleep_until(deadline)=>return Err(Error::new("timeout","WebRTC original preparation deadline")),
            answer=peer.accept_offer_and_gather(&sdp)=>answer.map_err(|_|engine_error())?,
        };
        if !answer.starts_with("v=0\r\n") || answer.len()>512*1024 || answer.contains('\0') { return Err(engine_error()); }
        bounded_read(&stop,deadline,||original_sdp(&store,&origin,&initial)).await?;
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
    let cleanup = binding.shutdown().await.map_err(|_| {
        Error::new(
            "cleanup-unconfirmed",
            "WebRTC original binding close failed",
        )
    });
    #[cfg(test)]
    let cleanup = match reservation {
        Some(reservation) => cleanup.and(reservation.close().await),
        None => cleanup,
    };
    state.send_replace(State::Closed(cleanup.clone().and(result)));
    // Setup/authority refusal and actual close confirmation remain distinct.
    cleanup
}
#[cfg(test)]
mod tests;
