//! Controlled source/target states qualify Store authority, not native IO.
use super::*;
use std::sync::Arc;
struct Data {
    f: Fixture,
    request: PeerAuthorityRequest,
    source: Arc<crate::NativeMediaOrigin>,
    target: Arc<crate::NativeMediaOrigin>,
}
fn fixture(actor: usize, receive: bool) -> Data {
    let (mut f, mut request, _, source) = parent_permissions_origin(false, true, false, true, true);
    if receive {
        let identity = f.store.revalidate(&f.keys[actor]).unwrap().identity_id;
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope.operations.push("stream.subscribe".into());
        f.keys[actor] = f
            .store
            .issue(&identity, scope, crate::now() + 600)
            .unwrap()
            .id;
    }
    let mut target = f.native_request_for(actor, 600, "data-receiver");
    let PeerCommand::NativeConnectionOffer(offer) = &mut target.command else {
        panic!()
    };
    offer.offer = json!({"substrate":"quic","capabilities":{"data_protocols":["json"],"max_streams_per_connection":8,"identity_assurance_offered":"identified"},"streams_offered":[{"id":"application-in","kind":"data","direction":"recvonly","codec_preferences":["json"]}]});
    let proof = f.admit(&target);
    f.store
        .prepare_native_offer_with_peer(&f.keys[actor], &proof)
        .unwrap();
    let (lease, _) = f
        .store
        .begin_native_offer_with_peer(&f.keys[actor], &proof)
        .unwrap()
        .unwrap();
    let target = f
        .store
        .native_media_origin_with_peer(&f.keys[actor], &proof, &lease)
        .unwrap();
    {
        let c = f.store.connection().unwrap();
        c.execute(
            "UPDATE operation_effects SET state='confirmed' WHERE id=?1",
            [lease.effect_id()],
        )
        .unwrap();
        c.execute(
            "UPDATE operations SET state='completed',revision=revision+1 WHERE record_id=?1",
            [lease.record_id()],
        )
        .unwrap();
        c.execute("UPDATE connections SET state='ready',revision=1,observed_runtime_id=?2,readiness_json=?3 WHERE id=?1",params![target.connection_id(),f.store.runtime_epoch(),json!({"streams":["application-in"],"stream_revision":1,"participant_revision":1}).to_string()]).unwrap();
        c.execute("INSERT INTO native_connection_lifetimes(operation_record_id,connection_id,runtime_epoch,generation) VALUES(?1,?2,?3,?4)",params![lease.record_id(),lease.connection_id(),f.store.runtime_epoch(),lease.generation()]).unwrap();
    }
    drop(proof);
    let PeerCommand::NativeStream(wire) = &mut request.command else {
        panic!()
    };
    wire.command = Command::Data;
    wire.operation_id = "publish-application".into();
    wire.expected_revision = None;
    wire.by_participant = Some(f.pids[0].clone());
    wire.payload = json!({"from":f.pids[0],"msg_id":"application-message","label":"application","to":[f.pids[1]],"content_type":"application/json","body":"{\"hello\":true}"});
    Data {
        f,
        request,
        source,
        target,
    }
}
impl Data {
    fn wire(&self) -> &Intent {
        let PeerCommand::NativeStream(w) = &self.request.command else {
            panic!()
        };
        w
    }
    fn begin(&self) -> (crate::NativeStreamLease, crate::PeerAuthorityGuard) {
        let proof = admit(&self.f, &self.request).unwrap();
        self.f
            .store
            .accept_native_stream_with_peer(&self.f.keys[0], &proof)
            .unwrap();
        (
            self.f
                .store
                .begin_native_stream_with_peer(&self.f.keys[0], &proof)
                .unwrap()
                .unwrap()
                .0,
            proof,
        )
    }
    fn fresh(&self) -> crate::Result<crate::NativeDataRead> {
        self.f
            .store
            .try_native_data_read(&self.target, &self.source, self.wire())
    }
}
#[test]
fn native_data_read_requires_one_begun_original_message_and_has_no_writer_effect() {
    let d = fixture(1, true);
    let before = d.f.count("native_data_writes");
    assert!(d.fresh().is_err());
    let (lease, proof) = d.begin();
    let read =
        d.f.store
            .native_data_read_with_peer(&d.source, &proof, &lease)
            .unwrap();
    assert!(read.current());
    assert_eq!(read.media().target().connection.participant_id, d.f.pids[0]);
    drop(read);
    drop(proof);
    let read = d.fresh().unwrap();
    assert!(read.current());
    assert_eq!(read.media().target().connection.participant_id, d.f.pids[1]);
    assert_eq!(d.f.count("native_data_writes"), before);
}
#[test]
fn native_data_read_connection_creation_does_not_grant_subscription_permission() {
    let d = fixture(1, false);
    let (_, proof) = d.begin();
    drop(proof);
    assert_eq!(d.fresh().unwrap_err().code, "forbidden");
}
#[test]
fn native_data_read_exact_intent_and_original_audience_cannot_be_substituted() {
    let d = fixture(1, true);
    let (_, proof) = d.begin();
    drop(proof);
    for (field, value) in [
        ("body", json!("changed")),
        ("msg_id", json!("changed")),
        ("label", json!("changed")),
        ("to", json!([d.f.pids[2]])),
        ("extra", json!(true)),
    ] {
        let mut wire = d.wire().clone();
        wire.payload[field] = value;
        assert!(d
            .f
            .store
            .try_native_data_read(&d.target, &d.source, &wire)
            .is_err());
    }
    let d = fixture(2, true);
    let (_, proof) = d.begin();
    drop(proof);
    assert_eq!(d.fresh().unwrap_err().code, "not-recipient");
}
#[test]
fn native_data_read_current_target_permission_presence_and_data_floor_are_independent() {
    for mode in 0..4 {
        let d = fixture(1, true);
        let (_, proof) = d.begin();
        drop(proof);
        assert!(d.fresh().is_ok());
        let c = d.f.store.connection().unwrap();
        match mode {
            0 => {
                c.execute(
                    "UPDATE credentials SET revoked=1 WHERE id=?1",
                    [&d.f.keys[1]],
                )
                .unwrap();
            }
            1 => {
                c.execute("UPDATE session_participants SET state='left' WHERE session_id='session' AND participant_id=?1",[&d.f.pids[1]]).unwrap();
            }
            2 => {
                c.execute("UPDATE session_presence_capabilities SET capabilities_json='{}' WHERE session_id='session' AND participant_id=?1",[&d.f.pids[1]]).unwrap();
            }
            _ => {
                c.execute(
                    "UPDATE credentials SET operations=?2 WHERE id=?1",
                    params![
                        d.f.keys[1],
                        serde_json::to_string(
                            &Scope::collaboration(vec!["*".into()])
                                .operations
                                .into_iter()
                                .filter(|s| s != "history")
                                .collect::<Vec<_>>()
                        )
                        .unwrap()
                    ],
                )
                .unwrap();
            }
        }
        drop(c);
        assert!(d.fresh().is_err(), "case {mode}");
    }
}
#[test]
fn native_data_read_cancellation_and_source_revocation_stop_queued_delivery_without_new_identity() {
    for cancelled in [false, true] {
        let d = fixture(1, true);
        let (_, proof) = d.begin();
        drop(proof);
        if cancelled {
            d.f.store
                .execute_operation_action(
                    &d.f.admin,
                    connect_protocol::transitions::OperationAction::Cancel {
                        operation_id: "cancel-data-read".into(),
                        target_operation_id: d.wire().operation_id.clone(),
                        reason: None,
                    },
                )
                .unwrap();
        } else {
            d.f.store
                .connection()
                .unwrap()
                .execute(
                    "UPDATE credentials SET revoked=1 WHERE id=?1",
                    [&d.f.keys[0]],
                )
                .unwrap();
        }
        assert!(d.fresh().is_err());
        assert_eq!(d.f.count("native_data_messages"), 1);
        assert_eq!(d.f.count("native_data_writes"), 0);
    }
}
#[test]
fn native_data_read_held_current_check_uses_no_store_lock_and_fresh_contention_refuses() {
    let d = fixture(1, true);
    let (lease, proof) = d.begin();
    let read =
        d.f.store
            .native_data_read_with_peer(&d.source, &proof, &lease)
            .unwrap();
    let held = d.f.store.connection().unwrap();
    assert!(read.current());
    assert_eq!(d.fresh().unwrap_err().code, "authority-busy");
    drop(held);
    drop(read);
    drop(proof);
    assert!(d.fresh().is_ok());
}
