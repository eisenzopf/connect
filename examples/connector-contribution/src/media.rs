//! Independently authored bounded voice/audio lifecycle, mechanically split for the shipped example.
use super::*;

#[async_trait]
impl MediaExtension for Prism {
    async fn open(&self, request: &VoiceRequest) -> Result<Observation> {
        self.contract()
            .validate_address(&request.destination, None)
            .map_err(|_| Failure::Rejected)?;
        if !Self::id_valid(&request.mutation_id) {
            return Err(Failure::Rejected);
        }
        let _gate = self.gate.lock().map_err(|_| Failure::Uncertain)?;
        let mut ledger = self.read()?;
        if ledger.messages.contains_key(&request.mutation_id) {
            return Err(Failure::Rejected);
        }
        if let Some(prior) = ledger.sessions.get(&request.mutation_id) {
            if prior.request != *request {
                return Err(Failure::Rejected);
            }
            return if prior.uncertain {
                Err(Failure::Uncertain)
            } else {
                Ok(Self::voice_observation(
                    &request.mutation_id,
                    prior.state,
                    Verification::AuthenticatedResponse,
                ))
            };
        }
        let uncertain = matches!(self.take_fault(), Injection::UncertainVoice);
        ledger.sessions.insert(
            request.mutation_id.clone(),
            AudioSession {
                request: request.clone(),
                state: VoiceState::Starting,
                uncertain,
                left_open: true,
                right_open: true,
                effects: 1,
            },
        );
        self.write(&ledger)?;
        if uncertain {
            Err(Failure::Uncertain)
        } else {
            Ok(Self::voice_observation(
                &request.mutation_id,
                VoiceState::Starting,
                Verification::AuthenticatedResponse,
            ))
        }
    }
    async fn reconcile(&self, id: &str) -> Result<Observation> {
        let _gate = self.gate.lock().map_err(|_| Failure::Uncertain)?;
        let mut ledger = self.read()?;
        let session = ledger.sessions.get_mut(id).ok_or(Failure::Rejected)?;
        let terminal = matches!(session.state, VoiceState::Ended | VoiceState::Failed);
        if !terminal {
            session.state = match self.take_fault() {
                Injection::VoiceFailure => VoiceState::Failed,
                Injection::PeerLoss => VoiceState::Ended,
                _ => VoiceState::Active,
            };
        }
        session.uncertain = false;
        if matches!(session.state, VoiceState::Ended | VoiceState::Failed) {
            session.left_open = false;
            session.right_open = false;
        }
        let observation = Self::voice_observation(id, session.state, Verification::Reconciliation);
        self.write(&ledger)?;
        Ok(observation)
    }
    async fn close(&self, id: &str) -> Result<Observation> {
        let _gate = self.gate.lock().map_err(|_| Failure::Uncertain)?;
        let mut ledger = self.read()?;
        let session = ledger.sessions.get_mut(id).ok_or(Failure::Rejected)?;
        // Teardown by retained logical ID applies to both peer legs, even if open timed out.
        session.left_open = false;
        session.right_open = false;
        session.uncertain = false;
        if session.state != VoiceState::Failed {
            session.state = VoiceState::Ended;
        }
        let observation = Self::voice_observation(id, session.state, Verification::Reconciliation);
        self.write(&ledger)?;
        Ok(observation)
    }
    async fn exchange(&self, id: &str, pcm: &[i16]) -> Result<Vec<i16>> {
        if pcm.len() != FRAME_SAMPLES {
            return Err(Failure::Rejected);
        }
        let _gate = self.gate.lock().map_err(|_| Failure::Uncertain)?;
        let ledger = self.read()?;
        let session = ledger.sessions.get(id).ok_or(Failure::Rejected)?;
        if session.uncertain
            || session.state != VoiceState::Active
            || !session.left_open
            || !session.right_open
        {
            return Err(Failure::Rejected);
        }
        // Actual peer boundary: sender encodes little endian PCM; peer decodes,
        // phase-inverts with saturation, encodes; receiver decodes the wire reply.
        let outgoing: Vec<u8> = pcm.iter().flat_map(|s| s.to_le_bytes()).collect();
        let wire_reply: Vec<u8> = outgoing
            .chunks_exact(2)
            .flat_map(|b| {
                i16::from_le_bytes([b[0], b[1]])
                    .saturating_neg()
                    .to_le_bytes()
            })
            .collect();
        Ok(wire_reply
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect())
    }
}
