//! Original pre-cut B custody, recorded before private A/audio preparation.
use super::*;
use crate::canonical_replacement::{continuation::Phase, start::StartedReplacement};
use rvoip_core::{ids::StreamId, peer_admission::PeerAdmission};
use std::future::Future;

// This is the only local boundary between synchronous current admission and
// native/SQL observation. Even an ownership refusal releases every captured
// Reader and coordinator. The owner itself lives in the original Root, never
// solely in this future, before the first native poll.
async fn own_then_observe<C, N>(
    captured: C,
    own: impl FnOnce(&C) -> Result<()>,
    native: impl Future<Output = Result<N>>,
    bind: impl FnOnce(N) -> Result<()>,
    record: impl Future<Output = Result<()>>,
) -> Result<()> {
    let owned = own(&captured);
    drop(captured);
    owned?;
    let original = native.await?;
    bind(original)?;
    record.await
}

impl Runtime {
    pub(in crate::canonical_replacement) fn prepare_public_video_reservation<'a>(
        &'a self,
        root: &'a Arc<PreparedReplacement>,
        started: &'a Arc<StartedReplacement>,
        peer: &'a Arc<PeerAdmission>,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if !root.public_video.required() {
                return Ok(());
            }
            if !matches!(
                &started.lifecycle,
                super::super::webrtc::ReplacementLifecycle::WebRtc(_)
            ) {
                return Err(Error::forbidden());
            }
            let stored = root.public_video.reservation()?.clone();
            let public_stream = StreamId::from_string(&stored.intent().public_stream_id);
            let cancellation = root.handle.cancellation();
            let mut until = root.deadline;
            // This is a one-shot resource preparation, not a source/receive
            // grant. A later observation may reuse the already-owned object;
            // it must never ask Core to remint a dropped one-shot reservation.
            let captured = if root.public_video.reserved().is_ok() {
                None
            } else {
                let (roles, invocation) = super::super::projection::read(
                    &self.role_projection,
                    &self.stop,
                    until,
                    || async {
                        let invocation=self.continuation_read_once(root,started,Phase::ReserveVideo).await?;
                        match self.core.try_prepare_staged_public_video_reservation_with_held(
                            &root.handle,&public_stream,&invocation,
                        ).map_err(|_|Error::forbidden())? {
                            Some(reserved)=>root.public_video.own_reserved(root,reserved)?,
                            None=>return Err(Error::new("authority-busy","Original B reservation metadata is busy before one-shot installation")),
                        }
                        Ok(invocation)
                    },
                )
                .await?;
                until = until.min(invocation.deadline);
                Some((roles, invocation))
            };
            own_then_observe(
                captured,
                |captured| {
                    // Resource ownership was installed synchronously inside
                    // the exact successful admission above. No owner exists
                    // after a None outcome; projection drops that original
                    // Reader/coordinator before any fresh effect-free retry.
                    let _=captured;
                    if self.stop.is_cancelled() || cancellation.is_cancelled()
                        || tokio::time::Instant::now() >= until
                        || !root.public_video.reserved()?.is_current()
                    {
                        return Err(Error::forbidden());
                    }
                    Ok(())
                },
                async {
                    if let Some(original) = root.public_video.negotiated.get() {
                        return Ok(original.clone());
                    }
                    let adapter = self.webrtc.get().ok_or_else(Error::forbidden)?.adapter();
                    // `peer` is the SAME original Arc retained by the owned
                    // preparation worker, not a registry/current-peer lookup.
                    // This accessor observes scalars only; cancelling it never
                    // detaches a native worker or starts a B writer.
                    tokio::select! { biased;
                        _ = self.stop.cancelled() => Err(Error::forbidden()),
                        _ = cancellation.cancelled() => Err(Error::forbidden()),
                        _ = tokio::time::sleep_until(until) => Err(Error::new("timeout", "Original B negotiation cutoff elapsed")),
                        native = adapter.reserved_video_port_binding(
                            &root.handle.intent().replacement, peer, until,
                        ) => native.map_err(|_| Error::forbidden()),
                    }
                },
                |native| {
                    if self.stop.is_cancelled() || cancellation.is_cancelled()
                        || tokio::time::Instant::now() >= until
                    {
                        return Err(Error::forbidden());
                    }
                    root.public_video.bind_negotiated(root, native)
                },
                self.record_public_video_phase(
                    root,
                    started,
                    connect_store::NativeReplacementPublicVideoStage::NegotiatedNeverFed,
                ),
            )
            .await
        })
    }
}

#[cfg(test)]
mod tests;
