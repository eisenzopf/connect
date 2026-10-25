//! Owned canonical TLS WebSocket intake. Retirement controls never authorize IO.
use super::canonical_websocket::CanonicalWebSocketConfig;
use anyhow::{Context, Result};
use connect_service::Service;
use rvoip_uctp::v2::server::{CanonicalProfile, ProfileServer};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::mpsc, task::JoinHandle};
use tokio_util::sync::CancellationToken;

pub(crate) struct Listener {
    pub(crate) address: SocketAddr,
    pub(crate) contact: String,
    peers: ProfileServer,
    stop: CancellationToken,
    task: Option<JoinHandle<Result<()>>>,
}
impl Listener {
    pub(crate) async fn start(
        config: &CanonicalWebSocketConfig,
        tls: Arc<rustls::ServerConfig>,
        profile: CanonicalProfile,
        parent: CancellationToken,
        key_deadline: tokio::time::Instant,
        service: &Arc<Service>,
        stop_other_transports: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self> {
        let listener = TcpListener::bind(config.bind)
            .await
            .context("Bind canonical WSS")?;
        let address = listener.local_addr()?;
        let contact = config.contact(address);
        let (sender, receiver) = mpsc::channel(profile.max_connections());
        let peers = rvoip_websocket::canonical::start(
            receiver,
            tls,
            profile,
            contact.clone(),
            config.mount_path.clone(),
            Some(config.allowed_origins.iter().cloned().collect()),
        )
        .map_err(|_| anyhow::anyhow!("Cannot start canonical WSS supervisor"))?;
        let stop = parent.child_token();
        let cancelled = stop.clone();
        let owned_peers = peers.clone();
        let task = crate::worker::spawn(
            service.readiness.clone(),
            "canonical-wss-intake",
            stop.clone(),
            async move {
                struct Owned(ProfileServer, Arc<dyn Fn() + Send + Sync>);
                impl Drop for Owned {
                    fn drop(&mut self) {
                        self.0.begin_shutdown();
                        // Do not mark an unexpected failure as requested stop:
                        // the callback retires linked transports, not this worker's token.
                        (self.1)();
                    }
                }
                let _owned = Owned(owned_peers, stop_other_transports);
                loop {
                    tokio::select! {biased;
                        _=cancelled.cancelled()=>break,
                        _=tokio::time::sleep_until(key_deadline)=>anyhow::bail!("Canonical signing key expired"),
                        _=sender.closed()=>anyhow::bail!("Canonical WSS supervisor stopped"),
                        incoming=listener.accept()=>{
                            let (socket,_) = incoming.context("Canonical WSS accept")?;
                            // Excess raw sockets are dropped before TLS/authentication.
                            // The shared physical factory additionally bounds live canonical peers.
                            let _ = sender.try_send(socket);
                        },
                    }
                }
                drop(listener);
                drop(sender);
                Ok(())
            },
        );
        Ok(Self {
            address,
            contact,
            peers,
            stop,
            task: Some(task),
        })
    }
    pub(crate) fn control(&self) -> ProfileServer {
        self.peers.clone()
    }
    pub(crate) fn begin_shutdown(&self) {
        self.stop.cancel();
        self.peers.begin_shutdown();
    }
    pub(crate) async fn shutdown(mut self) -> Result<()> {
        self.begin_shutdown();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let peer_result = tokio::time::timeout_at(deadline, self.peers.shutdown())
            .await
            .context("Canonical WSS peer drain exceeded five seconds")?;
        let task = self.task.as_mut().expect("Owned canonical WSS intake");
        let intake = tokio::time::timeout_at(deadline, task).await;
        match intake {
            Ok(result) => {
                self.task.take();
                result.context("Canonical WSS intake task")??;
            }
            Err(_) => anyhow::bail!("Canonical WSS intake drain exceeded five seconds"),
        }
        peer_result.map_err(|_| anyhow::anyhow!("Canonical WSS peer drain failed"))
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        self.begin_shutdown();
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
