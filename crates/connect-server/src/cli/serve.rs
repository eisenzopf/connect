//! Shared ready reporting and shutdown signals for local server commands.
use crate::runtime::RunningServer;
use anyhow::Result;

/// Report bound interfaces without credentials or private state.
pub fn report(server: &RunningServer) {
    if let Some(address) = server.webhook_address {
        println!("webhook_only={address}");
    }
    if let Some(address) = server.http_address {
        println!("http_json={address}");
    }
    if let Some(address) = server.mcp_address {
        println!("mcp_streamable_http=http://{address}/mcp");
    }
    println!(
        "ready transport=quic alpn={} profile={} address={} schema={} websocket_parity={}",
        rvoip_uctp::UCTP_RAW_QUIC_ALPN,
        connect_protocol::PROFILE,
        server.address,
        connect_store::SCHEMA_VERSION,
        server
            .websocket_address
            .map(|a| a.to_string())
            .unwrap_or_else(|| "disabled".into())
    );
}

/// Install Unix shutdown handlers before reporting ready. Keeping the handles
/// across setup and serving makes cancellation safe at either boundary.
pub(crate) struct Shutdown {
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
}
impl Shutdown {
    pub(crate) fn install() -> Result<Self> {
        Ok(Self {
            #[cfg(unix)]
            interrupt: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?,
            #[cfg(unix)]
            terminate: tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?,
        })
    }
    pub(crate) async fn wait(&mut self, server: &RunningServer) -> Result<()> {
        #[cfg(unix)]
        {
            tokio::select! {
                _ = self.interrupt.recv() => {},
                _ = self.terminate.recv() => {},
                _ = server.service.readiness.failed() => {
                    tracing::error!("Worker failed; draining server without retrying uncertain work");
                }
            }
        }
        #[cfg(not(unix))]
        {
            tokio::select! {
                result = tokio::signal::ctrl_c() => result?,
                _ = server.service.readiness.failed() => {}
            }
        }
        Ok(())
    }
}

/// Serve until shutdown and drain even when registering signal handlers fails.
pub async fn run(server: RunningServer) -> Result<()> {
    let result = async {
        let mut shutdown = Shutdown::install()?;
        report(&server);
        shutdown.wait(&server).await
    }
    .await;
    let shutdown = server.shutdown().await;
    result.and(shutdown)?;
    println!("stopped");
    Ok(())
}

/// Run the signed canonical listener with owned shutdown on signals/failure.
pub async fn run_canonical(server: crate::canonical::RunningCanonicalServer) -> Result<()> {
    let result = async {
        let mut shutdown = Shutdown::install()?;
        println!("ready transport=quic alpn={} profile=connect/2 address={} schema={}",
            std::str::from_utf8(rvoip_uctp::v2::RAW_QUIC_ALPN).unwrap_or("invalid"), server.address, connect_store::SCHEMA_VERSION);
        if let Some(address)=server.http_address { println!("http_json={address}"); }
        if let Some(address)=server.mcp_address { println!("mcp_streamable_http=http://{address}/mcp"); }
        #[cfg(unix)]
        tokio::select! { _=shutdown.interrupt.recv()=>{}, _=shutdown.terminate.recv()=>{}, _=server.service.readiness.failed()=>{} }
        #[cfg(not(unix))]
        tokio::select! { result=tokio::signal::ctrl_c()=>result?, _=server.service.readiness.failed()=>{} }
        Ok(())
    }.await;
    let stopped = server.shutdown().await;
    result.and(stopped)?;
    println!("stopped");
    Ok(())
}
