//! Receive-independent cancellation of one owned live media connection.
use rvoip_uctp::{envelope::UctpEnvelope, substrate::PeerMediaRouter};
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};
use tokio::{sync::broadcast, task::JoinHandle};
use tokio_util::sync::CancellationToken;

pub(super) struct Binding {
    pub conversation_id: String,
    pub session_id: String,
    pub connection_id: String,
}

pub(super) fn observe(
    mut events: broadcast::Receiver<UctpEnvelope>,
    connection: quinn::Connection,
    router: Arc<PeerMediaRouter>,
    cancellation: CancellationToken,
    reason: Arc<AtomicU8>,
    binding: Arc<Binding>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let cause = loop {
            tokio::select! {
                _ = cancellation.cancelled() => break 1,
                _ = connection.closed() => break 2,
                event = events.recv() => match event {
                    Ok(event) => match crate::media_end::is_terminal(
                        &event, &binding.conversation_id, &binding.session_id, &binding.connection_id,
                    ) {
                        Ok(false) => {},
                        Ok(true) => break 1,
                        Err(_) => break 3,
                    },
                    // Losing control events cannot leave an unobservable media
                    // worker running indefinitely. Rejoin with current authority.
                    Err(_) => break 2,
                }
            }
        };
        // Publish why the leg stopped before waking its data-plane consumers.
        // A lost control stream/server is not a successful remote hangup.
        let _ = reason.compare_exchange(0, cause, Ordering::AcqRel, Ordering::Acquire);
        if cause != 1 {
            connection.close(5_u32.into(), b"native-media-control-lost");
        }
        cancellation.cancel();
        router.remove_connection(&rvoip_uctp::substrate::PeerMediaConnectionKey::new(
            rvoip_core::ids::SessionId::from_string(&binding.session_id),
            rvoip_core::ids::ConnectionId::from_string(&binding.connection_id),
        ));
    })
}
