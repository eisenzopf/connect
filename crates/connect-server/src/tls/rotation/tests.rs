use super::*;
use std::{net::SocketAddr, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Fixture {
    _directory: tempfile::TempDir,
    certificate: PathBuf,
    key: PathBuf,
    ca: rcgen::CertifiedIssuer<'static, rcgen::KeyPair>,
}
impl Fixture {
    fn new() -> Self {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/gate-6/https-listeners");
        std::fs::create_dir_all(&root).unwrap();
        let directory = tempfile::tempdir_in(root).unwrap();
        let mut ca = rcgen::CertificateParams::default();
        ca.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        ca.key_usages = vec![rcgen::KeyUsagePurpose::KeyCertSign];
        let ca =
            rcgen::CertifiedIssuer::self_signed(ca, rcgen::KeyPair::generate().unwrap()).unwrap();
        Self {
            certificate: directory.path().join("chain.pem"),
            key: directory.path().join("key.pem"),
            _directory: directory,
            ca,
        }
    }
    fn replace(&self, name: &str, expired: bool) -> Vec<u8> {
        let mut parameters = rcgen::CertificateParams::new(vec![name.into()]).unwrap();
        let now = std::time::SystemTime::now();
        parameters.not_before = (now - Duration::from_secs(60)).into();
        parameters.not_after = if expired {
            now - Duration::from_secs(30)
        } else {
            now + Duration::from_secs(3600)
        }
        .into();
        parameters.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
        let key = rcgen::KeyPair::generate().unwrap();
        let certificate = parameters.signed_by(&key, &self.ca).unwrap();
        std::fs::write(&self.certificate, certificate.pem()).unwrap();
        let mut options = std::fs::OpenOptions::new();
        options.create(true).write(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        options
            .open(&self.key)
            .unwrap()
            .write_all(key.serialize_pem().as_bytes())
            .unwrap();
        certificate.der().to_vec()
    }
    fn client(&self) -> tokio_rustls::TlsConnector {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(self.ca.der().clone()).unwrap();
        let mut config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        config.resumption = rustls::client::Resumption::disabled();
        tokio_rustls::TlsConnector::from(Arc::new(config))
    }
}
type Client = tokio_rustls::client::TlsStream<tokio::net::TcpStream>;
async fn connect(f: &Fixture, address: SocketAddr, expected: &[u8]) -> Client {
    let stream = tokio::time::timeout(Duration::from_secs(3), async {
        let socket = tokio::net::TcpStream::connect(address).await.unwrap();
        f.client()
            .connect(
                rustls::pki_types::ServerName::try_from("localhost").unwrap(),
                socket,
            )
            .await
            .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(
        stream.get_ref().1.peer_certificates().unwrap()[0].as_ref(),
        expected
    );
    stream
}
async fn echo(stream: &mut Client, value: u8) {
    tokio::time::timeout(Duration::from_secs(3), async {
        stream.write_all(&[value]).await.unwrap();
        stream.flush().await.unwrap();
        assert_eq!(stream.read_u8().await.unwrap(), value);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cloned_listener_configs_rotate_actual_tls_handshakes_and_preserve_existing_sessions() {
    let f = Fixture::new();
    let first = f.replace("localhost", false);
    let identity = ReloadableServerTls::load(&f.certificate, &f.key).unwrap();
    assert_eq!(
        identity.reload().unwrap(),
        TlsReload {
            revision: 1,
            changed: false
        }
    );
    // Protocol-specific clones share the resolver, as canonical QUIC/WT/WSS
    // and record HTTPS do. Full handshakes use ordinary CA/name validation.
    let mut config = (*identity.config()).clone();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut sessions = tokio::task::JoinSet::new();
        for _ in 0..3 {
            let (socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let acceptor = acceptor.clone();
            sessions.spawn(async move {
                let mut stream =
                    tokio::time::timeout(Duration::from_secs(3), acceptor.accept(socket))
                        .await
                        .unwrap()
                        .unwrap();
                while let Ok(byte) = stream.read_u8().await {
                    stream.write_all(&[byte]).await.unwrap();
                    stream.flush().await.unwrap();
                }
                stream.shutdown().await.unwrap();
            });
        }
        while let Some(result) = sessions.join_next().await {
            result.unwrap();
        }
    });
    let mut old = connect(&f, address, &first).await;
    echo(&mut old, 1).await;
    let second = f.replace("localhost", false);
    assert_ne!(first, second);
    assert_eq!(
        identity.reload().unwrap(),
        TlsReload {
            revision: 2,
            changed: true
        }
    );
    let mut new = connect(&f, address, &second).await;
    echo(&mut new, 2).await;
    echo(&mut old, 3).await;
    std::fs::write(&f.key, b"incomplete operator key replacement").unwrap();
    assert!(identity.reload().is_err());
    let mut retained = connect(&f, address, &second).await;
    echo(&mut retained, 4).await;
    for stream in [&mut old, &mut new, &mut retained] {
        stream.shutdown().await.unwrap();
    }
    drop((old, new, retained));
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
}

#[test]
fn failed_expired_mismatched_oversized_or_narrower_identity_reload_keeps_last_good_revision() {
    for fault in 0..4 {
        let f = Fixture::new();
        f.replace("localhost", false);
        let certificate = std::fs::read(&f.certificate).unwrap();
        let key = std::fs::read(&f.key).unwrap();
        let identity = ReloadableServerTls::load(&f.certificate, &f.key).unwrap();
        match fault {
            0 => {
                f.replace("localhost", true);
            }
            1 => {
                std::fs::write(&f.key, rcgen::KeyPair::generate().unwrap().serialize_pem()).unwrap()
            }
            2 => std::fs::write(&f.certificate, vec![0; 256 * 1024 + 1]).unwrap(),
            _ => {
                f.replace("different.invalid", false);
            }
        }
        assert!(identity.reload().is_err(), "fault={fault}");
        std::fs::write(&f.certificate, certificate).unwrap();
        std::fs::write(&f.key, key).unwrap();
        assert_eq!(
            identity.reload().unwrap(),
            TlsReload {
                revision: 1,
                changed: false
            }
        );
    }
}
