//! Controlled epoch application and DTMF custody, not physical rotation proof.
use super::*;
use crate::{PeerAuthorityRequest, PeerCommand};
use std::{collections::BTreeMap, time::Duration};
fn rotate(s: &Signal) -> (Arc<crate::NativeMediaOrigin>, String) {
    use base64::Engine;
    let old = s.f.store.revalidate(&s.f.keys[1]).unwrap();
    let next =
        s.f.store
            .issue(&old.identity_id, old.scope.clone(), crate::now() + 900)
            .unwrap();
    let key =
        s.f.store
            .resolve_peer_signing_key(&old.credential_id, "tnt_default", "native-key-1", "EdDSA")
            .unwrap();
    let revision =
        s.f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT revision FROM peer_signing_keys WHERE key_id='native-key-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
    s.f.store
        .execute_provisioning(
            &s.f.admin,
            ProvisioningCommand::SigningKey {
                operation_id: "bind-dtmf-refresh-successor".into(),
                identity_id: old.identity_id,
                credential_id: next.id.clone(),
                device_id: "device-1".into(),
                key_id: "native-key-1".into(),
                algorithm: SigningAlgorithm::EdDSA,
                public_key: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key.public_key),
                not_before: key.not_before,
                expires_at: key.expires_at,
                expected_revision: Some(revision),
            },
        )
        .unwrap();
    let mut request = native_selection(&s.f, &s.native, "refresh-signal-binding");
    request.command = PeerCommand::Binding;
    request.credentials = vec![s.f.keys[1].clone(), next.id.clone()];
    let (proof, plan) =
        s.f.store
            .admit_native_credential_refresh(
                &request,
                &[s.native.clone()],
                &crate::AuthorityWait::new(Duration::from_secs(2)),
            )
            .unwrap();
    let renewed = plan.targets()[0].next().clone();
    let attempt =
        s.f.store
            .prepare_native_credential_refresh(&plan, &proof, "dtmf-epoch-rotation")
            .unwrap();
    s.f.store
        .begin_native_credential_refresh(&attempt, &plan, &proof)
        .unwrap();
    let applied = attempt
        .applied(BTreeMap::from([(
            s.native.offer_record_id().into(),
            renewed.credential_epoch() + 1,
        )]))
        .unwrap();
    s.f.store
        .record_native_credential_refresh_applied(&applied)
        .unwrap();
    drop(proof);
    (renewed, next.id)
}
fn allow(s: &Signal) {
    permissions(
        &s.f,
        &s.f.keys[1],
        &[
            "connect:control",
            "conversation.get",
            "subscribe",
            "dtmf.send",
            "dtmf.receive",
        ],
    );
}

#[test]
fn dtmf_refresh_received_queued_recipient_keeps_original_epoch_and_fresh_input_uses_current_view() {
    let s = fixture();
    allow(&s);
    let original = observation();
    let queued = freeze(&s, original.clone());
    let frozen: String =
        s.f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT target_json FROM received_dtmf_recipients WHERE operation_record_id=?1",
                [queued.record_id()],
                |r| r.get(0),
            )
            .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&frozen).unwrap()["credential_epoch"],
        0
    );
    let (renewed, next) = rotate(&s);
    assert!(s
        .f
        .store
        .try_received_dtmf_read(&queued.targets()[0])
        .is_err());
    let c = s.f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT credential_id FROM native_admission_sources WHERE operation_record_id=?1",
            [s.native.offer_record_id()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        next
    );
    assert_eq!(
        c.query_row(
            "SELECT target_json FROM received_dtmf_recipients WHERE operation_record_id=?1",
            [queued.record_id()],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        frozen
    );
    drop(c);
    s.f.store
        .try_record_received_dtmf_outcome(&queued.targets()[0], D::NotStarted)
        .unwrap();
    assert_eq!(
        s.f.store.finish_received_dtmf(&queued).unwrap().state,
        OperationState::Failed
    );
    let admin = s.f.store.revalidate(&s.f.admin).unwrap();
    s.f.store.revoke(&admin, &s.f.keys[1]).unwrap();
    let mut fresh = original;
    fresh.observation_id = "actual-input-after-epoch-rotation".into();
    let ReceivedDtmfPreparation::New(batch) =
        s.f.store
            .prepare_received_dtmf(
                SipSignalOrigin::Established(s.phone.clone()),
                fresh,
                vec![NativeDtmfRecipient {
                    origin: renewed,
                    registration_id: "actual-new-epoch-registration".into(),
                }],
            )
            .unwrap()
    else {
        panic!()
    };
    assert_eq!(batch.targets().len(), 1);
    let read =
        s.f.store
            .try_received_dtmf_read(&batch.targets()[0])
            .unwrap();
    s.f.store
        .try_start_received_dtmf_write(&read, "original-rotated-recipient-frame")
        .unwrap();
    drop(read);
    s.f.store
        .try_record_received_dtmf_outcome(&batch.targets()[0], D::LocalSubmitted)
        .unwrap();
    assert_eq!(
        s.f.store.finish_received_dtmf(&batch).unwrap().state,
        OperationState::Completed
    );
    let c = s.f.store.connection().unwrap();
    let current: String = c
        .query_row(
            "SELECT target_json FROM received_dtmf_recipients WHERE operation_record_id=?1",
            [batch.record_id()],
            |r| r.get(0),
        )
        .unwrap();
    let current: serde_json::Value = serde_json::from_str(&current).unwrap();
    assert_eq!(current["credential_epoch"], 1);
    assert_eq!(current["credential_id"], next);
}

fn native_batch(
    s: &Signal,
    origin: Arc<crate::NativeMediaOrigin>,
    credential: &str,
    targets: Vec<NativeDtmfRecipient>,
    gateways: Vec<crate::NativeDtmfGatewayRecipient>,
    operation: &str,
    key: &str,
    device: &str,
) -> crate::NativeDtmfBatch {
    let request = PeerAuthorityRequest {
        credentials: vec![credential.into()],
        device: crate::PeerDeviceClaim {
            id: device.into(),
            kind: "desktop".into(),
            platform: "isolated-store-fixture".into(),
        },
        key_id: key.into(),
        algorithm: "EdDSA".into(),
        command: PeerCommand::NativeStream(connect_protocol::native_streams::Intent {
            operation_id: operation.into(),
            conversation_id: s.f.cid.clone(),
            session_id: "session".into(),
            connection_id: origin.connection_id().into(),
            expected_revision: None,
            by_participant: None,
            command: connect_protocol::native_streams::Command::Dtmf,
            payload: json!({"digits":"5","duration_ms":100,"method":"info"}),
        }),
    };
    let proof = s.f.admit(&request);
    s.f.store
        .accept_native_stream_with_peer(credential, &proof)
        .unwrap();
    let lease =
        s.f.store
            .begin_native_stream_with_peer(credential, &proof)
            .unwrap()
            .unwrap()
            .0;
    let observation = connect_protocol::native_dtmf::SourceObservation {
        envelope_id: format!("observed-{operation}"),
        received_at: chrono::Utc::now(),
        stream_revision: lease.expected_revision(),
        credential_revision: origin.credential_epoch() + 1,
        signing_key_id: key.into(),
        signing_algorithm: "EdDSA".into(),
    };
    s.f.store
        .prepare_native_mixed_dtmf_with_peer(&lease, origin, targets, gateways, &proof, observation)
        .unwrap()
}
#[test]
fn dtmf_refresh_native_source_and_native_recipient_use_applied_epoch_without_changing_lineage() {
    let s = fixture();
    allow(&s);
    permissions(&s.f, &s.route, &["sip_dial", "dtmf.send", "dtmf.receive"]);
    let (renewed, next) = rotate(&s);
    let admin = s.f.store.revalidate(&s.f.admin).unwrap();
    s.f.store.revoke(&admin, &s.f.keys[1]).unwrap();
    let outgoing = native_batch(
        &s,
        renewed.clone(),
        &next,
        vec![],
        vec![crate::NativeDtmfGatewayRecipient {
            origin: SipSignalOrigin::Established(s.phone.clone()),
            registration_id: "original-moved-sip-recipient".into(),
        }],
        "native-after-refresh",
        "native-key-1",
        "device-1",
    );
    assert_eq!(outgoing.observation().credential_revision, 2);
    let read =
        s.f.store
            .try_native_dtmf_gateway_read(&outgoing.gateway_targets()[0])
            .unwrap();
    s.f.store
        .try_start_native_dtmf_gateway_write(&read, "original-epoch-two-sip-schedule")
        .unwrap();
    drop(read);
    s.f.store
        .try_record_native_dtmf_gateway_outcome(&outgoing.gateway_targets()[0], D::LocalSubmitted)
        .unwrap();
    assert_eq!(
        s.f.store.finish_native_dtmf(&outgoing).unwrap().state,
        OperationState::Completed
    );
    let sender = crate::connection_control::tests::consultation_reads::ready(&s.f, 2);
    s.f.store.connection().unwrap().execute("UPDATE credentials SET operations=json_insert(operations,'$[#]','dtmf.send') WHERE id=?1",[&s.f.keys[2]]).unwrap();
    let incoming = native_batch(
        &s,
        sender,
        &s.f.keys[2],
        vec![NativeDtmfRecipient {
            origin: renewed,
            registration_id: "native-current-epoch-recipient".into(),
        }],
        vec![],
        "native-to-rotated-recipient",
        "native-key-2",
        "device-2",
    );
    let read =
        s.f.store
            .try_native_dtmf_read(&incoming.targets()[0])
            .unwrap();
    s.f.store
        .try_start_native_dtmf_write(&read, "original-current-epoch-native-frame")
        .unwrap();
    drop(read);
    s.f.store
        .try_record_native_dtmf_outcome(&incoming.targets()[0], D::LocalSubmitted)
        .unwrap();
    assert_eq!(
        s.f.store.finish_native_dtmf(&incoming).unwrap().state,
        OperationState::Completed
    );
    let c = s.f.store.connection().unwrap();
    let value:String=c.query_row("SELECT target_json FROM native_dtmf_recipients WHERE registration_id='native-current-epoch-recipient'",[],|r|r.get(0)).unwrap();
    let value: serde_json::Value = serde_json::from_str(&value).unwrap();
    assert_eq!(value["credential_epoch"], 1);
    assert_eq!(value["credential_id"], next);
}
