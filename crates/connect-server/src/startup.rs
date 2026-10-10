//! Native accept tasks hold endpoint references; cancel them on failed startup.
use std::sync::Arc;

pub(crate) struct EndpointStartup(Option<Arc<quinn::Endpoint>>);
impl EndpointStartup {
    pub fn new(endpoint: &Arc<quinn::Endpoint>) -> Self {
        Self(Some(endpoint.clone()))
    }
    /// Transfer lifecycle ownership to the fully constructed RunningServer.
    pub fn finish(mut self) {
        self.0.take();
    }
}
impl Drop for EndpointStartup {
    fn drop(&mut self) {
        if let Some(endpoint) = &self.0 {
            endpoint.close(0u32.into(), b"startup failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn failed_startup_releases_endpoint_owned_by_a_pending_accept_task() {
        let endpoint = endpoint();
        let address = endpoint.local_addr().expect("bound address");
        let startup = EndpointStartup::new(&endpoint);
        let incoming = endpoint.clone();
        let accept = tokio::spawn(async move {
            assert!(incoming.accept().await.is_none());
        });
        drop(endpoint);
        assert!(std::net::UdpSocket::bind(address).is_err());
        drop(startup); // failed construction closes the outstanding accept
        tokio::time::timeout(Duration::from_secs(2), accept)
            .await
            .expect("accept cancelled")
            .expect("accept task joined");
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if std::net::UdpSocket::bind(address).is_ok() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("socket released after failed startup");
    }

    #[tokio::test]
    async fn successful_startup_keeps_accept_open_for_runtime_ownership() {
        let endpoint = endpoint();
        EndpointStartup::new(&endpoint).finish();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), endpoint.accept())
                .await
                .is_err()
        );
        endpoint.close(0u32.into(), b"test complete");
        assert!(endpoint.accept().await.is_none());
    }

    fn endpoint() -> Arc<quinn::Endpoint> {
        let temp = tempfile::tempdir().expect("isolated TLS directory");
        let generated = crate::tls::generate_local_certificate().expect("local certificate");
        let certificate = temp.path().join("certificate.der");
        let private_key = temp.path().join("private-key.der");
        std::fs::write(&certificate, generated.cert.der().as_ref()).expect("certificate");
        std::fs::write(&private_key, generated.signing_key.serialize_der()).expect("private key");
        Arc::new(
            rvoip_uctp::substrate::make_server_endpoint(
                "127.0.0.1:0".parse().expect("loopback"),
                crate::tls::server_config(&certificate, &private_key).expect("TLS config"),
                quinn::TransportConfig::default(),
            )
            .expect("native endpoint"),
        )
    }
}
