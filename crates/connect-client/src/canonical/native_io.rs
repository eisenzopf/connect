//! One owned datagram reader can progress beside the same peer's control waits.
use super::*;

pub(super) struct ReaderClaim(Arc<AtomicBool>);
impl ReaderClaim {
    pub(super) fn take(owner: &Arc<AtomicBool>) -> Result<Self> {
        owner
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| ClientError::Protocol)?;
        Ok(Self(owner.clone()))
    }
}
impl Drop for ReaderClaim {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Raw RTP/clock IO on one existing authenticated native peer. This owns its
/// single datagram reader, not a new connection or media permission. Signed
/// control may continue through the original `CanonicalClient` concurrently.
///
/// Closing/dropping that client still retires this IO. A captured credential or
/// server-key expiry cannot be renewed by refreshing the client: drop this
/// owner and claim again after refresh. The server rechecks current authority
/// and negotiated stream ownership for every packet. Receive cancellation is
/// atomic and leaves both this owner and control usable.
pub struct NativeDatagrams {
    connection: quinn::Connection,
    closed: Arc<AtomicBool>,
    expires_at: chrono::DateTime<chrono::Utc>,
    timeout: Duration,
    _reader: ReaderClaim,
}
impl NativeDatagrams {
    pub(super) fn current(&self) -> Result<()> {
        if self.closed.load(Ordering::Acquire) || self.connection.close_reason().is_some() {
            return Err(ClientError::Transport);
        }
        if chrono::Utc::now() >= self.expires_at {
            return Err(ClientError::Authentication);
        }
        Ok(())
    }
    /// Queue original encoded bytes locally; this is not server admission or
    /// delivery. No operation ID, RTP timestamp or packet body is rewritten.
    pub fn send(&self, bytes: &[u8]) -> Result<()> {
        super::native::datagram(bytes)?;
        self.submit(bytes)
    }
    pub(super) fn submit(&self, bytes: &[u8]) -> Result<()> {
        self.current()?;
        if self
            .connection
            .max_datagram_size()
            .is_none_or(|n| bytes.len() > n)
        {
            return Err(ClientError::Protocol);
        }
        self.connection
            .send_datagram(bytes.to_vec().into())
            .map_err(|_| ClientError::Transport)
    }
    pub async fn receive(&mut self) -> Result<Vec<u8>> {
        let bytes = self.read_raw().await?;
        super::native::datagram(&bytes)?;
        Ok(bytes)
    }
    pub(super) async fn read_raw(&mut self) -> Result<Vec<u8>> {
        self.current()?;
        let remaining = (self.expires_at - chrono::Utc::now())
            .to_std()
            .unwrap_or_default();
        let bytes =
            tokio::time::timeout(self.timeout.min(remaining), self.connection.read_datagram())
                .await
                .map_err(|_| ClientError::Timeout)?
                .map_err(|_| ClientError::Transport)?;
        self.current()?;
        if bytes.len() > 16 * 1024 + 36 {
            return Err(ClientError::Protocol);
        }
        Ok(bytes.to_vec())
    }
}
impl CanonicalClient {
    /// Reserve the one raw datagram reader while retaining this original peer
    /// for signed control. A second owner or legacy read refuses until drop.
    pub fn claim_native_datagrams(&self) -> Result<NativeDatagrams> {
        self.check_current()?;
        Ok(NativeDatagrams {
            connection: self.connection.clone(),
            closed: self.closed.clone(),
            expires_at: self.authenticated_until.min(self.server_key_expires_at),
            timeout: self.deadline,
            _reader: ReaderClaim::take(&self.native_datagram_reader)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::tests::{controlled, Fault};

    #[tokio::test]
    async fn native_io_duplex_progresses_while_original_signed_control_waits() {
        let (mut client, server, _) = controlled(Fault::NativeDuplexDuringControl).await;
        let mut media = client.claim_native_datagrams().unwrap();
        // An atomic cancelled packet read keeps the same owner usable.
        assert!(
            tokio::time::timeout(Duration::from_millis(5), media.receive())
                .await
                .is_err()
        );
        let control = client.request_value(
            "operation.get",
            json!({"operation_id":"op_probe"}),
            CommandScope::default(),
        );
        let packets = async {
            let clock = media.receive().await?;
            assert!(matches!(
                observe_native_media_datagram(&clock).unwrap(),
                ObservedNativeMediaDatagram::Clock(_)
            ));
            let packet = media.receive().await?;
            assert!(matches!(
                observe_native_media_datagram(&packet).unwrap(),
                ObservedNativeMediaDatagram::Rtp(_)
            ));
            // The server withholds this original ACK until actual media duplex
            // completes. Serial control-then-media execution cannot pass.
            media.send(&packet)?;
            Ok::<_, ClientError>(())
        };
        let (batch, ()) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::try_join!(control, packets)
        })
        .await
        .unwrap()
        .unwrap();
        batch.ack().unwrap();
        drop(media);
        client.shutdown().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn native_io_exclusive_reader_releases_without_retiring_control() {
        let (mut client, server, _) = controlled(Fault::NativeDuplexDuringControl).await;
        let media = client.claim_native_datagrams().unwrap();
        assert!(matches!(
            client.claim_native_datagrams(),
            Err(ClientError::Protocol)
        ));
        assert!(matches!(
            client.receive_native_media_datagram().await,
            Err(ClientError::Protocol)
        ));
        assert!(matches!(
            client.receive_owned_native_datagram().await,
            Err(ClientError::Protocol)
        ));
        drop(media);
        let second = client.claim_native_datagrams().unwrap();
        client.check_current().unwrap();
        drop(second);
        client.shutdown().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn native_io_captured_expiry_cannot_renew_and_client_close_retires_owner() {
        let (client, server, _) = controlled(Fault::NativeDuplexDuringControl).await;
        let mut media = client.claim_native_datagrams().unwrap();
        media.expires_at = chrono::Utc::now() - chrono::Duration::seconds(1);
        assert!(matches!(
            media.receive().await,
            Err(ClientError::Authentication)
        ));
        // Expiry of a captured owner cannot close a freshly authorized peer.
        client.check_current().unwrap();
        drop(media);
        let mut current = client.claim_native_datagrams().unwrap();
        client.close();
        assert!(matches!(
            current.receive().await,
            Err(ClientError::Transport)
        ));
        drop(current);
        client.shutdown().await.unwrap();
        server.await.unwrap();
    }
}
