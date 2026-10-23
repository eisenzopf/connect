//! Dedicated HTTP/1 attempts with authority custody in the actual socket.
//! No pooling, redirects or retries; cancellation closes owned pending I/O.
use connect_connector_api::{DispatchPermit, Failure};
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
};
use tokio_util::sync::CancellationToken;
use url::Url;

pub struct Client {
    base: Url,
    bearer: String,
    tls: Arc<rustls::ClientConfig>,
}
impl Client {
    pub fn new(base: &str, bearer: &str) -> Result<Self, Failure> {
        let roots = rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|_| Failure::Rejected)?
        .with_root_certificates(roots)
        .with_no_client_auth();
        Self::with_tls(base, bearer, Arc::new(tls))
    }
    /// Protected host configuration, also used by isolated pinned-TLS fixtures.
    pub fn with_tls(
        base: &str,
        bearer: &str,
        tls: Arc<rustls::ClientConfig>,
    ) -> Result<Self, Failure> {
        let base = Url::parse(base).map_err(|_| Failure::Rejected)?;
        let local = base.host_str().is_some_and(|h| {
            h == "localhost"
                || h.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        if !(base.scheme() == "https" || (base.scheme() == "http" && local))
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || bearer.contains(['\r', '\n'])
        {
            return Err(Failure::Rejected);
        }
        Ok(Self {
            base,
            bearer: bearer.into(),
            tls,
        })
    }
    pub async fn request(
        &self,
        method: Method,
        segments: &[&str],
        body: Option<serde_json::Value>,
        permit: DispatchPermit,
    ) -> Result<Response, Failure> {
        permit.check()?;
        let mut url = self.base.clone();
        {
            let mut path = url.path_segments_mut().map_err(|_| Failure::Rejected)?;
            path.pop_if_empty();
            for segment in segments {
                path.push(segment);
            }
        }
        let bytes = body
            .map(|v| serde_json::to_vec(&v))
            .transpose()
            .map_err(|_| Failure::Rejected)?
            .unwrap_or_default();
        if bytes.len() > 1024 * 1024 {
            return Err(Failure::Rejected);
        }
        if method != Method::GET && method != Method::HEAD && method != Method::OPTIONS {
            permit.admit_effect()?;
        }
        let stop = CancellationToken::new();
        // Exists before connect/handshake as well as during the HTTP driver.
        let _close = Close(stop.clone());
        let operation = async {
            let host = url
                .host_str()
                .ok_or(Failure::Rejected)?
                .trim_matches(['[', ']']);
            let port = url.port_or_known_default().ok_or(Failure::Rejected)?;
            let socket = TcpStream::connect((host, port))
                .await
                .map_err(|_| Failure::Uncertain)?;
            if url.scheme() == "http"
                && !socket
                    .peer_addr()
                    .map_err(|_| Failure::Uncertain)?
                    .ip()
                    .is_loopback()
            {
                return Err(Failure::Rejected);
            }
            let socket = Guarded {
                socket,
                permit: permit.clone(),
                stop: stop.clone(),
            };
            let io: Box<dyn Socket> = if url.scheme() == "https" {
                let name = rustls::pki_types::ServerName::try_from(host.to_owned())
                    .map_err(|_| Failure::Rejected)?;
                let tls = tokio_rustls::TlsConnector::from(self.tls.clone())
                    .connect(name, socket)
                    .await
                    .map_err(|_| Failure::Uncertain)?;
                Box::new(tls)
            } else {
                Box::new(socket)
            };
            let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(io))
                .await
                .map_err(|_| Failure::Uncertain)?;
            let cancellation = stop.clone();
            let custody = permit.clone();
            // The task owns the socket and its proof even when its caller drops
            // the request future. No JoinHandle abort is used as proof of cleanup.
            tokio::spawn(async move {
                tokio::pin!(connection);
                let mut check = tokio::time::interval(Duration::from_millis(10));
                loop {
                    tokio::select! { biased;
                        _=cancellation.cancelled()=>break,
                        _=tokio::time::sleep_until(custody.deadline().into())=>break,
                        _=check.tick()=>if custody.check().is_err() { break; },
                        _=&mut connection=>break,
                    }
                }
            });
            let uri = &url[url::Position::BeforePath..];
            let mut host_header = host.to_owned();
            if host.contains(':') {
                host_header = format!("[{host}]");
            }
            if url.port().is_some() {
                host_header = format!("{host_header}:{port}");
            }
            let request = Request::builder()
                .method(method)
                .uri(uri)
                .header("host", host_header)
                .header("user-agent", "Connect/0.1")
                .header("authorization", format!("Bearer {}", self.bearer))
                .header("content-type", "application/json")
                .header("connection", "close")
                .body(Full::new(bytes::Bytes::from(bytes)))
                .map_err(|_| Failure::Rejected)?;
            permit.check()?;
            let response = sender
                .send_request(request)
                .await
                .map_err(|_| Failure::Uncertain)?;
            let status = response.status().as_u16();
            let mut body = response.into_body();
            let mut data = vec![];
            while let Some(frame) = body.frame().await {
                let frame = frame.map_err(|_| Failure::Uncertain)?;
                if let Ok(bytes) = frame.into_data() {
                    if data.len().saturating_add(bytes.len()) > 1024 * 1024 {
                        return Err(Failure::Uncertain);
                    }
                    data.extend_from_slice(&bytes);
                }
            }
            let value = if data.is_empty() && !(200..300).contains(&status) {
                serde_json::Value::Null
            } else {
                serde_json::from_slice(&data).map_err(|_| Failure::Uncertain)?
            };
            Ok(Response { status, value })
        };
        tokio::time::timeout_at(permit.deadline().into(), operation)
            .await
            .unwrap_or(Err(Failure::Uncertain))
    }
}
pub use hyper::Method as HttpMethod;
pub struct Response {
    pub status: u16,
    pub value: serde_json::Value,
}
impl Response {
    pub fn success(self) -> Result<serde_json::Value, Failure> {
        if (200..300).contains(&self.status) {
            Ok(self.value)
        } else if (400..500).contains(&self.status) && self.status != 408 {
            Err(Failure::Rejected)
        } else {
            Err(Failure::Uncertain)
        }
    }
}
struct Close(CancellationToken);
impl Drop for Close {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
trait Socket: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Socket for T {}
struct Guarded {
    socket: TcpStream,
    permit: DispatchPermit,
    stop: CancellationToken,
}
impl Guarded {
    fn writable(&self) -> std::io::Result<()> {
        if self.stop.is_cancelled() || self.permit.check().is_err() {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Dispatch authority retired",
            ))
        } else {
            Ok(())
        }
    }
}
impl AsyncRead for Guarded {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().socket).poll_read(cx, buf)
    }
}
impl AsyncWrite for Guarded {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if let Err(e) = self.writable() {
            return Poll::Ready(Err(e));
        }
        Pin::new(&mut self.get_mut().socket).poll_write(cx, data)
    }
    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        if let Err(e) = self.writable() {
            return Poll::Ready(Err(e));
        }
        Pin::new(&mut self.get_mut().socket).poll_write_vectored(cx, data)
    }
    fn is_write_vectored(&self) -> bool {
        self.socket.is_write_vectored()
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        if let Err(e) = self.writable() {
            return Poll::Ready(Err(e));
        }
        Pin::new(&mut self.get_mut().socket).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().socket).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests;
