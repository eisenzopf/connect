//! Validate both native negotiation legs before signing a same-root confirmation.
use super::*;
use crate::canonical::{CommandScope, Envelope, Scope};
use serde_json::json;
use std::collections::BTreeSet;
pub(super) struct Pending {
    pub scope: CommandScope,
    payload: Value,
    answer: Option<Value>,
    selected: Option<NativeNegotiation>,
}
impl Pending {
    pub fn new(payload: Value, scope: CommandScope) -> Self {
        Self {
            payload,
            scope,
            answer: None,
            selected: None,
        }
    }
    fn roots(&self, frame: &Envelope) -> Result<()> {
        for (field, expected) in [
            (Scope::Conversation, self.scope.conversation_id.as_deref()),
            (Scope::Session, self.scope.session_id.as_deref()),
            (Scope::Connection, self.scope.connection_id.as_deref()),
            (Scope::Operation, self.scope.operation_id.as_deref()),
        ] {
            if expected.is_none() || frame.scope(field) != expected {
                return Err(Error::local(ErrorKind::Protocol));
            }
        }
        Ok(())
    }
    pub fn answer(&mut self, frame: &Envelope, request_id: &str) -> Result<()> {
        let fail = || Error::local(ErrorKind::Protocol);
        self.roots(frame)?;
        if self.answer.is_some()
            || frame.kind() != "connection.answer"
            || frame.scope(Scope::Reply) != Some(request_id)
        {
            return Err(fail());
        }
        let p = &frame.original()["payload"];
        if p["substrate"] != "quic" {
            return Err(fail());
        }
        let _: ParticipantId =
            serde_json::from_value(p["by_participant"].clone()).map_err(|_| fail())?;
        if self
            .payload
            .get("by_participant")
            .is_some_and(|v| v != &p["by_participant"])
        {
            return Err(fail());
        }
        let setup = &p["substrate_setup"];
        if setup["kind"] != "native-prepared/1"
            || setup["offer_id"] != request_id
            || setup["revision"] != 1
        {
            return Err(fail());
        }
        let streams = setup["streams"]
            .as_array()
            .filter(|s| !s.is_empty() && s.len() <= 32)
            .ok_or_else(fail)?;
        let offered = self.payload["streams_offered"]
            .as_array()
            .ok_or_else(fail)?;
        let mut ids = BTreeSet::new();
        let mut handles = BTreeSet::new();
        let mut selected = Vec::new();
        let mut expected = Vec::new();
        for stream in streams {
            let id: StreamId =
                serde_json::from_value(stream["strm_id"].clone()).map_err(|_| fail())?;
            if !ids.insert(id.clone()) {
                return Err(fail());
            }
            let original = offered
                .iter()
                .find(|v| v["id"] == stream["strm_id"])
                .ok_or_else(fail)?;
            let medium: Medium =
                serde_json::from_value(stream["kind"].clone()).map_err(|_| fail())?;
            let server_direction: Direction =
                serde_json::from_value(stream["direction"].clone()).map_err(|_| fail())?;
            let local_direction = server_direction.reverse();
            if stream["kind"] != original["kind"]
                || serde_json::to_value(local_direction).map_err(|_| fail())?
                    != original["direction"]
            {
                return Err(fail());
            }
            let name = stream["codec"]["name"].as_str().ok_or_else(fail)?;
            let normalized = connections::codec_name(name);
            if !original["codec_preferences"]
                .as_array()
                .ok_or_else(fail)?
                .iter()
                .any(|v| {
                    v.as_str()
                        .is_some_and(|v| connections::codec_name(v) == normalized)
                })
            {
                return Err(fail());
            }
            let params = connections::codec_parameters(medium, &normalized).ok_or_else(fail)?;
            for (field, value) in params.as_object().ok_or_else(fail)? {
                if stream["codec"]["params"][field] != *value {
                    return Err(fail());
                }
            }
            let handle = if medium == Medium::Data {
                None
            } else {
                let handle = stream["stream_local_id"]
                    .as_u64()
                    .and_then(|v| u16::try_from(v).ok())
                    .filter(|v| *v > 0)
                    .ok_or_else(fail)?;
                if !handles.insert(handle) {
                    return Err(fail());
                }
                let pt = match normalized.as_str() {
                    "pcmu" => 0,
                    "pcma" => 8,
                    "opus" => 111,
                    "vp8" => 96,
                    _ => return Err(fail()),
                };
                if stream["codec"]["params"]["payload_type"] != pt {
                    return Err(fail());
                }
                Some(handle)
            };
            selected.push(NegotiatedStream {
                id,
                medium,
                direction: local_direction,
                codec: stream["codec"].clone(),
                stream_local_id: handle,
            });
            expected.push(json!({"id":stream["strm_id"],"kind":stream["kind"],"direction":stream["direction"],"codec":stream["codec"]}));
        }
        if p["streams_answered"] != json!(expected) {
            return Err(fail());
        }
        self.selected = Some(NativeNegotiation {
            source_clock: p["capabilities"]["source_clock"] == "rtcp-sr-sdes/1"
                && self.payload["capabilities"]["source_clock"] == "rtcp-sr-sdes/1",
            stream_revision: StreamRevision::try_from(1)?,
            streams: selected,
        });
        self.answer = Some(p.clone());
        Ok(())
    }
    pub fn confirmation(
        &self,
        frame: &Envelope,
    ) -> Result<crate::canonical::payloads::ConnectionAnswer> {
        let fail = || Error::local(ErrorKind::Protocol);
        self.roots(frame)?;
        if frame.kind() != "connection.offer" || frame.scope(Scope::Reply).is_some() {
            return Err(fail());
        }
        let original = self.answer.as_ref().ok_or_else(fail)?;
        let p = &frame.original()["payload"];
        for field in [
            "by_participant",
            "substrate",
            "substrate_setup",
            "capabilities",
        ] {
            if p[field] != original[field] {
                return Err(fail());
            }
        }
        let streams = p["substrate_setup"]["streams"]
            .as_array()
            .ok_or_else(fail)?;
        let offered = p["streams_offered"].as_array().ok_or_else(fail)?;
        if offered.len() != streams.len() {
            return Err(fail());
        }
        let mut answered = Vec::new();
        let mut ids = BTreeSet::new();
        for offer in offered {
            let id = offer["id"].as_str().ok_or_else(fail)?;
            if !ids.insert(id) {
                return Err(fail());
            }
            let stream = streams
                .iter()
                .find(|s| s["strm_id"] == id)
                .ok_or_else(fail)?;
            if offer["kind"] != stream["kind"]
                || offer["direction"] != stream["direction"]
                || offer["codec_preferences"] != json!([stream["codec"]["name"]])
            {
                return Err(fail());
            }
        }
        for stream in streams {
            let direction: Direction =
                serde_json::from_value(stream["direction"].clone()).map_err(|_| fail())?;
            answered.push(json!({"id":stream["strm_id"],"kind":stream["kind"],"direction":direction.reverse(),"codec":stream["codec"]}));
        }
        serde_json::from_value(json!({"by_participant":p["by_participant"],"substrate":"quic","capabilities":self.payload["capabilities"],"streams_answered":answered,"substrate_setup":p["substrate_setup"]})).map_err(|_|fail())
    }
    pub fn selected(&self) -> Result<&NativeNegotiation> {
        self.selected
            .as_ref()
            .ok_or_else(|| Error::local(ErrorKind::Protocol))
    }
}
