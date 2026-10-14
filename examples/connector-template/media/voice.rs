//! Bounded PCM lifecycle fixture; no voice/video provider support is implied.
use crate::{CallEntry, LocalProvider};
use async_trait::async_trait;
use connect_connector_api::{conformance::Injection, lifecycle::*, Failure};
#[async_trait]
impl MediaExtension for LocalProvider {
    async fn open(&self, request: &VoiceRequest) -> Result<Observation, Failure> {
        if request.mutation_id.is_empty()
            || request.mutation_id.len() > 256
            || request.destination != "fixture-peer"
        {
            return Err(Failure::Rejected);
        }
        let mut state = self.state.lock().map_err(|_| Failure::Uncertain)?;
        if let Some(call) = state.calls.get(&request.mutation_id) {
            if call.request != *request {
                return Err(Failure::Rejected);
            }
            return if call.uncertain {
                Err(Failure::Uncertain)
            } else {
                Ok(call.receipt.clone())
            };
        }
        if state.calls.len() >= 128 {
            return Err(Failure::Rejected);
        }
        let uncertain = matches!(
            *self.injection.lock().map_err(|_| Failure::Uncertain)?,
            Injection::UncertainVoice
        );
        state.calls.insert(
            request.mutation_id.clone(),
            CallEntry {
                request: request.clone(),
                state: VoiceState::Active,
                uncertain,
                receipt: Self::observation(
                    &request.mutation_id,
                    State::Voice(VoiceState::Starting),
                    false,
                ),
            },
        );
        self.save(&state)?;
        if uncertain {
            Err(Failure::Uncertain)
        } else {
            Ok(Self::observation(
                &request.mutation_id,
                State::Voice(VoiceState::Starting),
                false,
            ))
        }
    }
    async fn reconcile(&self, id: &str) -> Result<Observation, Failure> {
        let mut state = self.state.lock().map_err(|_| Failure::Uncertain)?;
        let call = state.calls.get_mut(id).ok_or(Failure::Rejected)?;
        match *self.injection.lock().map_err(|_| Failure::Uncertain)? {
            Injection::VoiceFailure => call.state = VoiceState::Failed,
            Injection::PeerLoss => call.state = VoiceState::Ended,
            _ => {}
        }
        call.uncertain = false;
        let observed = call.state;
        self.save(&state)?;
        Ok(Self::observation(id, State::Voice(observed), true))
    }
    async fn close(&self, id: &str) -> Result<Observation, Failure> {
        let mut state = self.state.lock().map_err(|_| Failure::Uncertain)?;
        let call = state.calls.get_mut(id).ok_or(Failure::Rejected)?;
        // End is stable and pair-wide. Close cannot resurrect a terminal failure.
        if call.state != VoiceState::Failed {
            call.state = VoiceState::Ended;
        }
        call.uncertain = false;
        let observed = call.state;
        self.save(&state)?;
        Ok(Self::observation(id, State::Voice(observed), true))
    }
    async fn exchange(&self, id: &str, pcm: &[i16]) -> Result<Vec<i16>, Failure> {
        // One 20 ms mono 16 kHz PCM frame; encode little-endian bytes, decode at
        // the remote fixture endpoint, return the opposite-phase peer signal.
        if pcm.len() != 320 {
            return Err(Failure::Rejected);
        }
        let state = self.state.lock().map_err(|_| Failure::Uncertain)?;
        let call = state.calls.get(id).ok_or(Failure::Rejected)?;
        if call.state != VoiceState::Active || call.uncertain {
            return Err(Failure::Rejected);
        }
        let wire: Vec<u8> = pcm.iter().flat_map(|v| v.to_le_bytes()).collect();
        Ok(wire
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]).saturating_neg())
            .collect())
    }
}
