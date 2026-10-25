//! Store custody only; actual signed quarantine is qualified by native tests.
use super::*;
use crate::{PeerAuthorityRequest, PeerCommand};
fn grant(f: &mut Fixture, index: usize) {
    let identity = f.store.revalidate(&f.keys[index]).unwrap().identity_id;
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope
        .operations
        .extend(["session.handoff".into(), "stream.subscribe".into()]);
    f.keys[index] = f
        .store
        .issue(&identity, scope, crate::now() + 600)
        .unwrap()
        .id;
}
fn request(f: &Fixture, index: usize) -> PeerAuthorityRequest {
    let mut r = f.native_request_for(index, 300, &format!("handoff-prepared-{index}"));
    let PeerCommand::NativeConnectionOffer(intent) = &mut r.command else {
        panic!()
    };
    intent.offer["handoff_preparation"] = json!({"profile":"connect-handoff-preparation-v1"});
    r
}
#[test]
fn handoff_preparation_uses_its_own_immutable_signed_offer_and_no_caller_operation() {
    let mut f = Fixture::new();
    grant(&mut f, 0);
    let r = request(&f, 0);
    let proof = f.admit(&r);
    let first = f
        .store
        .prepare_native_offer_with_peer(
            &f.keys[0],
            &proof,
            Some(connect_protocol::provisioning::EndpointTransport::Quic),
        )
        .unwrap();
    assert!(first.effects.is_empty());
    let projection = f
        .store
        .project_connection_with_peer(&f.keys[0], &proof)
        .unwrap();
    assert!(projection.handoff_preparation);
    assert!(projection.private_consultation.is_none());
    assert_eq!(projection.endpoint_capabilities, vec!["audio"]);
    assert_eq!(
        f.store
            .prepare_native_offer_with_peer(
                &f.keys[0],
                &proof,
                Some(connect_protocol::provisioning::EndpointTransport::Quic)
            )
            .unwrap(),
        first
    );
    let c = f.store.connection().unwrap();
    let resources: String = c
        .query_row(
            "SELECT resources_json FROM native_offer_operations WHERE operation_record_id=?1",
            [first.record_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Vec<String>>(&resources).unwrap(),
        vec![projection.session.actor.identity_id]
    );
    drop(c);
    drop(proof);
    assert_eq!(f.count("native_handoffs"), 0);
    assert_eq!(f.count("native_consultation_legs"), 0);
}
#[test]
fn handoff_preparation_requires_named_current_target_permission_and_rejects_forged_or_mixed_profiles(
) {
    for bad in [
        "ungranted",
        "profile",
        "ready-claim",
        "mixed",
        "video",
        "one-way",
    ] {
        let mut f = Fixture::new();
        if bad != "ungranted" {
            grant(&mut f, 0);
        }
        let mut r = request(&f, 0);
        let PeerCommand::NativeConnectionOffer(intent) = &mut r.command else {
            panic!()
        };
        match bad {
            "profile" => intent.offer["handoff_preparation"]["profile"] = json!("other"),
            "ready-claim" => intent.offer["handoff_preparation"]["ready"] = json!(true),
            "mixed" => {
                intent.offer["private_consultation"] =
                    json!({"profile":"connect-private-audio-v1","operation_record_id":"copied"})
            }
            "video" => intent.offer["streams_offered"][0]["kind"] = json!("video"),
            "one-way" => intent.offer["streams_offered"][0]["direction"] = json!("sendonly"),
            _ => {}
        }
        let result = f
            .store
            .admit_peer_authority(
                &r,
                &crate::AuthorityWait::new(std::time::Duration::from_secs(1)),
            )
            .and_then(|proof| {
                f.store.prepare_native_offer_with_peer(
                    &f.keys[0],
                    &proof,
                    Some(connect_protocol::provisioning::EndpointTransport::Quic),
                )
            });
        assert!(
            result.is_err()
                || result.unwrap().state == connect_protocol::transitions::OperationState::Failed,
            "{bad}"
        );
        assert_eq!(f.count("connections"), 0, "{bad}");
        assert_eq!(f.count("operation_effects"), 0, "{bad}");
    }
}
#[test]
fn handoff_preparation_denies_all_peer_routes_and_loses_own_media_when_its_grant_is_removed() {
    let mut f = Fixture::new();
    grant(&mut f, 0);
    let r = request(&f, 0);
    let prepared = consultation_reads::ready_request(&f, 0, r);
    let other = consultation_reads::ready(&f, 1);
    let read = f.store.try_native_media_read(&prepared, None).unwrap();
    assert!(read.target().handoff_preparation);
    drop(read);
    assert!(f
        .store
        .try_native_media_read(&prepared, Some(&other))
        .is_err());
    assert!(f
        .store
        .try_native_media_read(&other, Some(&prepared))
        .is_err());
    let actor = f.store.revalidate(&f.keys[0]).unwrap();
    let _writer = f
        .store
        .acquire_authority(
            &[
                crate::AuthorityScope::identity(&actor.identity_id, crate::AuthorityMode::Write)
                    .unwrap(),
            ],
            &crate::AuthorityWait::new(std::time::Duration::from_secs(1)),
        )
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET operations='[\"connection.offer\"]' WHERE id=?1",
            [&f.keys[0]],
        )
        .unwrap();
    drop(_writer);
    assert!(f.store.try_native_media_read(&prepared, None).is_err());
    assert!(f.store.try_native_media_read(&other, None).is_ok());
}
#[test]
fn handoff_preparation_metadata_is_target_only_until_attributed_handoff_without_republishing_old_events(
) {
    use connect_protocol::transitions::*;
    let mut f = Fixture::new();
    grant(&mut f, 0);
    grant(&mut f, 1);
    grant(&mut f, 2);
    grant(&mut f, 3);
    let target = request(&f, 1);
    let target = consultation_reads::ready_request(&f, 1, target);
    let sources = vec![
        consultation_reads::ready(&f, 0),
        consultation_reads::ready(&f, 2),
        target,
    ];
    let connection = sources[2].connection_id();
    let preparation_events = || {
        let c = f.store.connection().unwrap();
        let mut q = c.prepare("SELECT id,recipient FROM journal WHERE json_extract(data_json,'$.connection_id')=?1 AND kind='connection_preparing' ORDER BY id").unwrap();
        q.query_map([connection], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
    };
    let before_events = preparation_events();
    assert!(!before_events.is_empty());
    let target_identity = f.store.revalidate(&f.keys[1]).unwrap().identity_id;
    assert!(before_events
        .iter()
        .all(|(_, recipient)| *recipient == target_identity));
    let selected = |index: usize| vec![(f.pids[index].clone(), 1)];
    let visible = |index: usize| {
        let allowed = super::super::private_consultation::observation_allowed(
            &f.store.connection().unwrap(),
            connection,
            &selected(index),
        )
        .unwrap();
        let actor = f.store.revalidate(&f.keys[index]).unwrap();
        assert_eq!(
            allowed,
            super::super::private_consultation::visible(
                &f.store.connection().unwrap(),
                &actor,
                connection
            )
            .unwrap()
        );
        allowed
    };
    for i in 0..4 {
        assert_eq!(visible(i), i == 1);
    }
    let command = TransitionCommand {
        operation_id: "admit-prepared-target".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(sources[0].connection_id().into()),
        expected_revision: 3,
        intent: TransitionIntent::Handoff {
            source_participant: f.pids[0].clone(),
            target_participant: f.pids[1].clone(),
            retained_connid: sources[1].connection_id().into(),
            candidate_connid: Some(connection.into()),
            outgoing: OutgoingDisposition::Observe,
            role_scope: RoleScope::Session,
            expected_conversation_revision: None,
        },
        payload_extensions: std::collections::BTreeMap::from([(
            "policy_ref".into(),
            json!(sources[2].offer_record_id()),
        )]),
    };
    f.store
        .admit_transition_command(&f.keys[0], command.clone())
        .unwrap();
    let origin = f.store.native_handoff_origin(&f.keys[0], command).unwrap();
    let plan = handoff_custody::plan(&f, &sources, &origin, OutgoingDisposition::Observe);
    let before_effects = f.count("operation_effects");
    for wrong in [
        serde_json::Value::Null,
        json!(sources[0].runtime_connection_id()),
        json!(sources[1].runtime_connection_id()),
    ] {
        let mut forged = plan.clone();
        forged["activate_muted"] = wrong;
        let read = f.store.try_native_handoff_read(&origin, &sources).unwrap();
        assert!(f.store.prepare_native_handoff(read, forged).is_err());
        assert_eq!(f.count("native_handoffs"), 0);
        assert_eq!(f.count("operation_effects"), before_effects);
    }
    let receipt = handoff_roles::capture(&f, &sources, &origin, OutgoingDisposition::Observe);
    f.store.complete_native_handoff(&receipt).unwrap().unwrap();
    for i in 0..4 {
        assert_eq!(visible(i), i != 3);
    }
    assert_eq!(preparation_events(), before_events);
    // Transfer again: the first incoming becomes Observer. The first Observer
    // is closed out, and only the new captured audience sees current metadata.
    let next = consultation_reads::ready_request(&f, 3, request(&f, 3));
    let next_sources = vec![
        sources[2].clone(),
        sources[1].clone(),
        next,
        sources[0].clone(),
    ];
    let command = TransitionCommand {
        operation_id: "second-prepared-target".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(connection.into()),
        expected_revision: 4,
        intent: TransitionIntent::Handoff {
            source_participant: f.pids[1].clone(),
            target_participant: f.pids[3].clone(),
            retained_connid: sources[1].connection_id().into(),
            candidate_connid: Some(next_sources[2].connection_id().into()),
            outgoing: OutgoingDisposition::Observe,
            role_scope: RoleScope::Session,
            expected_conversation_revision: None,
        },
        payload_extensions: std::collections::BTreeMap::from([(
            "policy_ref".into(),
            json!(next_sources[2].offer_record_id()),
        )]),
    };
    f.store
        .admit_transition_command(&f.keys[1], command.clone())
        .unwrap();
    let origin = f.store.native_handoff_origin(&f.keys[1], command).unwrap();
    let mut plan = handoff_custody::plan(&f, &next_sources, &origin, OutgoingDisposition::Observe);
    let read = f
        .store
        .try_native_handoff_read(&origin, &next_sources)
        .unwrap();
    assert!(
        f.store.prepare_native_handoff(read, plan.clone()).is_err(),
        "must close the prior observer receive programme"
    );
    plan["receivers"][sources[0].runtime_connection_id()] = json!([]);
    let read = f
        .store
        .try_native_handoff_read(&origin, &next_sources)
        .unwrap();
    let lease = f
        .store
        .prepare_native_handoff(read, plan.clone())
        .unwrap()
        .unwrap();
    let attempt = handoff_custody::begin(&f, lease);
    let receipt = attempt
        .applied(handoff_custody::observation(&next_sources, &origin, &plan))
        .unwrap();
    f.store.record_native_handoff_applied(&receipt).unwrap();
    f.store.complete_native_handoff(&receipt).unwrap().unwrap();
    for i in 0..4 {
        assert_eq!(visible(i), i != 0);
    }
    assert_eq!(preparation_events(), before_events);
}

#[test]
fn handoff_preparation_is_not_opened_by_an_unrelated_handoffs_complete_scene_read() {
    use connect_protocol::transitions::*;
    let mut f = Fixture::new();
    for i in 0..4 {
        grant(&mut f, i);
    }
    let request = request(&f, 3);
    let mut public = request.clone();
    public.command = PeerCommand::NativeConnectionOffer(f.request(3, "unrelated-public").intent);
    let prepared = consultation_reads::ready_request(&f, 3, request);
    let unrelated = consultation_reads::ready_request(&f, 3, public);
    let sources = vec![
        consultation_reads::ready(&f, 0),
        consultation_reads::ready(&f, 2),
        consultation_reads::ready(&f, 1),
        prepared,
        unrelated,
    ];
    let command = TransitionCommand {
        operation_id: "unrelated-handoff-scene".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(sources[0].connection_id().into()),
        expected_revision: 3,
        intent: TransitionIntent::Handoff {
            source_participant: f.pids[0].clone(),
            target_participant: f.pids[1].clone(),
            retained_connid: sources[1].connection_id().into(),
            candidate_connid: Some(sources[2].connection_id().into()),
            outgoing: OutgoingDisposition::Observe,
            role_scope: RoleScope::Session,
            expected_conversation_revision: None,
        },
        payload_extensions: std::collections::BTreeMap::from([(
            "policy_ref".into(),
            json!(sources[2].offer_record_id()),
        )]),
    };
    f.store
        .admit_transition_command(&f.keys[0], command.clone())
        .unwrap();
    let origin = f.store.native_handoff_origin(&f.keys[0], command).unwrap();
    let read = f.store.try_native_handoff_read(&origin, &sources).unwrap();
    for (receiver, publisher) in [(3, 4), (4, 3)] {
        assert!(read
            .media(
                sources[receiver].connection_id(),
                Some(sources[publisher].connection_id())
            )
            .is_none());
    }
    assert!(read
        .media(sources[1].connection_id(), Some(sources[2].connection_id()))
        .is_some());
}
