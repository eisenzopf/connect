//! Physical MCP TCP admission and finite IO lifetime precede HTTP parsing.
use connect_service::resources::ResourceBudget;
use std::{
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::OwnedSemaphorePermit,
    time::{Instant, Sleep},
};
use tokio_util::sync::CancellationToken;

const IDLE: Duration = Duration::from_secs(30);
const LIFETIME: Duration = Duration::from_secs(120);
// Leave time for the HTTP driver and parent task to join inside the existing
// five-second listener shutdown deadline.
const DRAIN: Duration = Duration::from_secs(1);

pub(crate) struct BoundedListener {
    pub listener: TcpListener,
    pub capacity: Arc<ResourceBudget>,
    pub cancellation: CancellationToken,
}

pub(crate) struct BoundedStream {
    stream: TcpStream,
    _slot: OwnedSemaphorePermit,
    cancelled: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    idle: Pin<Box<Sleep>>,
    lifetime: Pin<Box<Sleep>>,
    drain: Option<Pin<Box<Sleep>>>,
}
impl BoundedStream {
    fn check(&mut self, cx: &mut Context<'_>) -> io::Result<()> {
        if self.idle.as_mut().poll(cx).is_ready() || self.lifetime.as_mut().poll(cx).is_ready() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "MCP connection expired",
            ));
        }
        if self
            .cancelled
            .as_mut()
            .is_some_and(|future| future.as_mut().poll(cx).is_ready())
        {
            self.cancelled = None;
            self.drain = Some(Box::pin(tokio::time::sleep(DRAIN)));
        }
        if self
            .drain
            .as_mut()
            .is_some_and(|timer| timer.as_mut().poll(cx).is_ready())
        {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "MCP listener draining",
            ));
        }
        Ok(())
    }
    fn activity(&mut self) {
        self.idle.as_mut().reset(Instant::now() + IDLE);
    }
}
impl AsyncRead for BoundedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if let Err(error) = self.check(cx) {
            return Poll::Ready(Err(error));
        }
        let before = buffer.filled().len();
        let result = Pin::new(&mut self.stream).poll_read(cx, buffer);
        if matches!(&result, Poll::Ready(Ok(()))) && buffer.filled().len() > before {
            self.activity();
        }
        result
    }
}
impl AsyncWrite for BoundedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if let Err(error) = self.check(cx) {
            return Poll::Ready(Err(error));
        }
        let result = Pin::new(&mut self.stream).poll_write(cx, bytes);
        if matches!(&result, Poll::Ready(Ok(count)) if *count > 0) {
            self.activity();
        }
        result
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Err(error) = self.check(cx) {
            return Poll::Ready(Err(error));
        }
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Err(error) = self.check(cx) {
            return Poll::Ready(Err(error));
        }
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
impl axum::serve::Listener for BoundedListener {
    type Io = BoundedStream;
    type Addr = SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let accepted = tokio::select! { biased;
                _ = self.cancellation.cancelled() => std::future::pending().await,
                result = self.listener.accept() => result,
            };
            let (stream, address) = match accepted {
                Ok(value) => value,
                Err(error) => {
                    if !matches!(
                        error.kind(),
                        io::ErrorKind::ConnectionRefused
                            | io::ErrorKind::ConnectionAborted
                            | io::ErrorKind::ConnectionReset
                    ) {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                    continue;
                }
            };
            if self.cancellation.is_cancelled() {
                drop(stream);
                continue;
            }
            let slot = match self.capacity.clone().try_acquire_owned() {
                Ok(value) => value,
                Err(_) => {
                    drop(stream);
                    tokio::task::yield_now().await;
                    continue;
                }
            };
            return (
                BoundedStream {
                    stream,
                    _slot: slot,
                    cancelled: Some(Box::pin(self.cancellation.clone().cancelled_owned())),
                    idle: Box::pin(tokio::time::sleep(IDLE)),
                    lifetime: Box::pin(tokio::time::sleep(LIFETIME)),
                    drain: None,
                },
                address,
            );
        }
    }
    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }
}
