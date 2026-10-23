//! Target-owned signed isolation; a prepared peer is not decoded handoff readiness.
use super::private_audio::{AudioPreparation, NativeAudioPeer};
use super::*;

/// Owns the actual signed QUIC peer and its own immutable offer/policy reference.
/// The server confirmed muted, closed preparation. This neither accepts a caller
/// handoff nor authorizes public media, role promotion or private context export.
pub struct PreparedHandoffNativeAudio {
    peer: NativeAudioPeer,
    policy_record: String,
}
impl PreparedHandoffNativeAudio {
    pub fn step_up_required(&self) -> Option<&str> {
        self.peer.client.step_up_required()
    }

    /// Complete this peer's pending signed challenge with explicit consent or
    /// task authority. The server resumes its original logical mutation.
    pub async fn complete_step_up(&mut self, credential: String) -> Result<StepUpCompletion> {
        self.peer.client.complete_step_up(credential).await
    }
    pub async fn complete_step_up_with_cancel(
        &mut self,
        credential: String,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<StepUpCompletion> {
        self.peer
            .client
            .complete_step_up_with_cancel(credential, cancellation)
            .await
    }

    /// Refresh this physical peer with an explicit successor credential. The
    /// original signaling reader and its queued observations remain owned.
    pub async fn refresh_credential(
        &mut self,
        credential: String,
    ) -> Result<CanonicalAuthentication> {
        self.peer.client.refresh_credential(credential).await
    }
    pub async fn refresh_credential_with_cancel(
        &mut self,
        credential: String,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<CanonicalAuthentication> {
        self.peer
            .client
            .refresh_credential_with_cancel(credential, cancellation)
            .await
    }

    /// Submit on this exact physical peer. Isolation still governs recipients;
    /// preparation and this helper never supply independent signal permission.
    pub async fn send_dtmf(
        &mut self,
        operation_id: &str,
        expected_revision: Option<u64>,
        digits: &str,
        duration_ms: u32,
        method: &str,
    ) -> Result<payloads::OperationUpdated> {
        self.peer
            .send_dtmf(operation_id, expected_revision, digits, duration_ms, method)
            .await
    }
    pub fn policy_record_id(&self) -> &str {
        &self.policy_record
    }
    pub fn connection_id(&self) -> &str {
        self.peer.connection_id()
    }
    pub fn send_pcmu(&mut self, bytes: &[u8]) -> Result<()> {
        self.peer.send_pcmu(bytes)
    }
    pub async fn receive_pcmu(&mut self) -> Result<Vec<u8>> {
        self.peer.receive_pcmu().await
    }
    /// Receive verified retained signaling without taking over its wire reader.
    pub async fn receive_signaling(&mut self) -> Result<Envelope> {
        self.peer.client.receive().await
    }
    pub fn close(&self) {
        self.peer.close();
    }
    pub async fn shutdown(self) -> Result<()> {
        self.peer.shutdown().await
    }
}
impl CanonicalClient {
    /// Consumes this physical peer. Its own current named session.handoff grant
    /// and signed audio offer authorize isolation before a caller submits a
    /// handoff. The returned policy reference is a fact, never a transferable
    /// execution grant or a declaration that actual duplex readiness exists.
    pub async fn prepare_handoff_native_audio(
        self,
        scope: CommandScope,
        offer: Value,
    ) -> Result<PreparedHandoffNativeAudio> {
        let (peer, policy_record) = self
            .prepare_owned_native_audio(scope, AudioPreparation::Handoff, offer)
            .await?;
        Ok(PreparedHandoffNativeAudio {
            peer,
            policy_record,
        })
    }
}
