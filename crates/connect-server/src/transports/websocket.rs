//! Loopback-only WS parity entry point. rvoip owns the UCTP coordinator;
//! this module owns bounded framing and cancellable local listener lifetime.
use anyhow::Result;
use connect_service::{Service, StoreValidator};
use futures_util::{SinkExt, StreamExt};
use rvoip_uctp::{
    envelope::UctpEnvelope,
    state::{self, UctpCoordinator, UctpCoordinatorCaps, ENVELOPE_CHANNEL_CAP},
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{mpsc, Semaphore},
    task::{JoinHandle, JoinSet},
};
use tokio_tungstenite::tungstenite::{protocol::WebSocketConfig, Message};
use tokio_util::sync::CancellationToken;

pub(crate) struct Listener {
    pub address: SocketAddr,
    cancel: CancellationToken,
    task: Option<JoinHandle<()>>,
    output_high_water: Arc<AtomicUsize>,
}
impl Listener {
    pub async fn bind(
        address: SocketAddr,
        service: Arc<Service>,
        validator: Arc<StoreValidator>,
        connections: usize,
        deadline_ms: u64,
    ) -> Result<Self> {
        anyhow::ensure!(
            address.ip().is_loopback(),
            "WS parity listener requires loopback"
        );
        let listener = TcpListener::bind(address).await?;
        let address = listener.local_addr()?;
        let cancel = CancellationToken::new();
        let stopping = cancel.clone();
        let output_high_water = Arc::new(AtomicUsize::new(0));
        let high_water = output_high_water.clone();
        let task = tokio::spawn(async move {
            let slots = Arc::new(Semaphore::new(connections));
            let mut peers = JoinSet::new();
            loop {
                tokio::select! {
                    _=stopping.cancelled()=>break,
                    _=peers.join_next(),if !peers.is_empty()=>{},
                    accepted=listener.accept()=>{
                        let Ok((tcp,_))=accepted else {break};
                        let Ok(permit)=slots.clone().try_acquire_owned() else {continue};
                        let service=service.clone();let validator=validator.clone();let stop=stopping.clone();
                        let high_water=high_water.clone();
                        peers.spawn(async move {let _permit=permit;peer(tcp,service,validator,stop,deadline_ms,high_water).await;});
                    }
                }
            }
            drop(listener); // release the port before joining peer drains
            while peers.join_next().await.is_some() {}
        });
        Ok(Self {
            address,
            cancel,
            task: Some(task),
            output_high_water,
        })
    }
    pub fn output_high_water(&self) -> usize {
        self.output_high_water.load(Ordering::Relaxed)
    }
    pub async fn shutdown(mut self) -> Result<()> {
        self.cancel.cancel();
        if let Some(mut task) = self.task.take() {
            if tokio::time::timeout(Duration::from_secs(5), &mut task)
                .await
                .is_err()
            {
                task.abort();
                let _ = task.await;
                anyhow::bail!("WS parity shutdown exceeded 5 seconds");
            }
        }
        Ok(())
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

async fn peer(
    tcp: TcpStream,
    service: Arc<Service>,
    validator: Arc<StoreValidator>,
    stop: CancellationToken,
    deadline_ms: u64,
    high_water: Arc<AtomicUsize>,
) {
    let caps = UctpCoordinatorCaps {
        signaling_send_timeout: Duration::from_millis(deadline_ms),
        ..UctpCoordinatorCaps::default()
    };
    let auth_deadline = caps.authentication_deadline;
    // Bound the frame before JSON decoding. Matches UCTP's default 1 MiB
    // envelope bound, allowing escaped UTF-8 bodies without a tighter WS cap.
    let config = WebSocketConfig::default()
        .max_message_size(Some(1024 * 1024))
        .max_frame_size(Some(1024 * 1024));
    let upgrade = tokio_tungstenite::accept_async_with_config(tcp, Some(config));
    let ws = tokio::select! {
        result=tokio::time::timeout(auth_deadline,upgrade)=>match result {Ok(Ok(ws))=>ws,_=>return},
        _=stop.cancelled()=>return,
    };
    let (mut sink, mut stream) = ws.split();
    let (in_tx, in_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let (events_tx, mut events_rx) = mpsc::channel(ENVELOPE_CHANNEL_CAP);
    let coordinator = UctpCoordinator::start_full_with_caps(
        "websocket",
        in_rx,
        out_tx,
        events_tx,
        validator,
        Arc::new(state::default_v0_descriptor()),
        state::rejecting_handler(),
        caps,
    );
    coordinator.set_application_handler(service);
    let guard = state::spawn_auth_lifecycle_guard(coordinator.clone(), auth_deadline);
    let incoming = tokio::spawn(async move {
        loop {
            let next = tokio::select! {_=stop.cancelled()=>return,result=stream.next()=>result};
            match next {
                Some(Ok(Message::Text(text))) => {
                    let Ok(envelope) = serde_json::from_str::<UctpEnvelope>(&text) else {
                        return;
                    };
                    // Capacity waits must observe listener shutdown too. A
                    // pending long inbox read may otherwise keep this pump
                    // alive after cancellation, delaying peer supervision.
                    tokio::select! {
                        _=stop.cancelled()=>return,
                        result=in_tx.send(envelope)=>if result.is_err(){return},
                    }
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                _ => return,
            }
        }
    });
    let outgoing = tokio::spawn(async move {
        while let Some(envelope) = out_rx.recv().await {
            high_water.fetch_max(out_rx.len(), Ordering::Relaxed);
            let Ok(text) = serde_json::to_string(&envelope) else {
                return;
            };
            if !matches!(
                tokio::time::timeout(
                    Duration::from_millis(deadline_ms),
                    sink.send(Message::Text(text.into()))
                )
                .await,
                Ok(Ok(()))
            ) {
                high_water.fetch_max(out_rx.len(), Ordering::Relaxed);
                return;
            }
        }
        let _ = sink.close().await;
    });
    // This fixture has no live media projection. Consume coordinator lifecycle
    // events so authenticated application traffic cannot stall on an unused queue.
    let events = tokio::spawn(async move { while events_rx.recv().await.is_some() {} });
    state::supervise_peer_tasks(
        coordinator,
        vec![incoming, outgoing, events, guard],
        Duration::from_secs(2),
    )
    .await;
}
