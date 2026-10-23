//! Owned target preparation: actual decode-only input and guarded local output.
use crate::{canonical_media::Connection, canonical_runtime::Runtime};
use rvoip_core::{
    adapter::ConnectionAdapter,
    ids::{ConnectionId, SessionId},
    staged_streams::local_audio::LocalAudioPurpose,
    stream_lifecycle::{MediaFormat, Medium},
};
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
impl Runtime {
    pub(crate) fn queue_handoff_preparation(
        self: &Arc<Self>,
        entry: Arc<Connection>,
        physical: CancellationToken,
    ) {
        let Ok(mut work) = self.work.lock() else {
            return;
        };
        while work.try_join_next().is_some() {}
        let runtime = self.clone();
        // This worker owns no peer/Store authority reader between its ticks.
        // EOF/Session/server retirement cancels it and drops its actual decoder.
        work.spawn(async move {
            let sid = SessionId::from_string(entry.origin.session_id().to_owned());
            let id = ConnectionId::from_string(entry.origin.runtime_connection_id().to_owned());
            let until = tokio::time::Instant::now() + Duration::from_secs(30);
            let mut tick = tokio::time::interval(Duration::from_millis(100));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {biased;
                    _=runtime.stop.cancelled()=>return,
                    _=physical.cancelled()=>return,
                    _=tick.tick()=>{}
                }
                if tokio::time::Instant::now() >= until {
                    return;
                }
                match runtime.core.prepare_staged_audio_endpoint(
                    &sid,
                    &id,
                    entry.origin.offer_record_id(),
                ) {
                    Ok(preparation) => {
                        if entry.set_preparation(preparation).is_err() {
                            return;
                        }
                        break;
                    }
                    Err(_) => continue,
                }
            }
            let Ok(preparation) = entry.preparation() else {
                return;
            };
            let Some(controller) = runtime.native.staged_controller(&id) else {
                return;
            };
            let bindings = controller.active_bindings();
            let Some(binding) = bindings
                .iter()
                .find(|b| b.negotiated.requested.medium == Medium::Audio)
            else {
                return;
            };
            let MediaFormat::Encoded { clock_rate_hz, .. } = &binding.negotiated.format else {
                return;
            };
            let rate = *clock_rate_hz;
            let logical = binding.negotiated.requested.logical_id.clone();
            let start = tokio::time::Instant::now();
            loop {
                tokio::select! {biased;
                    _=runtime.stop.cancelled()=>return,
                    _=physical.cancelled()=>return,
                    _=tick.tick()=>{}
                }
                if tokio::time::Instant::now() >= until {
                    return;
                }
                // A different audience commit may invalidate old IO stamps
                // without consuming this independent closed preparation. Keep
                // its bounded owner available to obtain fresh actual output.
                // The local writer independently rechecks the exact raw group,
                // port and current permission; consumed preparations fail shut.
                if preparation.has_duplex_io() {
                    continue;
                }
                // Actual writer completion, separately from this enqueue, mints
                // the output stamp. Missing/contended/current-denied IO remains
                // unready; it cannot steal the original call's speaking route.
                let timestamp = (start.elapsed().as_micros() * u128::from(rate) / 1_000_000) as u32;
                let _ = controller.try_queue_local_silence(
                    &logical,
                    LocalAudioPurpose::EndpointPreparation {
                        operation_id: entry.origin.offer_record_id().into(),
                    },
                    timestamp,
                );
            }
        });
    }
}
