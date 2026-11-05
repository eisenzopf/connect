//! Store custody and current authorization controls using controlled engine
//! observations. Actual decoded private replacement/return lives in server tests.
use super::*;
use crate::connection_control::private_consultation::replacement_lineage;

fn private_scene() -> (Movement, String) {
    let (f, sources, partner, consultation, _execution, receipt) =
        consultation_audiences::established_fixture();
    f.store
        .record_native_consultation_decoded(&receipt)
        .unwrap();
    assert_eq!(
        f.store
            .complete_native_consultation(&receipt)
            .unwrap()
            .unwrap()
            .state,
        OperationState::Completed
    );
    let parent = consultation.record_id().to_owned();
    let target = super::super::endpoint(&f, vec!["audio".into()], "replacement");
    let key = super::super::key(&f);
    let revision = f
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
        operation_id: "replace-private-primary".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(sources[0].connection_id().into()),
        expected_revision: revision,
        intent: TransitionIntent::Replace {
            endpoint_id: Some(target),
            candidate_connid: None,
            retained_connid: partner.connection_id().into(),
            video: None,
        },
        payload_extensions: Default::default(),
    };
    f.store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let origin = f
        .store
        .native_replacement_origin(&key, command.clone())
        .unwrap();
    let scene = vec![
        sources[0].clone(),
        partner,
        sources[1].clone(),
        sources[2].clone(),
    ];
    let prepared = super::super::allocation::prepared_target_for_scene(
        &["stream.subscribe"],
        (f, key, command, origin, scene),
    );
    (fixture_prepared(true, prepared), parent)
}
fn private_capture(m: &Movement, parent: &str) -> crate::NativeReplacementCommitted {
    let read =
        m.f.store
            .try_native_replacement_scene_read(&m.origin, &m.sources)
            .unwrap();
    let attempt =
        m.f.store
            .begin_native_replacement_commit(m.origin.clone(), &read)
            .unwrap();
    let expected = attempt.expected().unwrap();
    let mut observed = ready(&read, &expected);
    observed.private_consultation = Some(parent.into());
    attempt.committed(expected, &observed).unwrap()
}
fn complete(m: &Movement, parent: &str) -> Arc<NativeMediaOrigin> {
    let captured = private_capture(m, parent);
    m.f.store
        .record_native_replacement_committed(&captured)
        .unwrap();
    m.f.store
        .project_native_replacement_committed(&captured, &MetadataOnlyInstalled)
        .unwrap()
}
fn original_selection(m: &Movement, parent: &str) -> String {
    m.f.store
        .connection()
        .unwrap()
        .query_row(
            "SELECT selection_json FROM native_private_lifetimes WHERE operation_record_id=?1",
            [parent],
            |r| r.get(0),
        )
        .unwrap()
}
fn assert_private(m: &Movement, target: &NativeMediaOrigin) {
    assert!(m
        .f
        .store
        .try_native_media_read(target, Some(&m.sources[1]))
        .unwrap()
        .current());
    assert!(m
        .f
        .store
        .try_native_media_read(&m.sources[1], Some(target))
        .unwrap()
        .current());
    for outside in &m.sources[2..] {
        assert!(m
            .f
            .store
            .try_native_media_read(target, Some(outside))
            .is_err());
        assert!(m
            .f
            .store
            .try_native_media_read(outside, Some(target))
            .is_err());
    }
}

#[test]
fn private_successor_requires_actual_matching_parent_and_preserves_immutable_history_and_return() {
    let (m, parent) = private_scene();
    let original = original_selection(&m, &parent);
    let target = complete(&m, &parent);
    assert_private(&m, &target);
    assert_eq!(original_selection(&m, &parent), original);
    let c = m.f.store.connection().unwrap();
    assert_eq!(
        replacement_lineage::primary_for_return(&c, &parent, m.f.store.runtime_epoch()).unwrap(),
        target.connection_id()
    );
    assert_eq!(
        replacement_lineage::floor(&c, m.sources[1].connection_id())
            .unwrap()
            .unwrap()
            .partner,
        target.connection_id()
    );
    assert_eq!(
        replacement_lineage::floor(&c, target.connection_id())
            .unwrap()
            .unwrap()
            .record,
        parent
    );
    drop(c);
    let identity = m.f.store.revalidate(&m.key).unwrap().identity_id;
    let mut scope = Scope::collaboration(vec!["*".into()]);
    scope
        .operations
        .extend(["session.consult.return".into(), "stream.subscribe".into()]);
    let return_key =
        m.f.store
            .issue(&identity, scope, crate::now() + 600)
            .unwrap()
            .id;
    let revision =
        m.f.store
            .connection()
            .unwrap()
            .query_row(
                "SELECT topology_revision FROM sessions WHERE id='session'",
                [],
                |r| r.get(0),
            )
            .unwrap();
    let command = TransitionCommand {
        operation_id: "return-successor-private".into(),
        conversation_id: m.f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(target.connection_id().into()),
        expected_revision: revision,
        intent: TransitionIntent::ConsultReturn {
            consultation_record_id: parent.clone(),
            retained_connid: m.sources[2].connection_id().into(),
        },
        payload_extensions: Default::default(),
    };
    m.f.store
        .admit_transition_command(&return_key, command.clone())
        .unwrap();
    let origin =
        m.f.store
            .native_consultation_return_origin(&return_key, command)
            .unwrap();
    let mut actual = m.sources[1..].to_vec();
    actual.push(target);
    assert!(m
        .f
        .store
        .try_native_return_read(&origin, &actual, &[])
        .is_ok());
    assert_eq!(original_selection(&m, &parent), original);
}

#[test]
fn private_successor_wrong_or_missing_parent_cannot_be_captured_as_public() {
    for supplied in [None, Some("other-private-root")] {
        let (m, _parent) = private_scene();
        let read =
            m.f.store
                .try_native_replacement_scene_read(&m.origin, &m.sources)
                .unwrap();
        let attempt =
            m.f.store
                .begin_native_replacement_commit(m.origin.clone(), &read)
                .unwrap();
        let expected = attempt.expected().unwrap();
        let mut observed = ready(&read, &expected);
        observed.private_consultation = supplied.map(str::to_owned);
        assert!(attempt.committed(expected, &observed).is_err());
    }
}

#[test]
fn private_successor_requires_current_target_key_device_membership_generation_and_runtime() {
    for mutation in [
        "key",
        "credential",
        "device",
        "generation",
        "membership",
        "runtime",
        "expiry",
    ] {
        let (m, parent) = private_scene();
        let target = complete(&m, &parent);
        assert_private(&m, &target);
        let c = m.f.store.connection().unwrap();
        match mutation {
            "key" => {
                c.execute(
                    "UPDATE peer_signing_keys SET revoked=1 WHERE key_id=?1",
                    [&m.target_request.key_id],
                )
                .unwrap();
            }
            "credential" => {
                c.execute(
                    "UPDATE credentials SET revoked=1 WHERE id=?1",
                    [&m.target_request.credentials[0]],
                )
                .unwrap();
            }
            "device" => {
                c.execute(
                    "UPDATE devices SET revoked=1 WHERE id=?1",
                    [&m.target_request.device.id],
                )
                .unwrap();
            }
            "generation" => {
                c.execute(
                    "UPDATE connections SET generation=generation+1 WHERE id=?1",
                    [target.connection_id()],
                )
                .unwrap();
            }
            "membership" => {
                c.execute(
                    "UPDATE participants SET active=0 WHERE participant_id=?1",
                    [target.participant_id()],
                )
                .unwrap();
            }
            "runtime" => {
                c.execute(
                    "UPDATE connections SET observed_runtime_id='wrong-runtime' WHERE id=?1",
                    [target.connection_id()],
                )
                .unwrap();
            }
            "expiry" => {
                c.execute(
                    "UPDATE credentials SET expires_at=?2 WHERE id=?1",
                    params![m.target_request.credentials[0], crate::now() - 1],
                )
                .unwrap();
            }
            _ => unreachable!(),
        }
        drop(c);
        assert!(
            m.f.store
                .try_native_media_read(target.as_ref(), Some(&m.sources[1]))
                .is_err(),
            "{mutation}"
        );
        assert!(
            m.f.store
                .try_native_media_read(&m.sources[1], Some(target.as_ref()))
                .is_err(),
            "{mutation}"
        );
        assert!(
            m.f.store
                .try_native_media_read(&m.sources[2], Some(target.as_ref()))
                .is_err(),
            "{mutation}: no public fallback"
        );
    }
}

#[test]
fn private_successor_old_key_cannot_authorize_new_physical_owner_or_revoke_its_independent_grant() {
    let (m, parent) = private_scene();
    let target = complete(&m, &parent);
    assert_private(&m, &target);
    let admin = m.f.store.revalidate(&m.f.admin).unwrap();
    m.f.store.revoke(&admin, &m.f.keys[0]).unwrap();
    assert!(m
        .f
        .store
        .try_native_media_read(&m.sources[0], Some(&m.sources[1]))
        .is_err());
    assert_private(&m, &target);
    m.f.store
        .revoke(&admin, &m.target_request.credentials[0])
        .unwrap();
    assert!(m
        .f
        .store
        .try_native_media_read(&m.sources[1], Some(&target))
        .is_err());
}

#[test]
fn private_successor_original_private_expiry_is_not_renewed_by_longer_target_grant() {
    let (m, parent) = private_scene();
    // A short original current grant followed by a longer target grant must not
    // extend the private context. The deadline becomes immutable at capture.
    let original_until = crate::now() + 2;
    m.f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET expires_at=?2 WHERE id=?1",
            params![m.f.keys[0], original_until],
        )
        .unwrap();
    let captured = private_capture(&m, &parent);
    m.f.store
        .record_native_replacement_committed(&captured)
        .unwrap();
    let target =
        m.f.store
            .project_native_replacement_committed(&captured, &MetadataOnlyInstalled)
            .unwrap();
    let c = m.f.store.connection().unwrap();
    let original = original_selection_unlocked(&c, &parent);
    let resolved = replacement_lineage::resolve(
        &c,
        &parent,
        &original,
        m.sources[0].connection_id(),
        m.f.store.runtime_epoch(),
    )
    .unwrap();
    let original: Value = serde_json::from_str(&original).unwrap();
    let upper = original["private"]
        .as_array()
        .unwrap()
        .iter()
        .chain(std::iter::once(&original["remote"]))
        .filter_map(|v| v["physical_expires_at"].as_i64())
        .min()
        .unwrap();
    assert!(resolved.private_expires_at.unwrap() <= upper);
    drop(c);
    let read =
        m.f.store
            .try_native_media_read(&m.sources[1], Some(&target))
            .unwrap();
    assert!(read.expires_at() <= resolved.private_expires_at.unwrap());
    assert!(resolved.private_expires_at.unwrap() <= original_until);
    drop(read);
    let wait = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while crate::now() < original_until && std::time::Instant::now() < wait {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(crate::now() >= original_until);
    assert!(
        m.f.store
            .revalidate(&m.target_request.credentials[0])
            .is_ok(),
        "fresh target grant is still live"
    );
    assert!(
        m.f.store
            .try_native_media_read(&m.sources[1], Some(&target))
            .is_err(),
        "private lifetime cannot renew through it"
    );
    assert!(m
        .f
        .store
        .try_native_media_read(&m.sources[2], Some(&target))
        .is_err());
}
fn original_selection_unlocked(c: &rusqlite::Connection, parent: &str) -> String {
    c.query_row(
        "SELECT selection_json FROM native_private_lifetimes WHERE operation_record_id=?1",
        [parent],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn private_successor_stopped_capture_never_publishes_a_private_or_public_owner() {
    let (m, parent) = private_scene();
    let captured = private_capture(&m, &parent);
    m.f.store
        .record_native_replacement_committed(&captured)
        .unwrap();
    let origin = captured.activation().origin();
    let absent =
        super::super::candidate::Absent(origin.runtime_connection_id().into(), origin.generation());
    let stopped =
        m.f.store
            .project_native_replacement_stopped(&captured, &absent)
            .unwrap();
    assert!(m.f.store.try_native_media_read(&stopped, None).is_err());
    assert!(m
        .f
        .store
        .try_native_media_read(&m.sources[1], Some(&stopped))
        .is_err());
    let c = m.f.store.connection().unwrap();
    assert_ne!(
        replacement_lineage::primary_for_return(&c, &parent, m.f.store.runtime_epoch()).unwrap(),
        stopped.connection_id()
    );
}

#[test]
fn private_successor_same_capture_replay_has_one_effect_and_conflicting_history_is_refused() {
    let (m, parent) = private_scene();
    let captured = private_capture(&m, &parent);
    m.f.store
        .record_native_replacement_committed(&captured)
        .unwrap();
    let target =
        m.f.store
            .project_native_replacement_committed(&captured, &MetadataOnlyInstalled)
            .unwrap();
    let before = Store::inspect_snapshot(m.f.store.path()).unwrap();
    let replay =
        m.f.store
            .project_native_replacement_committed(&captured, &MetadataOnlyInstalled)
            .unwrap();
    assert_eq!(target.connection_id(), replay.connection_id());
    assert_eq!(Store::inspect_snapshot(m.f.store.path()).unwrap(), before);
    let c = m.f.store.connection().unwrap();
    let selected = original_selection_unlocked(&c, &parent);
    let mut changed: Value = serde_json::from_str(&selected).unwrap();
    changed["private"][0]["generation"] = json!(999);
    assert!(replacement_lineage::resolve(
        &c,
        &parent,
        &changed.to_string(),
        m.sources[0].connection_id(),
        m.f.store.runtime_epoch()
    )
    .is_err());
    assert!(c.execute("UPDATE native_replacement_steps SET observation_json='{}' WHERE operation_record_id=?1 AND phase='commit'",[target.offer_record_id()]).is_err(),"captured lineage is immutable");
}

#[test]
fn private_successor_cancelled_before_commit_retains_original_private_pair() {
    let (m, _parent) = private_scene();
    cancel(&m.f, &m.key, &m.command);
    assert!(m
        .f
        .store
        .try_native_replacement_scene_read(&m.origin, &m.sources)
        .is_err());
    assert!(m
        .f
        .store
        .try_native_media_read(&m.sources[0], Some(&m.sources[1]))
        .unwrap()
        .current());
    assert!(m
        .f
        .store
        .try_native_media_read(&m.sources[1], Some(&m.sources[0]))
        .unwrap()
        .current());
    assert!(m
        .f
        .store
        .try_native_media_read(&m.sources[2], Some(&m.sources[0]))
        .is_err());
}

fn second_scene(m: Movement, first: Arc<NativeMediaOrigin>) -> Movement {
    let Movement {
        f, key, sources, ..
    } = m;
    let endpoint = super::super::endpoint(&f, vec!["audio".into()], "replacement-2");
    let revision = f
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
        operation_id: "replace-private-primary-again".into(),
        conversation_id: f.cid.clone(),
        session_id: "session".into(),
        connection_id: Some(first.connection_id().into()),
        expected_revision: revision,
        intent: TransitionIntent::Replace {
            endpoint_id: Some(endpoint),
            candidate_connid: None,
            retained_connid: sources[1].connection_id().into(),
            video: None,
        },
        payload_extensions: Default::default(),
    };
    f.store
        .admit_transition_command(&key, command.clone())
        .unwrap();
    let origin = f
        .store
        .native_replacement_origin(&key, command.clone())
        .unwrap();
    let mut scene = vec![first];
    scene.extend(sources[1..].iter().cloned());
    let prepared = super::super::allocation::prepared_target_for_scene_named(
        &["stream.subscribe"],
        (f, key, command, origin, scene),
        "second-private",
    );
    fixture_prepared(true, prepared)
}

#[test]
fn private_successor_second_generation_pins_exact_confirmed_predecessor_and_old_floor_stays_restrictive(
) {
    let (m, parent) = private_scene();
    let original = original_selection(&m, &parent);
    let first = complete(&m, &parent);
    let first_record = first.offer_record_id().to_owned();
    let m = second_scene(m, first.clone());
    let second = complete(&m, &parent);
    assert_eq!(second.generation(), first.generation() + 1);
    assert_private(&m, &second);
    assert!(m
        .f
        .store
        .try_native_media_read(&first, Some(&m.sources[1]))
        .is_err());
    let c = m.f.store.connection().unwrap();
    let observation:String=c.query_row("SELECT observation_json FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit'",[second.offer_record_id()],|r|r.get(0)).unwrap();
    let value: Value = serde_json::from_str(&observation).unwrap();
    assert_eq!(
        value["projection"]["private_successor"]["predecessor_record_id"],
        first_record
    );
    assert!(
        value["projection"]["private_successor"]["predecessor_capture_sha256"]
            .as_str()
            .is_some()
    );
    assert_eq!(
        replacement_lineage::floor(&c, first.connection_id())
            .unwrap()
            .unwrap()
            .partner,
        m.sources[1].connection_id()
    );
    assert_eq!(
        replacement_lineage::primary_for_return(&c, &parent, m.f.store.runtime_epoch()).unwrap(),
        second.connection_id()
    );
    drop(c);
    assert_eq!(original_selection(&m, &parent), original);
}

#[test]
fn private_successor_changed_or_missing_predecessor_root_and_physical_tuple_are_refused() {
    for change in [
        "root",
        "digest",
        "connection",
        "epoch",
        "core",
        "missing-parent",
        "foreign-parent",
    ] {
        let (m, parent) = private_scene();
        let first = complete(&m, &parent);
        let m = second_scene(m, first);
        let second = complete(&m, &parent);
        assert_private(&m, &second);
        // The production trigger rejects edits. For this isolated corruption
        // control only, remove that protection to exercise the reader's exact
        // ancestry checks against still-retained real committed predecessor rows.
        let c = m.f.store.connection().unwrap();
        let raw:String=c.query_row("SELECT observation_json FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit'",[second.offer_record_id()],|r|r.get(0)).unwrap();
        let mut value: Value = serde_json::from_str(&raw).unwrap();
        let lineage = &mut value["projection"]["private_successor"];
        match change {
            "missing-parent" => *lineage = Value::Null,
            "foreign-parent" => lineage["consultation_record_id"] = json!("missing-consultation"),
            "root" => lineage["predecessor_record_id"] = json!("missing-predecessor"),
            "digest" => lineage["predecessor_capture_sha256"] = json!("wrong-capture"),
            "connection" => {
                lineage["predecessor"]["connection_id"] = json!(m.sources[2].connection_id())
            }
            "epoch" => lineage["predecessor"]["membership_epoch"] = json!(999),
            "core" => {
                lineage["predecessor"]["runtime_connection_id"] = json!("wrong-physical-port")
            }
            _ => unreachable!(),
        }
        assert!(c.execute("UPDATE native_replacement_steps SET observation_json=?2 WHERE operation_record_id=?1 AND phase='commit'",params![second.offer_record_id(),value.to_string()]).is_err());
        c.execute_batch("DROP TRIGGER native_replacement_step_immutable")
            .unwrap();
        c.execute("UPDATE native_replacement_steps SET observation_json=?2 WHERE operation_record_id=?1 AND phase='commit'",params![second.offer_record_id(),value.to_string()]).unwrap();
        drop(c);
        assert!(
            m.f.store
                .try_native_media_read(&m.sources[1], Some(&second))
                .is_err(),
            "{change}"
        );
        assert!(
            m.f.store
                .try_native_media_read(&m.sources[2], Some(&second))
                .is_err(),
            "{change}: never public"
        );
    }
}

#[test]
fn private_successor_restart_cannot_reopen_the_previous_runtime_or_public_fallback() {
    let (m, parent) = private_scene();
    let target = complete(&m, &parent);
    let path = m.f.store.path().to_owned();
    drop(m.f.store);
    let successor = Store::open(&path, connect_protocol::Limits::default()).unwrap();
    assert!(successor.try_native_media_read(&target, None).is_err());
    assert!(successor
        .try_native_media_read(&m.sources[1], Some(&target))
        .is_err());
    let c = successor.connection().unwrap();
    assert!(
        crate::connection_control::private_consultation::active::current(&successor, &c, &parent)
            .is_err()
    );
    assert!(
        replacement_lineage::floor(&c, target.connection_id())
            .unwrap()
            .is_some(),
        "historical context remains restrictive without granting current IO"
    );
}

#[test]
fn private_successor_new_target_still_requires_original_private_recipient_and_subscription_scope() {
    for permission in ["subscription", "recipient"] {
        let (m, parent) = private_scene();
        let target = complete(&m, &parent);
        assert_private(&m, &target);
        let mut scope =
            m.f.store
                .revalidate(&m.target_request.credentials[0])
                .unwrap()
                .scope;
        match permission {
            "subscription" => scope
                .operations
                .retain(|operation| operation != "stream.subscribe"),
            "recipient" => scope.recipients.clear(),
            _ => unreachable!(),
        }
        m.f.store
            .connection()
            .unwrap()
            .execute(
                "UPDATE credentials SET operations=?2,recipients=?3 WHERE id=?1",
                params![
                    m.target_request.credentials[0],
                    serde_json::to_string(&scope.operations).unwrap(),
                    serde_json::to_string(&scope.recipients).unwrap()
                ],
            )
            .unwrap();
        assert!(
            m.f.store
                .try_native_media_read(&m.sources[1], Some(&target))
                .is_err(),
            "{permission}"
        );
        assert!(
            m.f.store
                .try_native_media_read(&target, Some(&m.sources[1]))
                .is_err(),
            "{permission}"
        );
        assert!(m
            .f
            .store
            .try_native_media_read(&m.sources[2], Some(&target))
            .is_err());
    }
}

#[test]
fn private_successor_packet_interval_drains_before_original_target_authority_writer() {
    let (m, parent) = private_scene();
    let target = complete(&m, &parent);
    let interval =
        m.f.store
            .try_native_media_read(&m.sources[1], Some(&target))
            .unwrap();
    let identity =
        m.f.store
            .revalidate(&m.target_request.credentials[0])
            .unwrap()
            .identity_id;
    let scopes = [crate::AuthorityScope::identity(&identity, crate::AuthorityMode::Write).unwrap()];
    assert!(m.f.store.try_acquire_authority(&scopes).is_err());
    assert!(interval.current());
    drop(interval);
    drop(m.f.store.try_acquire_authority(&scopes).unwrap());
    assert_private(&m, &target);
}
