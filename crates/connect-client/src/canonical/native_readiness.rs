//! The actual verified native callback owns readiness identity. Decoded counts
//! remain an explicit application observation, never inferred from RTP or ACKs.
use super::*;
use crate::sdk::{native_negotiation::Pending, Direction, Medium};
use std::sync::Mutex;

const MAX_INTEGER: u64 = 9_007_199_254_740_991;
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Challenge {
    nonce: String,
    source_generation: u64,
    target_generation: u64,
    stream_revision: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    profile: String,
    operation_record_id: String,
    readiness: Challenge,
}
/// Exact selected receiving resource on the original private preparation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeReadinessStream {
    pub stream_id: String,
    pub stream_local_id: u16,
    pub payload_type: u8,
    pub native_egress: connect_protocol::replacement_activation::NativeEgressIdentity,
}
/// An opaque server-issued challenge, obtainable only after this physical
/// reader verifies both negotiation legs and writes their exact signed answer.
/// It grants no media permission and is not a decoded readiness observation.
#[derive(Clone)]
pub struct NativeReplacementReadiness {
    request: String,
    callback: Value,
    payload: Value,
    scope: CommandScope,
    source_connection: String,
    challenge: Challenge,
    audio: NativeReadinessStream,
    video: Option<NativeReadinessStream>,
}
impl NativeReplacementReadiness {
    pub fn record_id(&self) -> &str {
        self.payload["replacement_preparation"]["operation_record_id"]
            .as_str()
            .expect("validated original record")
    }
    pub fn connection_id(&self) -> Option<&str> {
        self.scope.connection_id.as_deref()
    }
    pub fn operation_id(&self) -> Option<&str> {
        self.scope.operation_id.as_deref()
    }
    pub fn audio(&self) -> &NativeReadinessStream {
        &self.audio
    }
    pub fn video(&self) -> Option<&NativeReadinessStream> {
        self.video.as_ref()
    }
    pub fn source_generation(&self) -> u64 {
        self.challenge.source_generation
    }
    pub fn target_generation(&self) -> u64 {
        self.challenge.target_generation
    }
    pub fn stream_revision(&self) -> u64 {
        self.challenge.stream_revision
    }
    /// Validate historical public membership from the original verified
    /// operation.updated payload against this physical reader's two signed
    /// negotiation legs. This is neither live authority nor observed IO.
    pub fn activation_from_completed(
        &self,
        status: &Value,
    ) -> Result<connect_protocol::replacement_activation::NativeActivationBoundary> {
        use connect_protocol::replacement_activation::{
            NativeActivationAudience as A, NativeActivationBoundary,
        };
        let b: NativeActivationBoundary =
            serde_json::from_value(status["result"]["native_activation"].clone())
                .map_err(|_| ClientError::Protocol)?;
        b.validate().map_err(|_| ClientError::Protocol)?;
        let result = &status["result"];
        if status["state"] != "completed"
            || status["command_type"] != "connection.replace"
            || status["record_id"] != self.payload["replacement_preparation"]["operation_record_id"]
            || status["record_id"] != b.operation_record_id
            || status["operation_id"].as_str() != self.operation_id()
            || Some(b.operation_id.as_str()) != self.operation_id()
            || b.source_connection_id != self.source_connection
            || Some(b.target_connection_id.as_str()) != self.connection_id()
            || result["connection_id"] != b.target_connection_id
            || result["replaces"] != b.source_connection_id
            || result["generation"] != b.target_generation
            || b.target_generation != self.target_generation()
            || result["readiness"]["stream_revision"] != b.stream_revision
            || b.stream_revision <= self.stream_revision()
            || result["activation_state"] != "ready"
        {
            return Err(ClientError::Protocol);
        }
        match &b.audience {
            A::Public { streams } => {
                let expected = std::iter::once(&self.audio)
                    .chain(self.video.iter())
                    .collect::<Vec<_>>();
                if streams.len() != expected.len() {
                    return Err(ClientError::Protocol);
                }
                for stream in expected {
                    streams
                        .iter()
                        .find(|c| c.stream_id == stream.stream_id)
                        .ok_or(ClientError::Protocol)?
                        .matches(
                            &stream.stream_id,
                            stream.stream_local_id,
                            &stream.native_egress,
                            self.target_generation(),
                        )
                        .map_err(|_| ClientError::Protocol)?;
                }
            }
            A::PrivateConsultation { .. } => {} // Explicitly has no public membership.
            A::Unsupported { .. } => return Err(ClientError::Protocol),
        }
        Ok(b)
    }
    /// Freeze actual non-concealed decoded PCM and, if selected, a decoded
    /// keyframe on this fresh decoder. Keep this value unchanged on recovery.
    /// The SDK cannot inspect a caller-owned codec: do not use packet counts,
    /// PLC, FEC, negotiation or operation completion as these observations.
    pub fn with_decoded(
        &self,
        audio_decoded_samples: u64,
        video: Option<NativeDecodedVideoReadiness>,
    ) -> Result<NativeReplacementReady> {
        if !(1..=MAX_INTEGER).contains(&audio_decoded_samples) {
            return Err(ClientError::Protocol);
        }
        let video = match (&self.video, video) {
            (None, None) => Value::Null,
            (Some(stream), Some(v))
                if v.decoded_keyframes > 0
                    && v.decoded_keyframes <= v.decoded_frames
                    && v.decoded_frames <= MAX_INTEGER =>
            {
                json!({"stream_id":stream.stream_id,"decoded_frames":v.decoded_frames,
                    "decoded_keyframes":v.decoded_keyframes,"decoder_startup":"fresh-peer-first-decoded-frame"})
            }
            _ => return Err(ClientError::Protocol),
        };
        let mut payload = self.payload.clone();
        payload["replacement_preparation"]["profile"] = json!("connect-replacement-ready-v1");
        payload["replacement_preparation"]["readiness"] = json!({
            "nonce":self.challenge.nonce,"source_generation":self.challenge.source_generation,
            "target_generation":self.challenge.target_generation,"stream_revision":self.challenge.stream_revision,
            "audio":{"stream_id":self.audio.stream_id,"decoded_samples":audio_decoded_samples},"video":video});
        Ok(NativeReplacementReady {
            preparation: self.clone(),
            payload,
        })
    }
}
/// Counts returned by an actual fresh video decoder. A native keyframe count is
/// required; this interface has no browser-counter fallback or inferred count.
#[derive(Clone, Copy, Debug)]
pub struct NativeDecodedVideoReadiness {
    pub decoded_frames: u64,
    pub decoded_keyframes: u64,
}
/// Frozen original readiness continuation. Cloning preserves the exact intent;
/// it does not create another allocation, media start or replacement operation.
#[derive(Clone)]
pub struct NativeReplacementReady {
    preparation: NativeReplacementReadiness,
    payload: Value,
}

#[derive(Default)]
pub(super) struct Shared {
    state: Mutex<Option<State>>,
}
struct State {
    request: String,
    scope: CommandScope,
    payload: Value,
    pending: Pending,
    answer: Option<Value>,
    callback: Option<Value>,
    expected_answer: Option<Value>,
    confirmed: bool,
    ended: bool,
    frozen: Option<Value>,
    required_video: Vec<String>,
    source_connection: String,
}
fn roots(scope: &CommandScope, frame: &Envelope) -> bool {
    [
        (Scope::Conversation, scope.conversation_id.as_deref()),
        (Scope::Session, scope.session_id.as_deref()),
        (Scope::Connection, scope.connection_id.as_deref()),
        (Scope::Operation, scope.operation_id.as_deref()),
    ]
    .into_iter()
    .all(|(kind, expected)| expected.is_some() && frame.scope(kind) == expected)
}
impl Shared {
    pub fn begin(
        &self,
        request: &str,
        scope: &CommandScope,
        payload: &Value,
        required_video: Vec<String>,
        source_connection: &str,
    ) -> Result<()> {
        if payload["replacement_preparation"]["profile"] != "connect-replacement-media-v1" {
            return Ok(());
        }
        let mut guard = self.state.lock().map_err(|_| ClientError::Protocol)?;
        if guard.is_some()
            || required_video.len() > 1
            || payload["replacement_preparation"]["operation_record_id"]
                .as_str()
                .is_none()
        {
            return Err(ClientError::Protocol);
        }
        *guard = Some(State {
            request: request.into(),
            scope: scope.clone(),
            payload: payload.clone(),
            pending: Pending::replacement(payload.clone(), scope.clone()),
            answer: None,
            callback: None,
            expected_answer: None,
            confirmed: false,
            ended: false,
            frozen: None,
            required_video,
            source_connection: source_connection.to_owned(),
        });
        Ok(())
    }
    pub fn verified(&self, frame: &Envelope) -> Result<()> {
        let mut guard = self.state.lock().map_err(|_| ClientError::Protocol)?;
        let Some(s) = guard.as_mut() else {
            return Ok(());
        };
        let room = frame.scope(Scope::Conversation) == s.scope.conversation_id.as_deref()
            && frame.scope(Scope::Session) == s.scope.session_id.as_deref();
        if room
            && (frame.kind() == "session.ended"
                || (frame.kind() == "connection.end"
                    && frame.scope(Scope::Connection) == s.scope.connection_id.as_deref()))
        {
            s.ended = true;
            return Ok(());
        }
        if s.ended || s.confirmed || !roots(&s.scope, frame) {
            return Ok(());
        }
        if frame.kind() == "connection.answer" {
            let _: Marker = serde_json::from_value(
                frame.original()["payload"]["replacement_preparation"].clone(),
            )
            .map_err(|_| ClientError::Protocol)?;
            s.pending
                .answer(frame, &s.request)
                .map_err(|_| ClientError::Protocol)?;
            s.answer = Some(frame.original()["payload"].clone());
        } else if frame.kind() == "connection.offer" {
            if s.callback.is_some() {
                return Err(ClientError::Protocol);
            }
            let answer = s
                .pending
                .confirmation(frame)
                .map_err(|_| ClientError::Protocol)?;
            // Both independently signed negotiation legs must carry the exact
            // same server challenge; absence is not a fallback profile.
            let actual = &frame.original()["payload"]["replacement_preparation"];
            if actual != &s.answer.as_ref().ok_or(ClientError::Protocol)?["replacement_preparation"]
            {
                return Err(ClientError::Protocol);
            }
            s.expected_answer =
                Some(serde_json::to_value(answer).map_err(|_| ClientError::Protocol)?);
            s.callback = Some(frame.original().clone());
        }
        Ok(())
    }
    pub fn answer(&self, frame: &Envelope) -> Result<payloads::ConnectionAnswer> {
        let guard = self.state.lock().map_err(|_| ClientError::Protocol)?;
        let s = guard
            .as_ref()
            .filter(|s| !s.ended && !s.confirmed)
            .ok_or(ClientError::Protocol)?;
        if !roots(&s.scope, frame) || s.callback.as_ref() != Some(frame.original()) {
            return Err(ClientError::Protocol);
        }
        serde_json::from_value(s.expected_answer.clone().ok_or(ClientError::Protocol)?)
            .map_err(|_| ClientError::Protocol)
    }
    pub fn check_answer(
        &self,
        frame: &Envelope,
        answer: &payloads::ConnectionAnswer,
        scope: &CommandScope,
    ) -> Result<bool> {
        let guard = self.state.lock().map_err(|_| ClientError::Protocol)?;
        let Some(s) = guard.as_ref() else {
            return Ok(false);
        };
        if scope.operation_id != s.scope.operation_id
            || scope.connection_id != s.scope.connection_id
        {
            return Ok(false);
        }
        if s.ended
            || s.confirmed
            || !roots(&s.scope, frame)
            || s.callback.as_ref() != Some(frame.original())
            || s.expected_answer.as_ref()
                != Some(&serde_json::to_value(answer).map_err(|_| ClientError::Protocol)?)
        {
            return Err(ClientError::Protocol);
        }
        Ok(true)
    }
    pub fn confirmed(&self) -> Result<()> {
        let mut guard = self.state.lock().map_err(|_| ClientError::Protocol)?;
        let s = guard.as_mut().ok_or(ClientError::Protocol)?;
        if s.ended || s.confirmed || s.callback.is_none() {
            return Err(ClientError::Protocol);
        }
        s.confirmed = true;
        Ok(())
    }
    pub fn outgoing(&self, frame: &Value) -> Result<()> {
        let mut guard = self.state.lock().map_err(|_| ClientError::Protocol)?;
        let Some(s) = guard.as_mut() else {
            return Ok(());
        };
        if frame["cid"].as_str() == s.scope.conversation_id.as_deref()
            && frame["sid"].as_str() == s.scope.session_id.as_deref()
            && (frame["type"] == "session.end"
                || (frame["connid"].as_str() == s.scope.connection_id.as_deref()
                    && matches!(
                        frame["type"].as_str(),
                        Some(
                            "connection.end"
                                | "connection.update"
                                | "stream.subscribe"
                                | "stream.unsubscribe"
                        )
                    )))
        {
            s.ended = true;
        }
        Ok(())
    }
    pub fn preparation(&self) -> Result<NativeReplacementReadiness> {
        let guard = self.state.lock().map_err(|_| ClientError::Protocol)?;
        let s = guard
            .as_ref()
            .filter(|s| s.confirmed && !s.ended)
            .ok_or(ClientError::Protocol)?;
        let callback = s.callback.as_ref().ok_or(ClientError::Protocol)?;
        let marker: Marker =
            serde_json::from_value(callback["payload"]["replacement_preparation"].clone())
                .map_err(|_| ClientError::Protocol)?;
        let c = &marker.readiness;
        if marker.profile != "connect-replacement-media-v1"
            || s.payload["replacement_preparation"]["operation_record_id"]
                != marker.operation_record_id
            || c.nonce.is_empty()
            || c.nonce.len() > 128
            || !c
                .nonce
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || c.source_generation == 0
            || c.target_generation > MAX_INTEGER
            || c.source_generation.checked_add(1) != Some(c.target_generation)
            || c.stream_revision != 2
        {
            return Err(ClientError::Protocol);
        }
        let selected = s.pending.selected().map_err(|_| ClientError::Protocol)?;
        if selected
            .video_feedback
            .values()
            .any(|v| v.connection_generation != c.target_generation)
        {
            return Err(ClientError::Protocol);
        }
        let receive = |medium: Medium| -> Result<Vec<NativeReadinessStream>> {
            selected
                .streams
                .iter()
                .filter(|s| s.medium == medium && s.direction != Direction::Sendonly)
                .map(|stream| {
                    let raw = callback["payload"]["substrate_setup"]["streams"]
                        .as_array()
                        .ok_or(ClientError::Protocol)?
                        .iter()
                        .find(|v| v["strm_id"].as_str() == Some(stream.id.as_str()))
                        .ok_or(ClientError::Protocol)?;
                    let native_egress:connect_protocol::replacement_activation::NativeEgressIdentity=serde_json::from_value(raw["native_egress"].clone()).map_err(|_|ClientError::Protocol)?;
                    native_egress.validate().map_err(|_|ClientError::Protocol)?;
                    if native_egress.connection_generation!=c.target_generation {return Err(ClientError::Protocol);}
                    Ok(NativeReadinessStream {
                        native_egress,
                        stream_id: stream.id.as_str().to_owned(),
                        stream_local_id: stream.stream_local_id.ok_or(ClientError::Protocol)?,
                        payload_type: raw["codec"]["params"]["payload_type"]
                            .as_u64()
                            .and_then(|v| u8::try_from(v).ok())
                            .filter(|v| *v < 128)
                            .ok_or(ClientError::Protocol)?,
                    })
                })
                .collect()
        };
        let mut audio = receive(Medium::Audio)?;
        let mut video = receive(Medium::Video)?
            .into_iter()
            .filter(|stream| s.required_video.contains(&stream.stream_id))
            .collect::<Vec<_>>();
        if audio.len() != 1 || video.len() != s.required_video.len() {
            return Err(ClientError::Protocol);
        }
        Ok(NativeReplacementReadiness {
            request: s.request.clone(),
            callback: callback.clone(),
            payload: s.payload.clone(),
            scope: s.scope.clone(),
            source_connection: s.source_connection.clone(),
            challenge: marker.readiness,
            audio: audio.remove(0),
            video: video.pop(),
        })
    }
    fn admit(&self, ready: &NativeReplacementReady) -> Result<()> {
        let current = self.preparation()?;
        if current.request != ready.preparation.request
            || current.callback != ready.preparation.callback
            || current.payload != ready.preparation.payload
        {
            return Err(ClientError::Protocol);
        }
        let mut guard = self.state.lock().map_err(|_| ClientError::Protocol)?;
        let s = guard
            .as_mut()
            .filter(|s| s.confirmed && !s.ended)
            .ok_or(ClientError::Protocol)?;
        if s.frozen.as_ref().is_some_and(|v| v != &ready.payload) {
            return Err(ClientError::Protocol);
        }
        s.frozen = Some(ready.payload.clone());
        Ok(())
    }
}
impl CanonicalClient {
    /// Return the exact independently validated parameter answer for the
    /// callback already verified by this original physical reader. Signing it
    /// confirms resources only; it does not report decoded media readiness.
    pub fn native_replacement_answer(
        &self,
        callback: &Envelope,
    ) -> Result<payloads::ConnectionAnswer> {
        self.check_current()?;
        self.native_video.readiness.answer(callback)
    }
    /// Obtain this exact physical peer's signed and confirmed preparation.
    pub fn native_replacement_readiness(&self) -> Result<NativeReplacementReadiness> {
        self.check_current()?;
        self.native_video.readiness.preparation()
    }
    /// Submit the original frozen decoded-readiness continuation. This is not
    /// completion: recover its original operation status before retiring the old
    /// owner or opening Data. Replays keep this same value and original deadline.
    pub async fn confirm_native_replacement_ready(
        &mut self,
        ready: &NativeReplacementReady,
    ) -> Result<payloads::OperationUpdated> {
        self.check_current()?;
        self.native_video.readiness.admit(ready)?;
        let batch = self
            .request_value(
                "connection.offer",
                ready.payload.clone(),
                ready.preparation.scope.clone(),
            )
            .await?;
        super::calls::operation_reply(
            &batch,
            ready
                .preparation
                .scope
                .operation_id
                .as_deref()
                .ok_or(ClientError::Protocol)?,
        )
    }
}

#[cfg(test)]
mod tests;
