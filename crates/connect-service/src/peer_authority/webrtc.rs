//! One immutable WebRTC answer, separate from native-prepared datagram grants.
use super::*;
use connect_protocol::Error;
use rvoip_core::stream_lifecycle::{MediaFormat, Medium, StreamDirection};
use rvoip_uctp::v2::{auth_dispatch::AuthorizedEnvelope, payloads::ConnectionAnswer};

pub(crate) fn bind_answer(
    store: &Store,
    a: &AuthorizedEnvelope,
    answer: &ConnectionAnswer,
) -> connect_protocol::Result<()> {
    let authority = a
        .authority::<StoreInvocationAuthority>()
        .ok_or_else(Error::forbidden)?;
    let grant = authority
        .guard::<Arc<Grant>>()
        .ok_or_else(Error::forbidden)?;
    let PeerCommand::NativeConnectionOffer(intent) = &grant.request.command else {
        return Err(Error::forbidden());
    };
    if crate::canonical_native::intent(a.envelope()).as_ref() != Ok(intent)
        || a.envelope().id() != grant.invocation_id
        || !grant.proof.current()
        || intent.endpoint_transport()? != connect_protocol::provisioning::EndpointTransport::Webrtc
    {
        return Err(Error::forbidden());
    }
    let view =
        store.project_connection_with_peer(authority.binding().credential_id(), &grant.proof)?;
    let base = crate::canonical_native::base_profile()?;
    let (_, plan) =
        crate::canonical_media::offer_plan(&view, &base, a.principal()).map_err(|_| {
            Error::new(
                "incompatible-capabilities",
                "WebRTC canonical stream plan refused",
            )
        })?;
    let payload = serde_json::to_value(answer).map_err(|_| Error::forbidden())?;
    validate(&payload, &plan, grant.proof.native_member().map(|m| m.0))?;
    let mut saved = grant
        .webrtc_answer
        .try_lock()
        .map_err(|_| Error::forbidden())?;
    if saved.is_some() {
        return Err(Error::new("conflict", "WebRTC answer already bound"));
    }
    *saved = Some(payload);
    Ok(())
}
fn validate(
    payload: &serde_json::Value,
    plan: &rvoip_uctp::v2::native_negotiation::NativeOfferPlan,
    participant: Option<&str>,
) -> connect_protocol::Result<()> {
    let bad = || {
        Error::new(
            "incompatible-capabilities",
            "WebRTC answer differs from canonical stream plan",
        )
    };
    let sdp = payload["substrate_setup"]["sdp"].as_str().ok_or_else(bad)?;
    if payload["substrate"] != "websocket+webrtc"
        || payload["substrate_setup"]["kind"] != "websocket+webrtc"
        || payload["by_participant"].as_str() != participant
        || !sdp.starts_with("v=0\r\n")
        || sdp.contains('\0')
        || sdp.len() > 512 * 1024
    {
        return Err(bad());
    }
    let streams = payload["streams_answered"].as_array().ok_or_else(bad)?;
    if streams.is_empty() || streams.len() != plan.selected().len() || streams.len() > 32 {
        return Err(bad());
    }
    let mut ids = std::collections::BTreeSet::new();
    for stream in streams {
        let id = stream["id"].as_str().ok_or_else(bad)?;
        if !ids.insert(id) {
            return Err(bad());
        }
        let selected = plan
            .selected()
            .iter()
            .find(|s| s.requested.logical_id.as_str() == id)
            .ok_or_else(bad)?;
        let kind = match selected.requested.medium {
            Medium::Audio => "audio",
            Medium::Video => "video",
            Medium::Screen => "screen",
            Medium::Data => "data",
        };
        let direction = match selected.requested.direction {
            StreamDirection::Send => "sendonly",
            StreamDirection::Receive => "recvonly",
            StreamDirection::Both => "sendrecv",
        };
        if stream["kind"] != kind || stream["direction"] != direction {
            return Err(bad());
        }
        let codec = &stream["codec"];
        match &selected.format {
            MediaFormat::Encoded {
                name,
                clock_rate_hz,
                channels,
                ..
            } => {
                let normalized = match codec["name"].as_str().ok_or_else(bad)? {
                    "g.711-mu" => "pcmu",
                    "g.711-a" => "pcma",
                    v => v,
                };
                if normalized != name
                    || codec["params"]["sample_rate"].as_u64() != Some(*clock_rate_hz as u64)
                    || codec["params"]["channels"].as_u64() != Some(*channels as u64)
                    || codec["params"]["payload_type"]
                        .as_u64()
                        .is_none_or(|p| p > 127)
                {
                    return Err(bad());
                }
            }
            MediaFormat::ReliableOrdered(p) => {
                let name = match p {
                    rvoip_core::capability::DataProtocol::Text => "text",
                    rvoip_core::capability::DataProtocol::Json => "json",
                    rvoip_core::capability::DataProtocol::Binary => "binary",
                };
                if codec["name"] != name || codec["params"]["reliability"] != "reliable_ordered" {
                    return Err(bad());
                }
            }
        }
    }
    Ok(())
}
impl StorePeerAuthority {
    pub(super) fn webrtc_outgoing(
        &self,
        grant: &Grant,
        offer: &Envelope,
        answer: &Envelope,
    ) -> Result<(), DispatchError> {
        let PeerCommand::NativeConnectionOffer(intent) = &grant.request.command else {
            return Err(DispatchError::Authority);
        };
        if crate::canonical_native::intent(offer).as_ref() != Ok(intent)
            || offer.id() != grant.invocation_id
            || answer.kind() != "connection.answer"
            || answer.scope(Scope::Reply) != Some(offer.id())
        {
            return Err(DispatchError::Authority);
        }
        self.native_scopes(&grant.request.command, answer)?;
        let saved = grant
            .webrtc_answer
            .try_lock()
            .map_err(|_| DispatchError::Authority)?;
        if saved.as_ref() != Some(&answer.original()["payload"]) {
            return Err(DispatchError::Authority);
        }
        Ok(())
    }
}
