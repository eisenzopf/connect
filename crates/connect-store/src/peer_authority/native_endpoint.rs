//! Resolve this authenticated Device's already verified native endpoint.
use super::*;
use connect_protocol::connection_control::PrepareRequest;
impl Store {
    /// No address or endpoint ID comes from the offer payload. Selection does
    /// not prove media readiness and never provisions or upgrades under a reader.
    pub fn select_native_endpoint_with_peer(
        &self,
        credential: &str,
        proof: &PeerAuthorityGuard,
        transport: connect_protocol::provisioning::EndpointTransport,
    ) -> Result<String> {
        if !matches!(
            transport,
            connect_protocol::provisioning::EndpointTransport::Quic
                | connect_protocol::provisioning::EndpointTransport::Webtransport
        ) {
            return Err(Error::forbidden());
        }
        let PeerCommand::NativeConnectionOffer(intent) = &proof.request.command else {
            return Err(Error::forbidden());
        };
        let c = self.connection()?;
        self.require_peer_source(&c, proof, credential)?;
        let actor = crate::identity::actor_from_connection(&c, credential)?;
        let preparation = PrepareRequest::new(intent.clone(), "selection-only".into());
        if let Some(saved) = crate::operations::receipts::receipt(
            &c,
            &actor.identity_id,
            &preparation.preparation_id,
        )? {
            crate::operations::receipts::check_receipt_authority(&c, &actor, &saved, None)?;
            let connect_protocol::Operation::ConnectionPrepare { request } = &saved.operation
            else {
                return Err(Error::forbidden());
            };
            if request.intent != *intent {
                return Err(Error::new("conflict", "Connection offer intent conflicts"));
            }
            self.require_peer_control(&c, proof, credential, &saved.operation)?;
            let matches: bool = c
                .query_row(
                    "SELECT transport=?2 FROM endpoints WHERE id=?1",
                    params![request.endpoint_id, transport.as_str()],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            if !matches {
                return Err(Error::forbidden());
            }
            return Ok(request.endpoint_id.clone());
        }
        let resources = crate::connection_control::intent_resources(&c, &actor, intent)?;
        if resources.iter().any(|id| !actor.permits_recipient(id)) {
            return Err(Error::forbidden());
        }
        let mut statement=c.prepare("SELECT e.id FROM endpoints e JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.tenant_id=e.tenant_id AND d.revoked=0 WHERE e.identity_id=?1 AND e.device_id=?2 AND e.transport=?3 AND e.verification_state='verified' ORDER BY e.id LIMIT 2").map_err(|_|Error::storage())?;
        let ids = statement
            .query_map(
                params![actor.identity_id, proof.key.device_id, transport.as_str()],
                |r| r.get::<_, String>(0),
            )
            .map_err(|_| Error::storage())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| Error::storage())?;
        self.require_peer_source(&c, proof, credential)?;
        match ids.as_slice() {
            [one] => Ok(one.clone()),
            [] => Err(Error::new(
                "not-ready",
                "This Device has no verified native endpoint",
            )),
            _ => Err(Error::new(
                "conflict",
                "This Device has ambiguous native endpoints",
            )),
        }
    }
}
