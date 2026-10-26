//! One protected HTTP/1.1 response owns authorization until the actual socket
//! closes. Hyper may copy/consume Body bytes earlier; Body EOS releases nothing.
use axum::{
    extract::connect_info::Connected,
    serve::{IncomingStream, Listener},
};
use connect_protocol::Error;
use connect_service::CanonicalHttpRecordResponse;
use std::{
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::OwnedSemaphorePermit,
    time::{Instant, Sleep},
};
use tokio_util::sync::CancellationToken;

/// The physical writer retains this owner even after Hyper consumes Body bytes.
pub(super) trait ProtectedResponse: Send {
    fn status(&self) -> u16;
    fn body(&self) -> connect_protocol::Result<&[u8]>;
    fn deadline(&self) -> Instant;
    fn current(&self) -> bool;
}
impl ProtectedResponse for CanonicalHttpRecordResponse {
    fn status(&self) -> u16 {
        self.status()
    }
    fn body(&self) -> connect_protocol::Result<&[u8]> {
        self.body()
    }
    fn deadline(&self) -> Instant {
        self.deadline()
    }
    fn current(&self) -> bool {
        self.current()
    }
}
struct Held {
    response: Box<dyn ProtectedResponse>,
    _capacity: Arc<OwnedSemaphorePermit>,
}
#[derive(Default)]
struct State {
    held: Option<Held>,
    closed: bool,
    waker: Option<Waker>,
}
#[derive(Clone)]
pub(super) struct Custody {
    state: Arc<Mutex<State>>,
    cancellation: CancellationToken,
}
impl std::fmt::Debug for Custody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HttpSocketCustody(redacted)")
    }
}
impl Custody {
    fn new(cancellation: CancellationToken) -> Self {
        Self {
            state: Arc::new(Mutex::new(State::default())),
            cancellation,
        }
    }
    pub(super) fn cancellation(&self) -> CancellationToken {
        self.cancellation.child_token()
    }
    pub(super) fn install<R: ProtectedResponse + 'static>(
        &self,
        response: R,
        capacity: Arc<OwnedSemaphorePermit>,
    ) -> Result<(u16, Vec<u8>), Error> {
        let mut state = self.state.lock().map_err(|_| Error::storage())?;
        if state.closed || state.held.is_some() || self.cancellation.is_cancelled() {
            return Err(Error::forbidden());
        }
        let status = response.status();
        let bytes = response.body()?.to_vec();
        state.held = Some(Held {
            response: Box::new(response),
            _capacity: capacity,
        });
        if let Some(waker) = state.waker.take() {
            waker.wake()
        }
        Ok((status, bytes))
    }
    fn close(&self) {
        self.cancellation.cancel();
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.closed = true;
            // Explicitly release even if request/ConnectInfo clones outlive IO.
            state.held.take();
            if let Some(waker) = state.waker.take() {
                waker.wake()
            }
        }
    }
}
pub(super) struct GuardedListener {
    pub(super) listener: TcpListener,
    pub(super) cancellation: CancellationToken,
    pub(super) tls: Option<Arc<rustls::ServerConfig>>,
    pub(super) handshakes: Arc<tokio::sync::Semaphore>,
}
type TlsStream = tokio_rustls::server::TlsStream<TcpStream>;
enum Io {
    Plain(TcpStream),
    Handshake(Pin<Box<dyn Future<Output = io::Result<TlsStream>> + Send>>),
    Tls(Box<TlsStream>),
}
pub(super) struct GuardedStream {
    stream: Io,
    custody: Custody,
    cancelled: Pin<Box<dyn Future<Output = ()> + Send>>,
    timer: Option<(Instant, Pin<Box<Sleep>>)>,
}
impl GuardedStream {
    fn ready(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Io::Handshake(future) = &mut self.stream {
            match future.as_mut().poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(stream)) => self.stream = Io::Tls(Box::new(stream)),
            }
        }
        Poll::Ready(Ok(()))
    }
    fn check(&mut self, cx: &mut Context<'_>) -> io::Result<()> {
        let mut expired = false;
        let mut state = self
            .custody
            .state
            .lock()
            .map_err(|_| io::Error::other("HTTP custody unavailable"))?;
        if let Some(held) = &state.held {
            // Protected bytes stop immediately. Other requests still need IO
            // to send their cancellation response during graceful shutdown.
            expired = self.cancelled.as_mut().poll(cx).is_ready();
            let deadline = held.response.deadline();
            if self.timer.as_ref().is_none_or(|(old, _)| *old != deadline) {
                self.timer = Some((deadline, Box::pin(tokio::time::sleep_until(deadline))))
            }
            if let Some((_, timer)) = &mut self.timer {
                expired |= timer.as_mut().poll(cx).is_ready()
            }
            expired |= !held.response.current();
        }
        if expired || state.closed {
            state.closed = true;
            state.held.take();
            self.custody.cancellation.cancel();
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "HTTP response authority ended",
            ));
        }
        state.waker = Some(cx.waker().clone());
        Ok(())
    }
}
impl Drop for GuardedStream {
    fn drop(&mut self) {
        self.custody.close()
    }
}
impl AsyncRead for GuardedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if let Err(e) = self.check(cx) {
            return Poll::Ready(Err(e));
        }
        match self.ready(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            Poll::Ready(Ok(())) => {}
        }
        match &mut self.stream {
            Io::Plain(stream) => Pin::new(stream).poll_read(cx, buffer),
            Io::Tls(stream) => Pin::new(&mut **stream).poll_read(cx, buffer),
            Io::Handshake(_) => unreachable!("handshake readiness checked"),
        }
    }
}
impl AsyncWrite for GuardedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if let Err(e) = self.check(cx) {
            return Poll::Ready(Err(e));
        }
        match self.ready(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            Poll::Ready(Ok(())) => {}
        }
        match &mut self.stream {
            Io::Plain(stream) => Pin::new(stream).poll_write(cx, bytes),
            Io::Tls(stream) => Pin::new(&mut **stream).poll_write(cx, bytes),
            Io::Handshake(_) => unreachable!("handshake readiness checked"),
        }
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Err(e) = self.check(cx) {
            return Poll::Ready(Err(e));
        }
        match self.ready(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            Poll::Ready(Ok(())) => {}
        }
        match &mut self.stream {
            Io::Plain(stream) => Pin::new(stream).poll_flush(cx),
            Io::Tls(stream) => Pin::new(&mut **stream).poll_flush(cx),
            Io::Handshake(_) => unreachable!("handshake readiness checked"),
        }
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // Keep custody until the connection driver drops this actual IO, even
        // when shutdown is pending. Rejected authority never permits more writes.
        if let Err(e) = self.check(cx) {
            return Poll::Ready(Err(e));
        }
        match self.ready(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            Poll::Ready(Ok(())) => {}
        }
        match &mut self.stream {
            Io::Plain(stream) => Pin::new(stream).poll_shutdown(cx),
            Io::Tls(stream) => Pin::new(&mut **stream).poll_shutdown(cx),
            Io::Handshake(_) => unreachable!("handshake readiness checked"),
        }
    }
}
impl Listener for GuardedListener {
    type Io = GuardedStream;
    type Addr = SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (stream, address) = match self.listener.accept().await {
                Ok(value) => value,
                Err(_) => {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    continue;
                }
            };
            // Bound kernel output buffering per HTTP connection as well as the
            // application batch budget; stalled readers must exert backpressure.
            if socket2::SockRef::from(&stream)
                .set_send_buffer_size(16 * 1024)
                .is_err()
            {
                continue;
            }
            let stream = if let Some(config) = &self.tls {
                let Ok(slot) = self.handshakes.clone().try_acquire_owned() else {
                    continue;
                };
                let acceptor = tokio_rustls::TlsAcceptor::from(config.clone());
                Io::Handshake(Box::pin(async move {
                    let _slot = slot;
                    tokio::time::timeout(std::time::Duration::from_secs(5), acceptor.accept(stream))
                        .await
                        .map_err(|_| {
                            io::Error::new(io::ErrorKind::TimedOut, "TLS handshake expired")
                        })?
                }))
            } else {
                Io::Plain(stream)
            };
            let cancellation = self.cancellation.child_token();
            let custody = Custody::new(cancellation.clone());
            return (
                GuardedStream {
                    stream,
                    custody,
                    cancelled: Box::pin(cancellation.cancelled_owned()),
                    timer: None,
                },
                address,
            );
        }
    }
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }
}
impl Connected<IncomingStream<'_, GuardedListener>> for Custody {
    fn connect_info(stream: IncomingStream<'_, GuardedListener>) -> Self {
        stream.io().custody.clone()
    }
}
