//! Atomic non-evicting local enqueue. Pending is definite non-submission.
use super::*;
use std::{
    future::Future,
    task::{Context, Poll, Waker},
};

pub(super) fn try_submit(connection: &quinn::Connection, bytes: &[u8]) -> Result<()> {
    // Quinn send_datagram evicts older accepted-but-unsent datagrams. Its
    // wait variant calls the same atomic queue with drop=false. Poll ONCE:
    // Pending retains data only in the dropped future, never in that queue.
    let mut send = std::pin::pin!(connection.send_datagram_wait(bytes.to_vec().into()));
    let mut context = Context::from_waker(Waker::noop());
    match send.as_mut().poll(&mut context) {
        Poll::Ready(Ok(())) => Ok(()),
        Poll::Ready(Err(_)) => Err(ClientError::Transport),
        Poll::Pending => Err(ClientError::MediaBackpressure),
    }
}
