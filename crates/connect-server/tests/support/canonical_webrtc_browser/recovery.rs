//! Explicit local recovery faults. Scoped browser credentials are separately
//! issued/bound; operator authority and TLS private files stay in the Node host.
use super::*;

pub(super) fn fault_spec(f: &mut Fixture, ca: &Path) -> Value {
    let actor = f.server.store.authenticate(&f.client.credential).unwrap();
    let admin = f.server.store.authenticate(&f.admin).unwrap();
    let key = f
        .server
        .store
        .resolve_peer_signing_key(
            &actor.credential_id,
            "tnt_default",
            &f.client.signing_key_id,
            &f.client.signing_algorithm,
        )
        .unwrap();
    let mut credentials = Vec::new();
    for (label, seconds) in [("expiring", 45), ("revocable", 3600)] {
        let expires = chrono::Utc::now().timestamp() + seconds;
        let issued = f
            .server
            .store
            .issue(&actor.identity_id, actor.scope.clone(), expires)
            .unwrap();
        let result = f
            .server
            .store
            .execute_provisioning(
                &admin.credential_id,
                ProvisioningCommand::SigningKey {
                    operation_id: format!("recovery-bind-{label}"),
                    identity_id: actor.identity_id.clone(),
                    credential_id: issued.id.clone(),
                    device_id: f.client.device_id.clone(),
                    key_id: f.client.signing_key_id.clone(),
                    algorithm: key.algorithm,
                    public_key: URL_SAFE_NO_PAD.encode(&key.public_key),
                    not_before: key.not_before,
                    expires_at: key.expires_at,
                    expected_revision: Some(f.key_revision),
                },
            )
            .unwrap();
        let ProvisioningOutcome::SigningKey { revision, .. } = result else {
            panic!("bound fixture key")
        };
        f.key_revision = revision;
        credentials
            .push(json!({"kind":label,"id":issued.id,"token":issued.token,"expiresAt":expires}));
    }
    json!({
        "certificateDer": f.config.certificate, "privateKeyDer": f.config.private_key,
        "rootDer": ca, "quicPort": f.server.address.port(),
        "operatorEndpoint": format!("http://{}",f.server.http_address.unwrap()),
        "operatorCredential": f.admin, "credentials": credentials,
    })
}

pub(super) fn audit(f: &Fixture, other: &CanonicalClientConfig, report: &Value) -> Value {
    let fault = &report["transport_faults"];
    assert_eq!(fault["passed"], true);
    assert_eq!(fault["proxy"]["original_commands"], 1);
    assert_eq!(fault["new_message_commands"], 1);
    assert_eq!(fault["proxy"]["dropped_acks"], 1);
    assert!(fault["proxy"]["cut_websockets"].as_u64().unwrap() >= 1);
    assert!(fault["proxy"]["dropped_udp"].as_u64().unwrap() > 0);
    assert!(fault["proxy"]["forwarded_udp_client"].as_u64().unwrap() > 0);
    assert!(fault["proxy"]["forwarded_udp_server"].as_u64().unwrap() > 0);
    let c = rusqlite::Connection::open(f.server.store.path()).unwrap();
    let message: u64 = c
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE id='browser-dropped-message'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let operations: u64 = c.query_row("SELECT COUNT(*) FROM operations WHERE operation_id='browser-dropped-message-op' AND state='completed'", [], |r|r.get(0)).unwrap();
    let journal: u64 = c
        .query_row(
            "SELECT COUNT(*) FROM journal WHERE message_id='browser-dropped-message'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!((message, operations, journal), (1, 1, 2));
    for recipient in [&f.client.identity_id, &other.identity_id] {
        let count: u64 = c.query_row("SELECT COUNT(*) FROM journal WHERE message_id='browser-dropped-message' AND recipient=?1 AND kind='message'", [recipient], |r|r.get(0)).unwrap();
        assert_eq!(count, 1, "one fact per original authorized recipient");
    }
    let record: String = c.query_row("SELECT record_id FROM operations WHERE operation_id='browser-dropped-message-op' AND command_type='send'", [], |r|r.get(0)).unwrap();
    assert_eq!(fault["dropped_ack"]["record_id"], record);
    assert_eq!(fault["expiry"]["positive_before_expiry"], true);
    assert_eq!(fault["expiry"]["reconnect_refused"], true);
    assert_eq!(fault["revocation"]["positive_before_revocation"], true);
    assert_eq!(fault["revocation"]["reconnect_refused"], true);
    assert_eq!(fault["owned_proxy_joined"], true);
    json!({"passed":true,"message_rows":message,"operation_rows":operations,"journal_rows":journal,"scope":"One actual Message acceptance with withheld signed reply; original status-only recovery, TLS WSS cut, QUIC packet blackhole, real credential expiry/revocation. Control only; live transition media remains separate."})
}
