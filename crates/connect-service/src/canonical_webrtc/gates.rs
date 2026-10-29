//! Gates on original confirmed RTC bindings, under the signed command lease.
use super::*;
use crate::canonical_media::streams::RtcGatePlan;
use connect_store::NativeStreamProjection;
use rvoip_core::{
    ids::StreamId,
    stream_lifecycle::{Medium, StreamDirection, StreamGateAction},
};
use rvoip_uctp::v2::payloads::{ConnectionUpdateAction, Payload};
use std::collections::BTreeSet;
impl Runtime {
    pub(super) fn rtc_gate_plan(
        &self,
        a: &AuthorizedEnvelope,
        id: &ConnectionId,
        source: &NativeStreamProjection,
    ) -> Result<RtcGatePlan> {
        let Payload::ConnectionUpdate(update) = a.envelope().payload() else {
            return Err(Error::forbidden());
        };
        connect_protocol::native_gates::validate(&a.envelope().original()["payload"])?;
        let expected = a
            .envelope()
            .expected_revision()
            .ok_or_else(Error::forbidden)?;
        let controller = self
            .webrtc
            .get()
            .and_then(|h| h.adapter().staged_controller(id))
            .ok_or_else(Error::forbidden)?;
        if expected != source.stream_revision || controller.revision() != expected {
            return Err(Error::new("conflict", "RTC gate revision conflicts"));
        }
        let bindings = controller.active_bindings();
        if bindings.is_empty()
            || bindings.iter().any(|b| {
                !matches!(
                    b.negotiated.requested.medium,
                    Medium::Audio | Medium::Video | Medium::Screen
                )
            })
        {
            return Err(Error::new(
                "unsupported-media-profile",
                "RTC gate profile requires confirmed audio/VP8 bindings",
            ));
        }
        // Wire directions belong to the endpoint, while Core owns the reversed
        // server port. Hold changes gates only; it never renegotiates a binding.
        let action = match update.action {
            ConnectionUpdateAction::Hold => match update
                .details
                .as_ref()
                .and_then(|v| v["direction"].as_str())
            {
                Some("inactive") => StreamGateAction::Pause,
                Some("sendonly") => StreamGateAction::Hold {
                    direction: StreamDirection::Receive,
                },
                Some("recvonly") | None => StreamGateAction::Hold {
                    direction: StreamDirection::Send,
                },
                _ => return Err(Error::new("invalid-request", "Invalid RTC hold direction")),
            },
            ConnectionUpdateAction::Resume => StreamGateAction::Resume,
            ConnectionUpdateAction::Mute => StreamGateAction::Mute,
            ConnectionUpdateAction::Unmute => StreamGateAction::Unmute,
            _ => {
                return Err(Error::new(
                    "unsupported-media-profile",
                    "RTC update requires another control profile",
                ))
            }
        };
        let streams: BTreeSet<StreamId> = match &update.streams {
            Some(streams) => streams.iter().map(|id| StreamId::from_string(id)).collect(),
            None => bindings
                .iter()
                .map(|b| b.negotiated.requested.logical_id.clone())
                .collect(),
        };
        if streams.iter().any(|id| {
            !bindings
                .iter()
                .any(|b| &b.negotiated.requested.logical_id == id)
        }) {
            return Err(Error::new("not-found", "RTC gate stream is unavailable"));
        }
        Ok(RtcGatePlan {
            expected,
            streams,
            action,
        })
    }
}
