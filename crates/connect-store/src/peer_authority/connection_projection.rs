//! Current inputs for native allocation under the originating command fence.
use super::*;
use connect_protocol::connection_control::{PrepareRequest, PreparedConnection};
use serde_json::Value;

/// Backend facts only. Keep the exact originating PeerAuthorityGuard through
/// Core preparation/retention. This value cannot authorize future media reads.
#[derive(Clone)]
pub struct PeerConnectionProjection {
    pub session: PeerSessionProjection,
    pub connection: PreparedConnection,
    pub runtime_connection_id: String,
    pub device_id: String,
    pub endpoint_address: String,
    pub endpoint_transport: connect_protocol::provisioning::EndpointTransport,
    pub endpoint_capabilities: Vec<String>,
    pub accepted_capabilities: Value,
    pub presence_role: String,
    pub offer: Value,
    pub private_consultation: Option<String>,
    /// Exact target-owned signed isolation profile; not decoded readiness or a
    /// handoff execution grant. It may coexist with no other preparation kind.
    pub handoff_preparation: bool,
    /// A provisional primary floor cannot admit private input into a public
    /// Core scene before the audience attempt has possibly been submitted.
    pub publication_suspended: bool,
}

impl Store {
    pub fn project_connection_with_peer(
        &self,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<PeerConnectionProjection> {
        let mut c = self.connection()?;
        let tx = c.transaction().map_err(|_| Error::storage())?;
        let (_, _, view) = self.prepared_connection_at(&tx, credential, proof)?;
        Ok(view)
    }
    pub(crate) fn prepared_connection_at(
        &self,
        c: &Connection,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<(String, PrepareRequest, PeerConnectionProjection)> {
        let PeerCommand::NativeConnectionOffer(intent) = &proof.request.command else {
            return Err(Error::forbidden());
        };
        self.require_peer_source(c, proof, credential)?;
        let actor = crate::identity::actor_from_connection(c, credential)?;
        let preparation = PrepareRequest::new(intent.clone(), "projection-only".into());
        let saved = crate::operations::receipts::receipt(
            c,
            &actor.identity_id,
            &preparation.preparation_id,
        )?
        .ok_or_else(Error::forbidden)?;
        let connect_protocol::Operation::ConnectionPrepare { request } = &saved.operation else {
            return Err(Error::forbidden());
        };
        if request.intent != *intent {
            return Err(Error::forbidden());
        }
        self.require_peer_control(c, proof, credential, &saved.operation)?;
        crate::operations::receipts::check_receipt_authority(c, &actor, &saved, None)?;
        let record =
            crate::transitions::current_control_record(c, &actor, &preparation.preparation_id)?;
        if record.state != connect_protocol::transitions::OperationState::Completed {
            return Err(Error::forbidden());
        }
        let projection = current_projection(
            c,
            &actor,
            intent,
            &request.endpoint_id,
            &proof.key.device_id,
            self.runtime_epoch(),
            true,
        )?;
        self.require_peer_source(c, proof, credential)?;
        Ok((record.record_id, request.clone(), projection))
    }
}

pub(crate) fn current_projection(
    c: &Connection,
    actor: &crate::Actor,
    intent: &connect_protocol::connection_control::OfferIntent,
    endpoint_id: &str,
    device_id: &str,
    runtime_epoch: &str,
    initial: bool,
) -> Result<PeerConnectionProjection> {
    let member = crate::audience::member(c, &actor.identity_id, &intent.conversation_id, true)?;
    let session = crate::session_control::view(c, &intent.conversation_id, &intent.session_id)?;
    if !matches!(session.state.as_str(), "accepted" | "ready" | "active") {
        return Err(Error::forbidden());
    }
    struct Row {
        runtime: String,
        offer: String,
        device: String,
        address: String,
        endpoint_caps: String,
        transport: String,
        accepted: String,
        role: String,
        state: String,
        generation: u64,
        revision: u64,
    }
    // The initial projection additionally requires an entirely unobserved slot.
    // Future packet reads use the same current facts, including later revisions,
    // while the sealed origin below fences the exact generation and ownership.
    let row: Option<Row> = c.prepare_cached("SELECT o.runtime_connection_id,o.offer_json,e.device_id,e.address,e.capabilities_json,cap.capabilities_json,sp.role,n.state,n.generation,n.revision,e.transport FROM connections n JOIN connection_offers o ON o.connection_id=n.id JOIN endpoints e ON e.id=n.endpoint_id AND e.identity_id=n.identity_id JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.tenant_id=e.tenant_id AND d.revoked=0 JOIN session_participants sp ON sp.session_id=n.session_id AND sp.participant_id=n.participant_id AND sp.membership_epoch=n.membership_epoch JOIN session_presence_capabilities cap ON cap.session_id=sp.session_id AND cap.participant_id=sp.participant_id AND cap.membership_epoch=sp.membership_epoch WHERE n.id=?1 AND n.session_id=?2 AND n.participant_id=?3 AND n.membership_epoch=?4 AND n.identity_id=?5 AND n.endpoint_id=?6 AND n.state IN ('preparing','ready','active') AND (n.observed_runtime_id IS NULL OR n.observed_runtime_id=?10) AND (?9=0 OR (n.state='preparing' AND n.generation=1 AND n.revision=0 AND n.observed_runtime_id IS NULL AND n.readiness_json IS NULL)) AND sp.state='present' AND e.transport IN ('quic','webtransport','webrtc') AND e.verification_state='verified' AND e.device_id=?7 AND o.operation_id=?8").map_err(|_|Error::storage())?.query_row(
        params![intent.connection_id,intent.session_id,member.participant_id,member.membership_epoch,actor.identity_id,endpoint_id,device_id,intent.operation_id,initial,runtime_epoch],
        |r| Ok(Row {runtime:r.get(0)?,offer:r.get(1)?,device:r.get(2)?,address:r.get(3)?,endpoint_caps:r.get(4)?,accepted:r.get(5)?,role:r.get(6)?,state:r.get(7)?,generation:r.get(8)?,revision:r.get(9)?,transport:r.get(10)?}),
    ).optional().map_err(|_| Error::storage())?;
    let row = row.ok_or_else(Error::forbidden)?;
    let offer: Value = serde_json::from_str(&row.offer).map_err(|_| Error::storage())?;
    if offer != intent.offer || row.transport != intent.endpoint_transport()?.as_str() {
        return Err(Error::forbidden());
    }
    let conversation_revision = c
        .query_row(
            "SELECT revision FROM conversations WHERE id=?1 AND closed=0",
            [&intent.conversation_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|_| Error::storage())?
        .ok_or_else(Error::forbidden)?;
    let handoff_preparation =
        crate::connection_control::handoff_preparation::current(c, actor, intent)?;
    let private =
        crate::connection_control::private_consultation::context(c, intent, Some(endpoint_id))?;
    let publication_suspended = c.prepare_cached("SELECT EXISTS(SELECT 1 FROM native_private_content_floors f JOIN native_consultation_attempts n ON n.operation_record_id=f.operation_record_id WHERE f.connection_id=?1 AND f.is_candidate=0 AND n.started=0)").map_err(|_|Error::storage())?.query_row([&intent.connection_id],|r|r.get(0)).map_err(|_|Error::storage())?;
    let mut endpoint_capabilities: Vec<String> =
        serde_json::from_str(&row.endpoint_caps).map_err(|_| Error::storage())?;
    if private.is_some()
        || handoff_preparation
        || crate::connection_control::private_consultation::private_partner(
            c,
            &intent.connection_id,
        )?
        .is_some()
    {
        endpoint_capabilities.retain(|v| v == "audio");
    }
    Ok(PeerConnectionProjection {
        private_consultation: private.map(|p| p.record),
        handoff_preparation,
        publication_suspended,
        connection: PreparedConnection {
            connection_id: intent.connection_id.clone(),
            conversation_id: intent.conversation_id.clone(),
            session_id: intent.session_id.clone(),
            participant_id: member.participant_id.clone(),
            membership_epoch: member.membership_epoch,
            endpoint_id: endpoint_id.into(),
            generation: row.generation,
            revision: row.revision,
            state: row.state,
        },
        session: PeerSessionProjection {
            session,
            actor: member,
            conversation_revision,
            present: true,
        },
        runtime_connection_id: row.runtime,
        device_id: row.device,
        endpoint_address: row.address,
        endpoint_transport: match row.transport.as_str() {
            "quic" => connect_protocol::provisioning::EndpointTransport::Quic,
            "webtransport" => connect_protocol::provisioning::EndpointTransport::Webtransport,
            "webrtc" => connect_protocol::provisioning::EndpointTransport::Webrtc,
            _ => return Err(Error::forbidden()),
        },
        endpoint_capabilities,
        accepted_capabilities: serde_json::from_str(&row.accepted).map_err(|_| Error::storage())?,
        presence_role: row.role,
        offer,
    })
}
