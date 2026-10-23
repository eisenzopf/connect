//! Server-owned fresh private Vapi call and its signed native audio forwarder.
//! Provider proposals never dispatch Core effects. Application prompts are explicit.
use connect_client::canonical::{
    CanonicalClient, CanonicalClientConfig, CommandScope, PrivateNativeAudio,
};
use connect_protocol::{transitions::TransitionCommand, Error, Result};
use connect_service::Service;
use connect_store::{
    PrivateProviderApplied, PrivateProviderContext, PrivateProviderCreated, Store,
};
use futures_util::{SinkExt, StreamExt};
use reqwest::Url;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio::{net::TcpStream, sync::watch, task::JoinHandle};
use tokio_tungstenite::{
    tungstenite::{client::IntoClientRequest, Message},
    MaybeTlsStream, WebSocketStream,
};
use tokio_util::sync::CancellationToken;
mod audio;
pub(crate) use audio::AudioQueue;
mod coordinator;
pub use coordinator::{PrivateVapiCoordinator, PrivateVapiTarget};
mod transcripts;
mod transport;
use transport::*;
pub(crate) use transport::{creation_response, socket_with_limit, terminate};

/// Operator-selected transport and application prompt, never model arguments.
/// Plain HTTP/WS is limited to explicitly selected numeric loopback fixtures.
#[derive(Clone)]
pub struct PrivateVapiProfile {
    pub api_base: Url,
    pub websocket_origin: Url,
    pub loopback_fixture: bool,
    pub model: String,
    pub application_prompt: String,
    pub observations: Option<crate::agent_observations::AgentObservationConfig>,
}
/// The server must retain this owned job and drain it on shutdown. Dropping the
/// handle cancels the job; its task retains captured receipts through cleanup.
pub struct PrivateVapiBridge {
    stop: CancellationToken,
    task: Option<JoinHandle<Result<()>>>,
    status: watch::Receiver<PrivateVapiStatus>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrivateVapiStatus {
    Preparing,
    ContextReady,
    Stopped { problem: Option<String> },
}
/// Explicit server-only composition. These are existing protected credential IDs
/// and a configured AI peer. No credential/key is exposed to provider tools.
pub struct PrivateVapiInvocation {
    pub command: TransitionCommand,
    pub execution_credential: String,
    pub context_credential: String,
    pub native_peer: CanonicalClientConfig,
    pub native_endpoint: String,
    pub provider_endpoint: String,
}
pub type ProviderSecrets = Arc<dyn Fn(&str) -> Result<crate::secrets::SecretValue> + Send + Sync>;
impl PrivateVapiBridge {
    pub async fn start(
        service: Arc<Service>,
        invocation: PrivateVapiInvocation,
        profile: PrivateVapiProfile,
        secrets: ProviderSecrets,
    ) -> Result<Self> {
        Self::start_with_stop(
            service,
            invocation,
            profile,
            secrets,
            CancellationToken::new(),
        )
        .await
    }
    async fn start_with_stop(
        service: Arc<Service>,
        invocation: PrivateVapiInvocation,
        profile: PrivateVapiProfile,
        secrets: ProviderSecrets,
        stop: CancellationToken,
    ) -> Result<Self> {
        profile.validate()?;
        if stop.is_cancelled() {
            return Err(problem("cancelled"));
        }
        // Match the full original intent before reserving any new physical IO.
        let s = service.store.clone();
        let key = invocation.execution_credential.clone();
        let command = invocation.command.clone();
        let origin = store_call(move || s.native_consultation_origin(&key, command))
            .await
            .map_err(|error| phase_error("origin", error))?;
        let token = stop.clone();
        let (tx, status) = watch::channel(PrivateVapiStatus::Preparing);
        let task = tokio::spawn(async move {
            let result = run(service, invocation, profile, secrets, origin, token, &tx).await;
            tx.send_replace(PrivateVapiStatus::Stopped {
                problem: result.as_ref().err().map(|e| e.code.clone()),
            });
            result
        });
        Ok(Self {
            stop,
            task: Some(task),
            status,
        })
    }
    pub fn status(&self) -> PrivateVapiStatus {
        self.status.borrow().clone()
    }
    pub async fn context_ready(&mut self) -> Result<()> {
        loop {
            match self.status.borrow().clone() {
                PrivateVapiStatus::ContextReady => return Ok(()),
                PrivateVapiStatus::Stopped { problem } => {
                    return Err(Error::new(
                        problem.as_deref().unwrap_or("gone"),
                        "Private provider initialization stopped",
                    ))
                }
                _ => {}
            }
            self.status
                .changed()
                .await
                .map_err(|_| Error::forbidden())?;
        }
    }
    pub async fn shutdown(self) -> Result<()> {
        self.stop.cancel();
        self.finish().await
    }
    async fn finish(mut self) -> Result<()> {
        let Some(task) = self.task.take() else {
            return Ok(());
        };
        task.await.map_err(|_| Error::storage())?
    }
}
impl Drop for PrivateVapiBridge {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

async fn store_call<T: Send + 'static>(
    f: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| Error::storage())?
}
async fn preflight<T, F, Fut>(mut work: F, stop: &CancellationToken) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match work().await {
            Err(e)
                if matches!(e.code.as_str(), "authority-busy" | "storage-unavailable")
                    && tokio::time::Instant::now() < deadline =>
            {
                tokio::select! {_=stop.cancelled()=>return Err(e),_=tokio::time::sleep(Duration::from_millis(20))=>{}}
            }
            result => return result,
        }
    }
}
/// Observation retry never reacquires an IO permit or repeats a network write.
async fn record(
    store: &Arc<Store>,
    f: impl Fn(&Store) -> Result<()> + Send + Sync + 'static,
    stop: &CancellationToken,
) -> Result<()> {
    let f = Arc::new(f);
    loop {
        let work = f.clone();
        let s = store.clone();
        match store_call(move || work(&s)).await {
            Ok(()) => return Ok(()),
            Err(e) if matches!(e.code.as_str(), "authority-busy" | "storage-unavailable") => {
                tokio::select! {_ = stop.cancelled()=>return Err(e),_=tokio::time::sleep(Duration::from_millis(20))=>{}}
            }
            Err(e) => return Err(e),
        }
    }
}
async fn run(
    service: Arc<Service>,
    i: PrivateVapiInvocation,
    profile: PrivateVapiProfile,
    secrets: ProviderSecrets,
    origin: connect_store::NativeConsultationOrigin,
    stop: CancellationToken,
    status: &watch::Sender<PrivateVapiStatus>,
) -> Result<()> {
    let record_id = origin.record_id().to_owned();
    let connid = format!("pvc_{record_id}");
    let opid = format!("pvo_{record_id}");
    let capabilities = json!({"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":1,"identity_assurance_offered":"identified"});
    let offer = json!({"substrate":"quic","capabilities":capabilities,"private_consultation":{"profile":"connect-private-audio-v1","operation_record_id":record_id},"streams_offered":[{"id":"private-provider-audio","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]});
    let scope = CommandScope {
        conversation_id: Some(i.command.conversation_id.clone()),
        session_id: Some(i.command.session_id.clone()),
        connection_id: Some(connid.clone()),
        operation_id: Some(opid.clone()),
        expected_revision: Some(0),
    };
    let intent = connect_protocol::connection_control::OfferIntent {
        operation_id: opid,
        conversation_id: i.command.conversation_id.clone(),
        session_id: i.command.session_id.clone(),
        connection_id: connid,
        expected_revision: Some(0),
        by_participant: None,
        offer: offer.clone(),
    };
    let store = service.store.clone();
    let s = store.clone();
    let endpoint = i.native_endpoint.clone();
    store_call(move || s.register_native_consultation_leg(&origin, intent, &endpoint))
        .await
        .map_err(|error| phase_error("native-leg", error))?;
    let mut native = tokio::select! {
        _=stop.cancelled()=>return Err(problem("cancelled")),
        value=async {
            let client=CanonicalClient::open(i.native_peer).await.map_err(|_|problem("private-native-peer-failed"))?;
            client.prepare_private_native_audio(scope,&record_id,offer).await.map_err(|_|problem("private-native-preparation-failed"))
        }=>value?,
    };
    let svc = service.clone();
    let execution = i.execution_credential.clone();
    let command = i.command.clone();
    let context_key = i.context_credential.clone();
    let endpoint = i.provider_endpoint.clone();
    let context = preflight(
        || {
            let svc = svc.clone();
            let execution = execution.clone();
            let command = command.clone();
            let context_key = context_key.clone();
            let endpoint = endpoint.clone();
            store_call(move || {
                svc.prepare_private_provider_context(&execution, command, &context_key, &endpoint)
            })
        },
        &stop,
    )
    .await?;
    let initialized = initialize(&store, &context, &profile, &secrets, &stop)
        .await
        .map_err(|error| phase_error("provider-initialize", error));
    let (mut socket, receipt, created) = match initialized {
        Ok(v) => v,
        Err(e) => {
            native.close();
            return Err(e);
        }
    };
    // This is a scheduling hint; the existing native coordinator still requires
    // fresh scene authority, one possible submission and actual decoded audio.
    let resume = if stop.is_cancelled() {
        Err(problem("cancelled"))
    } else {
        store_call(move || svc.resume_private_consultation(&i.execution_credential, i.command))
            .await
            .map_err(|error| phase_error("resume", error))
    };
    let result = match resume {
        Ok(()) => {
            let observed = async {
                let mut capture = crate::agent_observations::Capture::private(
                    &service,
                    profile.observations.as_ref(),
                    &context,
                    &receipt,
                )
                .await?;
                status.send_replace(PrivateVapiStatus::ContextReady);
                let result = forward(
                    &store,
                    &context,
                    &receipt,
                    &mut native,
                    &mut socket,
                    &stop,
                    &mut capture,
                )
                .await;
                if let Some(capture) = capture {
                    capture.finish().await;
                }
                result
            }
            .await;
            observed
        }
        Err(e) => Err(e),
    };
    native.close();
    // A failed/timed-out send may retain private bytes in the socket sink.
    // Dropping that transport must precede releasing custody; teardown must
    // never flush such bytes after the media grant has ended.
    let may_flush = !result.as_ref().err().is_some_and(|e| {
        matches!(
            e.code.as_str(),
            "provider-media-timeout" | "provider-media-ended"
        )
    });
    let observed = if result
        .as_ref()
        .is_ok_and(|end| *end == MediaEnd::ProviderEnded)
    {
        true
    } else if may_flush {
        terminate(&mut socket, receipt.provider_ref()).await
    } else {
        drop(socket);
        false
    };
    if observed {
        let captured = receipt.clone();
        record(
            &store,
            move |s| s.record_private_provider_closed(&captured),
            &stop,
        )
        .await?;
    }
    // The actual created receipt remains owned until all initialization/media
    // and termination observations above have settled. No reconstructed call.
    drop(created);
    let _ = native.shutdown().await;
    result.map(|_| ())
}
fn phase_error(phase: &'static str, error: Error) -> Error {
    tracing::debug!(target: "connect_server::private_vapi", phase, code=%error.code, "Private initialization refused");
    error
}
fn problem(code: &str) -> Error {
    Error::new(code, "Private provider operation could not be confirmed")
}
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
