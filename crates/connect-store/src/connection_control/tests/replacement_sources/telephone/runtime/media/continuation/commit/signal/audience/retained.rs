//! Controlled complete selected SIP custody and recorded audience outcome; no wire IO.
use super::*;
use connect_protocol::transitions::{HoldTreatment, TransitionCommand, TransitionIntent};

#[test]
fn retained_established_sip_scene_records_its_own_revision_and_current_private_lifetime() {
    let (f, phone, sources, route) = established();
    drop(f.store.try_telephone_media_read(phone.clone()).unwrap());
    for key in [&f.keys[1], &f.keys[2]] {
        let mut actor = f.store.revalidate(key).unwrap();
        actor.scope.operations.push("stream.subscribe".into());
        f.store
            .connection()
            .unwrap()
            .execute(
                "UPDATE credentials SET operations=?2 WHERE id=?1",
                params![key, serde_json::to_string(&actor.scope.operations).unwrap()],
            )
            .unwrap();
    }
    {
        let c = f.store.connection().unwrap();
        c.execute("UPDATE identities SET kind='human' WHERE id=(SELECT identity_id FROM participants WHERE participant_id=?1)",[&f.pids[2]]).unwrap();
        c.execute(
            "UPDATE participants SET kind='human' WHERE participant_id=?1",
            [&f.pids[2]],
        )
        .unwrap();
    }
    let identity = f.store.revalidate(&f.keys[1]).unwrap().identity_id;
    let controller = f
        .store
        .issue(&identity, Scope::operator(), crate::now() + 600)
        .unwrap();
    let topology: u64 = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT topology_revision FROM sessions WHERE id='session'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let command = TransitionCommand {
        operation_id: "consult-with-retained-sip".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(sources[1].connection_id().into()),
        expected_revision: topology,
        intent: TransitionIntent::Consult {
            retained_connid: phone.snapshot().connection_id.clone(),
            private_participants: vec![f.pids[1].clone(), f.pids[2].clone()],
            hold_treatment: HoldTreatment::Silence,
        },
        payload_extensions: Default::default(),
    };
    f.store
        .admit_transition_command(&controller.id, command.clone())
        .unwrap();
    let origin = f
        .store
        .native_consultation_origin(&controller.id, command)
        .unwrap();
    // Provision the candidate's actual controlled signing identity before its
    // separate private admission; the telephone fixture has only two native keys.
    let _candidate_identity = f.native_request_for(2, 600, "retained-candidate-identity");
    let candidate = crate::connection_control::tests::consultation_audiences::candidate_named(
        &f,
        &origin,
        true,
        "retained-sip-consultant",
    );
    let remote = crate::ConsultationMediaOrigin::Sip(SipAudienceOrigin::Established(phone.clone()));
    let read = f
        .store
        .try_native_consultation_scene_read(
            &origin,
            &remote,
            &[sources[1].clone(), candidate.clone()],
            &[],
            &[],
        )
        .unwrap();
    assert!(read.is_retained_sip(&phone.snapshot().connection_id));
    assert!(read.sip(&phone.snapshot().connection_id).unwrap().current());
    assert!(read.media(&phone.snapshot().connection_id, None).is_none());
    assert!(read
        .media(
            sources[1].connection_id(),
            Some(&phone.snapshot().connection_id)
        )
        .is_none());
    let (p, r, c) = (
        sources[1].runtime_connection_id(),
        phone.snapshot().runtime_connection_id.as_str(),
        candidate.runtime_connection_id(),
    );
    let revision = |id: &str| {
        f.store.connection().unwrap().query_row("SELECT json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",[id],|row|row.get::<_,u64>(0)).unwrap()
    };
    let before = revision(&phone.snapshot().connection_id);
    let plan = json!({"operation_id":origin.record_id(),"audience_revision":0,"private_connections":[p,c],"receivers":{p:[c],c:[p],r:[]},"activate_muted":c,"expected_receiver_revisions":{p:revision(sources[1].connection_id()),c:revision(candidate.connection_id()),r:before}});
    let lease = f
        .store
        .prepare_native_consultation(read, plan.clone())
        .unwrap()
        .unwrap();
    let read = f.store.native_consultation_attempt_read(&lease).unwrap();
    let (attempt, read) = f.store.begin_native_consultation(lease, read).unwrap();
    drop(read);
    let revisions = plan["expected_receiver_revisions"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(id, n)| (id.clone(), json!(n.as_u64().unwrap() + 1)))
        .collect::<serde_json::Map<_, _>>();
    let receipt=attempt.applied(json!({"operation_id":origin.record_id(),"audience_revision":1,"receiver_revisions":revisions,"restored":false,"departed_connections":[],"participant_handoff":null,"participant_replacement":null})).unwrap();
    f.store
        .record_native_consultation_applied(&receipt)
        .unwrap();
    f.store
        .record_native_consultation_applied(&receipt)
        .unwrap();
    assert_eq!(revision(&phone.snapshot().connection_id), before + 1);
    f.store
        .record_native_consultation_decoded(&receipt)
        .unwrap();
    f.store
        .complete_native_consultation(&receipt)
        .unwrap()
        .expect("captured human private readiness must complete");
    let c = f.store.connection().unwrap();
    let current = crate::connection_control::private_consultation::active::current(
        &f.store,
        &c,
        origin.record_id(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(current.remote, phone.snapshot().connection_id);
    assert_eq!(current.primary, sources[1].connection_id());
    let selected: String = c
        .query_row(
            "SELECT selection_json FROM native_private_lifetimes WHERE operation_record_id=?1",
            [origin.record_id()],
            |r| r.get(0),
        )
        .unwrap();
    let selected: Value = serde_json::from_str(&selected).unwrap();
    assert_eq!(selected["remote"]["source_kind"], "established-sip");
    assert!(selected["remote"].get("key_id").is_none());
    drop(c);
    let topology: u64 = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT topology_revision FROM sessions WHERE id='session'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let returning = TransitionCommand {
        operation_id: "return-to-retained-sip".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(sources[1].connection_id().into()),
        expected_revision: topology,
        intent: TransitionIntent::ConsultReturn {
            consultation_record_id: origin.record_id().into(),
            retained_connid: phone.snapshot().connection_id.clone(),
        },
        payload_extensions: Default::default(),
    };
    f.store
        .admit_transition_command(&controller.id, returning.clone())
        .unwrap();
    let return_origin = f
        .store
        .native_consultation_return_origin(&controller.id, returning)
        .unwrap();
    let return_read = || {
        f.store
            .try_native_return_read(
                &return_origin,
                &[sources[1].clone(), candidate.clone()],
                &[SipAudienceOrigin::Established(phone.clone())],
            )
            .unwrap()
    };
    let read = return_read();
    assert!(read
        .sip_send(&phone.snapshot().connection_id, sources[1].connection_id())
        .is_some());
    assert!(read
        .sip_send(&phone.snapshot().connection_id, candidate.connection_id())
        .is_none());
    assert!(read
        .sip_receive(sources[1].connection_id(), &phone.snapshot().connection_id)
        .is_some());
    assert!(read
        .media(
            sources[1].connection_id(),
            Some(&phone.snapshot().connection_id)
        )
        .is_none());
    drop(read);
    permissions(&f, &route, &["sip_dial", "dtmf.send", "dtmf.receive"]);
    let read = return_read();
    assert!(read
        .sip_send(&phone.snapshot().connection_id, sources[1].connection_id())
        .is_none());
    assert!(read
        .sip_receive(sources[1].connection_id(), &phone.snapshot().connection_id)
        .is_some());
    drop(read);
    permissions(&f, &route, &["dtmf.send", "dtmf.receive"]);
    let c = f.store.connection().unwrap();
    assert!(
        crate::connection_control::private_consultation::active::current(
            &f.store,
            &c,
            origin.record_id()
        )
        .is_err()
    );
}
