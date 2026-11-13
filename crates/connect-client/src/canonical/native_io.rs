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
        super::native_admission::try_submit(&self.connection, bytes)
    }
    /// Wait for space for these original RTP/clock bytes without evicting an
    /// older queued datagram. The caller must provide its original capture or
    /// publication cutoff; this call additionally caps its own wait at200ms and
    /// this owner's captured authentication expiry. No timestamp/sequence,
    /// credential, operation or negotiated media permission is renewed.
    ///
    /// Dropping this future while capacity is pending submits nothing. Every
    /// capacity wake rechecks this same owner's current physical/credential
    /// state immediately before the atomic non-evicting enqueue attempt.
    /// Success means local enqueue only; the server still authorizes each RTP.
    pub async fn send_until(&self, bytes: &[u8], until: tokio::time::Instant) -> Result<()> {
        super::native::datagram(bytes)?;
        self.current()?;
        if self
            .connection
            .max_datagram_size()
            .is_none_or(|n| bytes.len() > n)
        {
            return Err(ClientError::Protocol);
        }
        let now = tokio::time::Instant::now();
        let remaining = (self.expires_at - chrono::Utc::now())
            .to_std()
            .unwrap_or_default();
        let until = until
            .min(now + Duration::from_millis(200))
            .min(now + remaining);
        if now >= until {
            return Err(ClientError::Timeout);
        }
        // This pinned future owns one immutable byte copy. Quinn's wait path
        // atomically refuses a full queue without evicting existing originals.
        // No authority, task, timer owner or queue exists outside this call.
        let mut pending = std::pin::pin!(self.connection.send_datagram_wait(bytes.to_vec().into()));
        let checked = std::future::poll_fn(|cx| {
            if let Err(problem) = self.current() {
                return std::task::Poll::Ready(Err(problem));
            }
            if tokio::time::Instant::now() >= until {
                return std::task::Poll::Ready(Err(ClientError::Timeout));
            }
            std::future::Future::poll(pending.as_mut(), cx)
                .map(|result| result.map_err(|_| ClientError::Transport))
        });
        tokio::select! { biased;
            _ = tokio::time::sleep_until(until) => {
                self.current()?;
                Err(ClientError::Timeout)
            },
            result = checked => result,
        }
    }
    /// One original receive budget, also used by the video reader's existing
    /// wait loop. Feedback wakeups must not renew its physical read timeout.
    pub(super) fn receive_deadline(&self) -> Result<tokio::time::Instant> {
        self.current()?;
        let remaining = (self.expires_at - chrono::Utc::now())
            .to_std()
            .unwrap_or_default();
        Ok(tokio::time::Instant::now() + self.timeout.min(remaining))
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
    #[tokio::test]
    async fn native_io_send_until_keeps_original_deadline_and_actual_duplex_bytes() {
        let (mut client, server, _) = controlled(Fault::NativeDuplexDuringControl).await;
        let mut media = client.claim_native_datagrams().unwrap();
        let control = client.request_value(
            "operation.get",
            json!({"operation_id":"op_probe"}),
            CommandScope::default(),
        );
        let packets = async {
            let _clock = media.receive().await?;
            let packet = media.receive().await?;
            let before = media.connection.datagram_send_buffer_space();
            let expired = media
                .send_until(
                    &packet,
                    tokio::time::Instant::now() - Duration::from_millis(1),
                )
                .await;
            let after = media.connection.datagram_send_buffer_space();
            let until = tokio::time::Instant::now() + Duration::from_millis(200);
            media.send_until(&packet, until).await?;
            Ok::<_, ClientError>((
                matches!(expired, Err(ClientError::Timeout)),
                before == after,
            ))
        };
        // Preserve errors until both exact owners have actually joined. The
        // existing signed ACK is withheld until the server receives the echo.
        let (control, packets) = tokio::join!(control, packets);
        drop(media);
        let shutdown = client.shutdown().await;
        let joined = server.await;
        let facts = packets.unwrap();
        control.unwrap().ack().unwrap();
        assert_eq!(facts, (true, true));
        assert!(shutdown.is_ok());
        assert!(joined.is_ok());
    }

    #[tokio::test]
    async fn native_io_capacity_wait_cancellation_and_wake_recheck_expiry_or_close_before_enqueue()
    {
        for close in [false, true] {
            let (client, server, _) = controlled(Fault::NativeDuplexDuringControl).await;
            let mut media = client.claim_native_datagrams().unwrap();
            let bytes =
                rvoip_uctp::substrate::pack_rtp_datagram(&rvoip_uctp::substrate::RtpDatagram {
                    flags: 0,
                    stream_local_id: 7,
                    seq: 1,
                    rtp: rvoip_uctp::substrate::RtpMediaPayload {
                        payload: vec![0x55; 1000].into(),
                        payload_type: 0,
                        sequence_number: 1,
                        timestamp: 160,
                        ssrc: 41,
                    },
                })
                .unwrap();
            // This default current-thread runtime intentionally does not yield
            // its real Quinn driver during the bounded setup/poll segment.
            // Fill only this test-owned transport using non-evicting admission.
            let mut full = false;
            for _ in 0..2048 {
                match media.send(&bytes) {
                    Ok(()) => {}
                    Err(ClientError::MediaBackpressure) => {
                        full = true;
                        break;
                    }
                    Err(_) => break,
                }
            }
            let space = media.connection.datagram_send_buffer_space();
            let cancelled_pending = {
                let mut pending = std::pin::pin!(media.send_until(
                    &bytes,
                    tokio::time::Instant::now() + Duration::from_millis(200)
                ));
                matches!(
                    std::future::Future::poll(
                        pending.as_mut(),
                        &mut std::task::Context::from_waker(std::task::Waker::noop())
                    ),
                    std::task::Poll::Pending
                )
                // Dropping the pending future submits no bytes and creates no
                // detached sender to resume after later driver capacity wake.
            };
            let unchanged = space == media.connection.datagram_send_buffer_space();
            if !close {
                media.expires_at = chrono::Utc::now() + chrono::Duration::milliseconds(20);
            }
            let terminal = {
                let mut pending = std::pin::pin!(media.send_until(
                    &bytes,
                    tokio::time::Instant::now() + Duration::from_millis(200)
                ));
                let before = std::future::Future::poll(
                    pending.as_mut(),
                    &mut std::task::Context::from_waker(std::task::Waker::noop()),
                );
                if close {
                    client.close();
                } else {
                    // Only the test-owned driver is paused; real UTC expiry
                    // passes without draining the full queue before repoll.
                    std::thread::sleep(Duration::from_millis(30));
                }
                let after = std::future::Future::poll(
                    pending.as_mut(),
                    &mut std::task::Context::from_waker(std::task::Waker::noop()),
                );
                (
                    matches!(before, std::task::Poll::Pending),
                    if close {
                        matches!(after, std::task::Poll::Ready(Err(ClientError::Transport)))
                    } else {
                        matches!(
                            after,
                            std::task::Poll::Ready(Err(ClientError::Authentication))
                        )
                    },
                )
            };
            drop(media);
            let shutdown = client.shutdown().await;
            let joined = server.await;
            assert!(
                full && cancelled_pending && unchanged,
                "actual non-evicting queue/cancel proof missing"
            );
            assert_eq!(terminal, (true, true));
            assert!(shutdown.is_ok());
            assert!(joined.is_ok());
        }
    }
}

/// One SDK child owns the SAME authenticated physical datagram reader. Unlike
/// the public bounded receive API, idle SDK media has no packet deadline.
/// This is crate-private raw IO, never stream ownership or new authority.
pub(crate) struct OwnedNativeIo {
    connection: quinn::Connection,
    closed: Arc<AtomicBool>,
    expires_at: chrono::DateTime<chrono::Utc>,
    status: super::owned_signaling::NativeStatus,
    _reader: ReaderClaim,
}
impl OwnedNativeIo {
    pub(super) fn current(&self) -> Result<()> {
        if self.closed.load(Ordering::Acquire) || self.connection.close_reason().is_some() {
            return Err(self
                .status
                .terminal_error()
                .unwrap_or(ClientError::Transport));
        }
        if chrono::Utc::now() >= self.expires_at {
            self.closed.store(true, Ordering::Release);
            self.connection
                .close(0u32.into(), b"canonical authority expired");
            return Err(ClientError::Authentication);
        }
        Ok(())
    }
    pub(crate) fn terminal_error(&self) -> Option<ClientError> {
        self.status.terminal_error()
    }
    pub(crate) fn send(&self, bytes: &[u8]) -> Result<()> {
        super::native::datagram(bytes)?;
        self.submit(bytes)
    }
    pub(crate) fn send_feedback(&self, bytes: &[u8]) -> Result<()> {
        rvoip_uctp::substrate::observe_video_feedback_datagram(bytes)
            .map_err(|_| ClientError::Protocol)?;
        self.submit(bytes)
    }
    fn submit(&self, bytes: &[u8]) -> Result<()> {
        self.current()?;
        if self
            .connection
            .max_datagram_size()
            .is_none_or(|n| bytes.len() > n)
        {
            return Err(ClientError::Protocol);
        }
        super::native_admission::try_submit(&self.connection, bytes)
    }
    pub(crate) async fn receive(&self) -> Result<Vec<u8>> {
        self.current()?;
        let bytes = self.connection.read_datagram().await.map_err(|_| {
            self.status
                .terminal_error()
                .unwrap_or(ClientError::Transport)
        })?;
        self.current()?;
        if bytes.len() > super::native::MAX_NATIVE_MEDIA_DATAGRAM + 16 {
            return Err(ClientError::Protocol);
        }
        Ok(bytes.to_vec())
    }
}
impl CanonicalClient {
    pub(crate) fn start_owned_native_io(&mut self) -> Result<OwnedNativeIo> {
        self.check_current()?;
        let reader = ReaderClaim::take(&self.native_datagram_reader)?;
        self.start_signaling_reader()?;
        // A successful start installs exactly this peer's original reader.
        // No fallible setup follows creation of that new owned task.
        let status = self
            .owned_signaling
            .as_ref()
            .expect("started owned signaling")
            .native_status();
        Ok(OwnedNativeIo {
            connection: self.connection.clone(),
            closed: self.closed.clone(),
            expires_at: self.authenticated_until.min(self.server_key_expires_at),
            status,
            _reader: reader,
        })
    }
    #[cfg(test)]
    pub(crate) fn take_owned_native_io(&self) -> Result<OwnedNativeIo> {
        self.check_current()?;
        let status = self
            .owned_signaling
            .as_ref()
            .ok_or(ClientError::Protocol)?
            .native_status();
        Ok(OwnedNativeIo {
            connection: self.connection.clone(),
            closed: self.closed.clone(),
            expires_at: self.authenticated_until.min(self.server_key_expires_at),
            status,
            _reader: ReaderClaim::take(&self.native_datagram_reader)?,
        })
    }
}
