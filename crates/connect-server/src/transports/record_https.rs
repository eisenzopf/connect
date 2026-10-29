//! Explicit pinned TLS endpoint for ephemeral record references. A locator
//! selects metadata; every GET derives new current bearer/source authority.
use super::http_record_io::{Custody, GuardedListener};
use axum::{
    extract::{ConnectInfo, Path, State},
    http::{header, HeaderMap, StatusCode, Version},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use connect_protocol::{
    canonical_http::{Problem, Rejection, PROFILE},
    Error,
};
use connect_service::{Service, StoreCredentialBinding, StoreValidator};
use rvoip_auth_core::BearerValidator;
use rvoip_uctp::v2::recording::RecordReferenceRegistry;
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordHttpsConfig {
    pub bind: SocketAddr,
    pub server_name: String,
    pub capacity: usize,
    pub ttl_seconds: u32,
}
impl RecordHttpsConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.bind.ip().is_loopback()
                && (1..=4096).contains(&self.capacity)
                && (1..=300).contains(&self.ttl_seconds),
            "Invalid record HTTPS bounds"
        );
        rustls::pki_types::ServerName::try_from(self.server_name.clone())
            .map_err(|_| anyhow::anyhow!("Invalid record HTTPS server name"))?;
        anyhow::ensure!(
            self.server_name.len() <= 253 && !self.server_name.contains(['/', '@', '?', '#']),
            "Invalid record HTTPS server name"
        );
        Ok(())
    }
}
#[derive(Clone)]
struct Data {
    service: Arc<Service>,
    base: String,
    capacity: Arc<tokio::sync::Semaphore>,
}
struct Retirement {
    stop: CancellationToken,
    registry: Arc<RecordReferenceRegistry>,
}
impl connect_store::RuntimeRetirementSignal for Retirement {
    fn retire(&self) {
        self.stop.cancel();
        self.registry.close();
    }
}
pub struct RecordHttpsListener {
    pub address: SocketAddr,
    pub registry: Arc<RecordReferenceRegistry>,
    stop: CancellationToken,
    task: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
    _retirement: Arc<dyn connect_store::RuntimeRetirementSignal>,
}
impl RecordHttpsListener {
    pub async fn bind(
        config: &RecordHttpsConfig,
        service: Arc<Service>,
        mut tls: rustls::ServerConfig,
        certificate: &std::path::Path,
    ) -> anyhow::Result<Self> {
        use rustls::client::danger::ServerCertVerifier;
        config.validate()?;
        // Refuse an advertised origin whose configured certificate cannot
        // authenticate that name; construction is not provider reachability.
        let der = rustls::pki_types::CertificateDer::from(std::fs::read(certificate)?);
        let mut roots = rustls::RootCertStore::empty();
        roots.add(der.clone())?;
        let verifier = rustls::client::WebPkiServerVerifier::builder(Arc::new(roots)).build()?;
        verifier.verify_server_cert(
            &der,
            &[],
            &rustls::pki_types::ServerName::try_from(config.server_name.clone())?,
            &[],
            rustls::pki_types::UnixTime::now(),
        )?;
        tls.alpn_protocols = vec![b"http/1.1".to_vec()];
        let listener = tokio::net::TcpListener::bind(config.bind).await?;
        let address = listener.local_addr()?;
        let mut url = url::Url::parse("https://localhost/v2/record-references")?;
        url.set_host(Some(&config.server_name))
            .map_err(|_| anyhow::anyhow!("Invalid record origin"))?;
        url.set_port(Some(address.port()))
            .map_err(|_| anyhow::anyhow!("Invalid record port"))?;
        let base = url.to_string();
        let registry = Arc::new(
            RecordReferenceRegistry::new(
                base.clone(),
                config.capacity,
                Duration::from_secs(config.ttl_seconds as u64),
            )
            .map_err(|_| anyhow::anyhow!("Invalid record registry"))?,
        );
        let stop = CancellationToken::new();
        let retirement: Arc<dyn connect_store::RuntimeRetirementSignal> = Arc::new(Retirement {
            stop: stop.clone(),
            registry: registry.clone(),
        });
        service.store.register_runtime_retirement(&retirement)?;
        service.install_record_references(registry.clone())?;
        let state = Data {
            service,
            base,
            capacity: Arc::new(tokio::sync::Semaphore::new(config.capacity.min(256))),
        };
        let app = Router::new()
            .route("/v2/record-references/{token}", get(download))
            .with_state(state);
        let token = stop.clone();
        let lifetime_registry = registry.clone();
        let handshakes = config.capacity.min(256);
        let task = tokio::spawn(async move {
            struct Finish(Arc<RecordReferenceRegistry>, CancellationToken);
            impl Drop for Finish {
                fn drop(&mut self) {
                    self.0.close();
                    self.1.cancel();
                }
            }
            let _finish = Finish(lifetime_registry, token.clone());
            let io = GuardedListener {
                listener,
                cancellation: token.clone(),
                tls: Some(Arc::new(tls)),
                handshakes: Arc::new(tokio::sync::Semaphore::new(handshakes)),
                connections: Arc::new(tokio::sync::Semaphore::new(handshakes)),
            };
            axum::serve(io, app.into_make_service_with_connect_info::<Custody>())
                .with_graceful_shutdown(token.cancelled_owned())
                .await
        });
        Ok(Self {
            address,
            registry,
            stop,
            task: Some(task),
            _retirement: retirement,
        })
    }
    pub fn begin_shutdown(&self) {
        self.registry.close();
        self.stop.cancel();
    }
    pub async fn shutdown(mut self) -> anyhow::Result<()> {
        self.begin_shutdown();
        if let Some(mut task) = self.task.take() {
            match tokio::time::timeout(Duration::from_secs(6), &mut task).await {
                Ok(result) => result??,
                Err(_) => {
                    task.abort();
                    let _ = task.await;
                    anyhow::bail!("Record HTTPS shutdown timed out")
                }
            }
        }
        Ok(())
    }
}
impl Drop for RecordHttpsListener {
    fn drop(&mut self) {
        self.begin_shutdown();
        if let Some(task) = self.task.take() {
            task.abort()
        }
    }
}
fn rejection(error: Error) -> Response {
    let problem = Problem::from_error(&error);
    let status = StatusCode::from_u16(problem.code).unwrap_or(StatusCode::BAD_REQUEST);
    (
        status,
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::CONNECTION, "close"),
        ],
        Json(Rejection {
            profile: PROFILE.into(),
            problem,
        }),
    )
        .into_response()
}
async fn download(
    State(state): State<Data>,
    ConnectInfo(connection): ConnectInfo<Custody>,
    Path(token): Path<String>,
    version: Version,
    headers: HeaderMap,
) -> Response {
    let raw = match super::http::bearer_token(&headers) {
        Ok(raw) => raw,
        Err(error) => return rejection(error),
    };
    let credential = match StoreValidator::canonical(state.service.store.clone())
        .validate_credential(&raw)
        .await
    {
        Ok(credential) => credential,
        Err(rvoip_auth_core::bearer::BearerAuthError::Unavailable(_)) => {
            return rejection(Error::storage())
        }
        Err(_) => return rejection(Error::unauthorized()),
    };
    let store = state.service.store.clone();
    let binding = match tokio::task::spawn_blocking(move || {
        let binding = StoreCredentialBinding::from_validated(&store, &credential)?;
        // This dedicated metadata listener refuses every browser Origin.
        store.check_enrollment_http_origin(binding.credential_id(), None)?;
        Ok::<_, Error>(binding)
    })
    .await
    {
        Ok(Ok(value)) => value,
        Ok(Err(error)) if error.code == "storage-unavailable" => return rejection(error),
        Ok(Err(_)) => return rejection(Error::unauthorized()),
        Err(_) => return rejection(Error::storage()),
    };
    if version != Version::HTTP_11 || headers.contains_key(header::UPGRADE) {
        return rejection(Error::new(
            "unsupported-command",
            "Record download requires HTTP1.1",
        ));
    }
    if token.len() != 32
        || !token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return rejection(Error::new("not-found", "Record reference unavailable"));
    }
    let Ok(permit) = state.capacity.clone().try_acquire_owned() else {
        return rejection(Error::new(
            "capacity-exceeded",
            "Record download capacity exhausted",
        ));
    };
    let url = format!("{}/{}", state.base, token);
    let response = match state
        .service
        .retrieve_record_reference(binding, &url, connection.cancellation())
        .await
    {
        Ok(response) => response,
        Err(error) => return rejection(error),
    };
    let (status, body) = match connection.install(response, Arc::new(permit)) {
        Ok(value) => value,
        Err(error) => return rejection(error),
    };
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
        [
            (header::CONTENT_TYPE, "application/vcon+json"),
            (header::CACHE_CONTROL, "no-store"),
            (header::CONNECTION, "close"),
        ],
        body,
    )
        .into_response()
}
