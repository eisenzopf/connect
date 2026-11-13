//! One fixed child services the original media queues beside signed control.
use super::*;
use std::any::Any;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Preserve the original future while observing its panic; cleanup still runs.
pub(super) async fn catch_work<T>(
    work: impl Future<Output = T>,
) -> std::result::Result<T, Box<dyn Any + Send>> {
    let mut work = std::pin::pin!(work);
    std::future::poll_fn(
        |cx| match catch_unwind(AssertUnwindSafe(|| work.as_mut().poll(cx))) {
            Ok(std::task::Poll::Ready(value)) => std::task::Poll::Ready(Ok(value)),
            Ok(std::task::Poll::Pending) => std::task::Poll::Pending,
            Err(panic) => std::task::Poll::Ready(Err(panic)),
        },
    )
    .await
}

pub(super) async fn run(
    io: crate::canonical::OwnedNativeIo,
    cache: Arc<std::sync::Mutex<NativeCache>>,
    inbox: Arc<media::Inbox>,
    mut requests: mpsc::Receiver<MediaCommand>,
    stop: CancellationToken,
) -> ErrorKind {
    let mut pending: Option<media_admission::Pending> = None;
    let work = catch_work(async { loop {
        let wakeup = pending.as_ref().map(|p| p.wakeup());
        tokio::select! {
            _ = stop.cancelled() => break ErrorKind::Cancelled,
            packet = io.receive() => match packet {
                Ok(bytes) => if let Err(error) = inbox.push(bytes) { break error.kind; },
                Err(crate::ClientError::Protocol) => {
                    if let Some(cause) = io.terminal_error() { break Error::transport(cause, None).kind; }
                    if let Err(error) = inbox.malformed() { break error.kind; }
                },
                Err(error) => break Error::transport(error, None).kind,
            },
            _ = tokio::time::sleep_until(wakeup.unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(60))), if wakeup.is_some() => {
                let mut original = pending.take().expect("one original pending media owner");
                match original.try_send(&io, &cache, &stop) {
                    Ok(None) => pending = Some(original),
                    Ok(Some(value)) => original.finish(Ok(value)),
                    Err(error) => original.finish(Err(error)),
                }
            },
            command = requests.recv(), if pending.is_none() => {
                let Some(command) = command else { break ErrorKind::Cancelled; };
                if command.reply.is_closed() || command.signal.is_cancelled() || stop.is_cancelled()
                    || tokio::time::Instant::now() >= command.expires {
                    let kind = if tokio::time::Instant::now() >= command.expires { ErrorKind::Deadline } else { ErrorKind::Cancelled };
                    let _ = command.reply.send(Err(Error::media(kind, false)));
                    continue;
                }
                match &command.packet {
                    MediaSend::Packet(_) => {
                        let mut original = media_admission::Pending::new(command);
                        match original.try_send(&io, &cache, &stop) {
                            Ok(None) => pending = Some(original),
                            Ok(Some(value)) => original.finish(Ok(value)),
                            Err(error) => original.finish(Err(error)),
                        }
                    },
                    MediaSend::Keyframe(target) => {
                        // Exactly one feedback serial. No retry of an entered callback.
                        let result = native_binding(&cache, &command.key).and_then(|binding|
                            inbox.send_keyframe(&binding, target, |bytes|
                                io.send_feedback(bytes).map_err(|error|
                                    Error::media(Error::transport(error, None).kind, false))));
                        let _ = command.reply.send(result);
                    },
                }
            },
        }
        tokio::task::consume_budget().await;
    }}).await;
    let failure = match &work {
        Ok(kind) => *kind,
        Err(_) => ErrorKind::Protocol,
    };
    // Wake a parent currently inside any original signed write. The parent owns
    // and joins this exact child before it completes canonical shutdown.
    stop.cancel();
    inbox.end();
    if let Some(original) = pending.take() {
        original.finish(Err(Error::media(failure, false)));
    }
    requests.close();
    while let Some(command) = requests.recv().await {
        let _ = command.reply.send(Err(Error::media(failure, false)));
    }
    if let Err(panic) = work {
        std::panic::resume_unwind(panic);
    }
    failure
}
