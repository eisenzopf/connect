//! The actual engine planner installs one immutable bounded negotiation plan.
//! Signed counteroffers and replies cannot borrow the initial offer grant.
use super::*;
use rvoip_core::stream_lifecycle::{MediaFormat, Medium, StreamDirection};
use rvoip_uctp::v2::{auth_dispatch::AuthorizedEnvelope, native_negotiation::NativeOfferPlan};
use serde_json::{json, Value};

pub(super) struct Planned {
    plan: NativeOfferPlan,
    substrate: &'static str,
}
pub(crate) fn bind_plan(
    a: &AuthorizedEnvelope,
    plan: &NativeOfferPlan,
    transport: rvoip_core::Transport,
) -> connect_protocol::Result<()> {
    let authority = a
        .authority::<StoreInvocationAuthority>()
        .ok_or_else(connect_protocol::Error::forbidden)?;
    let grant = authority
        .guard::<Arc<Grant>>()
        .ok_or_else(connect_protocol::Error::forbidden)?;
    let PeerCommand::NativeStream(wire) = &grant.request.command else {
        return Err(connect_protocol::Error::forbidden());
    };
    if crate::canonical_native_streams::intent(a.envelope()).as_ref() != Ok(wire)
        || a.envelope().id() != grant.invocation_id
        || !grant.proof.current()
    {
        return Err(connect_protocol::Error::forbidden());
    }
    let mut saved = grant
        .native_stream_plan
        .try_lock()
        .map_err(|_| connect_protocol::Error::forbidden())?;
    if saved.is_some() {
        return Err(connect_protocol::Error::new(
            "conflict",
            "Native stream negotiation already planned",
        ));
    }
    let endpoint = crate::canonical_runtime::endpoint_transport(transport)?;
    *saved = Some(Planned {
        plan: plan.clone(),
        substrate: endpoint.as_str(),
    });
    Ok(())
}
pub(crate) fn codec(format: &MediaFormat) -> Result<Value, DispatchError> {
    match format {
        MediaFormat::Encoded {
            name,
            clock_rate_hz,
            channels,
            fmtp,
        } => {
            let pt = match name.as_str() {
                "opus" => 111,
                "pcmu" => 0,
                "pcma" => 8,
                "vp8" => 96,
                _ => return Err(DispatchError::Unsupported),
            };
            let mut params =
                json!({"sample_rate":clock_rate_hz,"channels":channels,"payload_type":pt});
            if let Some(fmtp) = fmtp {
                params["fmtp"] = json!(fmtp);
            }
            Ok(json!({"name":name,"params":params}))
        }
        MediaFormat::ReliableOrdered(protocol) => {
            let name = match protocol {
                rvoip_core::capability::DataProtocol::Text => "text",
                rvoip_core::capability::DataProtocol::Json => "json",
                rvoip_core::capability::DataProtocol::Binary => "binary",
            };
            Ok(json!({"name":name,"params":{"reliability":"reliable_ordered"}}))
        }
    }
}
impl StorePeerAuthority {
    pub(super) fn native_stream_outgoing(
        &self,
        grant: &Grant,
        original: &Envelope,
        frame: &Envelope,
        answer: bool,
    ) -> Result<(), DispatchError> {
        let PeerCommand::NativeStream(wire) = &grant.request.command else {
            return Err(DispatchError::Authority);
        };
        if answer
            || crate::canonical_native_streams::intent(original).as_ref() != Ok(wire)
            || original.id() != grant.invocation_id
        {
            return Err(DispatchError::Authority);
        }
        self.native_scopes(&grant.request.command, frame)?;
        let plan = grant
            .native_stream_plan
            .try_lock()
            .map_err(|_| DispatchError::Authority)?;
        let saved = plan.as_ref().ok_or(DispatchError::Authority)?;
        let plan = &saved.plan;
        let p = &frame.original()["payload"];
        let setup = &p["substrate_setup"];
        if frame.kind() != "connection.offer"
            || frame.scope(Scope::Reply).is_some()
            || p["substrate"].as_str() != Some(saved.substrate)
            || p["by_participant"].as_str() != grant.proof.native_member().map(|v| v.0)
            || setup["kind"] != "native-prepared/1"
            || setup["offer_id"] != original.id()
            || setup["revision"].as_u64() != wire.expected_revision.and_then(|r| r.checked_add(1))
        {
            return Err(DispatchError::Authority);
        }
        let streams = setup["streams"]
            .as_array()
            .ok_or(DispatchError::Authority)?;
        if streams.is_empty() || streams.len() > 32 || streams.len() != plan.selected().len() {
            return Err(DispatchError::Capacity);
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut handles = std::collections::BTreeSet::new();
        for stream in streams {
            let id = stream["strm_id"].as_str().ok_or(DispatchError::Authority)?;
            let selected = plan
                .selected()
                .iter()
                .find(|s| s.requested.logical_id.as_str() == id)
                .ok_or(DispatchError::Authority)?;
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
            let handle = stream["stream_local_id"]
                .as_u64()
                .ok_or(DispatchError::Authority)?;
            if !ids.insert(id)
                || !handles.insert(handle)
                || !(1..=65535).contains(&handle)
                || stream["kind"] != kind
                || stream["direction"] != direction
                || stream["codec"] != codec(&selected.format)?
            {
                return Err(DispatchError::Authority);
            }
        }
        let expected=streams.iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":s["direction"],"codec_preferences":[s["codec"]["name"]]})).collect::<Vec<_>>();
        if p["streams_offered"] != json!(expected) {
            return Err(DispatchError::Authority);
        }
        Ok(())
    }
}
