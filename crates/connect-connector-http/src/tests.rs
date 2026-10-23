#![allow(clippy::unwrap_used)]
use super::*;
use connect_connector_api::dispatch::Authority;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};
struct Lease {
    alive: Arc<AtomicBool>,
    end: Instant,
    drop: Option<oneshot::Sender<()>>,
}
impl Authority for Lease {
    fn current(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }
    fn deadline(&self) -> Instant {
        self.end
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(drop) = self.drop.take() {
            let _ = drop.send(());
        }
    }
}
fn permit(ttl: Duration) -> (DispatchPermit, Arc<AtomicBool>, oneshot::Receiver<()>) {
    let alive = Arc::new(AtomicBool::new(true));
    let (drop, done) = oneshot::channel();
    (
        DispatchPermit::from_host(Arc::new(Lease {
            alive: alive.clone(),
            end: Instant::now() + ttl,
            drop: Some(drop),
        })),
        alive,
        done,
    )
}
#[tokio::test]
async fn actual_socket_rechecks_authority_before_each_write_and_drops_socket_before_custody() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let socket = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (mut peer, _) = listener.accept().await.unwrap();
    let (p, alive, mut done) = permit(Duration::from_secs(2));
    let mut io = Guarded {
        socket,
        permit: p,
        stop: CancellationToken::new(),
    };
    io.write_all(b"admitted").await.unwrap();
    let mut first = [0; 8];
    peer.read_exact(&mut first).await.unwrap();
    assert_eq!(&first, b"admitted");
    alive.store(false, Ordering::Release);
    assert_eq!(
        io.write_all(b"retired-secret").await.unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert!(matches!(
        done.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    drop(io);
    let mut tail = vec![];
    tokio::time::timeout(Duration::from_secs(1), peer.read_to_end(&mut tail))
        .await
        .unwrap()
        .unwrap();
    assert!(tail.is_empty());
    done.await.unwrap();
}
#[tokio::test]
async fn cancelling_http_future_closes_real_pending_connection_and_releases_transport_custody() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let client = Arc::new(Client::new(&base, "fixture-secret").unwrap());
    let (p, _, mut done) = permit(Duration::from_secs(3));
    let task = tokio::spawn(async move {
        client
            .request(
                Method::POST,
                &["effect"],
                Some(serde_json::json!({"body":"one attempt"})),
                p,
            )
            .await
    });
    let (mut peer, _) = listener.accept().await.unwrap();
    let mut raw = vec![];
    let mut buf = [0; 4096];
    loop {
        let n = tokio::time::timeout(Duration::from_secs(1), peer.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert!(n > 0);
        raw.extend_from_slice(&buf[..n]);
        if raw.windows(11).any(|w| w == b"one attempt") {
            break;
        }
    }
    assert!(matches!(
        done.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    task.abort();
    assert!(matches!(task.await,Err(error) if error.is_cancelled()));
    let mut tail = vec![];
    tokio::time::timeout(Duration::from_secs(1), peer.read_to_end(&mut tail))
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), done)
        .await
        .unwrap()
        .unwrap();
    let request = String::from_utf8(raw).unwrap();
    assert_eq!(request.matches("POST /effect").count(), 1);
}
#[tokio::test]
async fn permit_expiry_closes_stalled_http_without_retry_and_a_clone_cannot_create_twice() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let client = Arc::new(Client::new(&base, "fixture-secret").unwrap());
    let (p, _, done) = permit(Duration::from_secs(1));
    let clone = p.clone();
    let task = tokio::spawn(async move {
        client
            .request(
                Method::POST,
                &["effect"],
                Some(serde_json::json!({"body":"one"})),
                p,
            )
            .await
    });
    let (mut peer, _) = listener.accept().await.unwrap();
    let mut bytes = [0; 4096];
    let n = peer.read(&mut bytes).await.unwrap();
    assert!(n > 0);
    assert_eq!(clone.admit_effect(), Err(Failure::Uncertain));
    drop(clone);
    assert!(matches!(task.await.unwrap(), Err(Failure::Uncertain)));
    let mut tail = vec![];
    tokio::time::timeout(Duration::from_secs(1), peer.read_to_end(&mut tail))
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), done)
        .await
        .unwrap()
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn pinned_tls_pending_request_has_the_same_owned_cancellation_boundary() {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert.cert.der().clone()).unwrap();
    let client_tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der());
    let server_tls = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(vec![cert.cert.der().clone()], key.into())
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!(
        "https://localhost:{}",
        listener.local_addr().unwrap().port()
    );
    let client = Arc::new(Client::with_tls(&base, "fixture-secret", Arc::new(client_tls)).unwrap());
    let (p, _, mut done) = permit(Duration::from_secs(3));
    let task = tokio::spawn(async move {
        client
            .request(
                Method::POST,
                &["tls-effect"],
                Some(serde_json::json!({"body":"private"})),
                p,
            )
            .await
    });
    let (socket, _) = listener.accept().await.unwrap();
    let mut socket = tokio_rustls::TlsAcceptor::from(Arc::new(server_tls))
        .accept(socket)
        .await
        .unwrap();
    let mut bytes = [0; 4096];
    let n = socket.read(&mut bytes).await.unwrap();
    assert!(String::from_utf8_lossy(&bytes[..n]).contains("POST /tls-effect"));
    assert!(matches!(
        done.try_recv(),
        Err(tokio::sync::oneshot::error::TryRecvError::Empty)
    ));
    task.abort();
    let _ = task.await;
    let mut tail = vec![];
    // A cancelled TLS request may close without close_notify; neither outcome is a second request.
    let _ = tokio::time::timeout(Duration::from_secs(1), socket.read_to_end(&mut tail))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), done)
        .await
        .unwrap()
        .unwrap();
}
#[test]
fn configuration_rejects_plaintext_remote_and_credential_bearing_urls() {
    assert!(Client::new("http://[::1]:8080", "secret").is_ok());
    assert!(Client::new("http://[2001:db8::1]:8080", "secret").is_err());
    for value in [
        "http://provider.invalid",
        "https://user:secret@provider.invalid",
        "https://provider.invalid/#secret",
        "file:///private",
    ] {
        assert!(Client::new(value, "secret").is_err());
    }
}

#[tokio::test]
async fn read_only_status_permit_refuses_creation_and_retains_socket_until_cancelled_get_closes() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = Arc::new(
        Client::new(
            &format!("http://{}", listener.local_addr().unwrap()),
            "fixture-secret",
        )
        .unwrap(),
    );
    let (drop, done) = oneshot::channel();
    let p = DispatchPermit::from_host_readonly(Arc::new(Lease {
        alive: Arc::new(AtomicBool::new(true)),
        end: Instant::now() + Duration::from_secs(3),
        drop: Some(drop),
    }));
    assert!(matches!(
        client
            .request(Method::POST, &["effect"], None, p.clone())
            .await,
        Err(Failure::Rejected)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(30), listener.accept())
            .await
            .is_err()
    );
    let task = tokio::spawn(async move {
        client
            .request(Method::GET, &["status", "existing"], None, p)
            .await
    });
    let (mut peer, _) = listener.accept().await.unwrap();
    let mut bytes = vec![];
    let mut buf = [0; 2048];
    loop {
        let n = tokio::time::timeout(Duration::from_secs(1), peer.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&buf[..n]);
        if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    assert!(bytes.starts_with(b"GET /status/existing "));
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    let mut tail = vec![];
    tokio::time::timeout(Duration::from_secs(1), peer.read_to_end(&mut tail))
        .await
        .unwrap()
        .unwrap();
    assert!(tail.is_empty());
    tokio::time::timeout(Duration::from_secs(1), done)
        .await
        .unwrap()
        .unwrap();
}
