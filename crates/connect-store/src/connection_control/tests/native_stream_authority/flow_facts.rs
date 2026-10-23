//! Controlled attribution/rollback fixtures; production owns actual IO proof.
use super::*;
mod activation;
struct Observation(crate::NativeConnectionFlowFact);
impl crate::NativeConnectionFlowObservation for Observation {
    fn observe(&self) -> crate::Result<crate::NativeConnectionFlowFact> {
        Ok(self.0.clone())
    }
}
fn observed(origin: &crate::NativeMediaOrigin) -> Observation {
    Observation(crate::NativeConnectionFlowFact {
        runtime_connection_id: origin.runtime_connection_id().into(),
        generation: origin.generation(),
        stream_revision: 1,
        credential_revision: 1,
        observed_at_millis: chrono::Utc::now().timestamp_millis(),
        streams: vec!["audio".into()],
        negotiated_capabilities: json!({"audio_codecs":[{"name":"g.711-mu","params":{"sample_rate":8000,"channels":1}}]}),
    })
}
#[test]
fn original_flow_fact_is_immutable_and_recapture_does_not_create_effects_or_rewrite_time() {
    let (f, _, _, origin) = parent_permissions_origin(true, true, false, false, true);
    let read = f.store.try_native_media_read(&origin, None).unwrap();
    let first = observed(&origin);
    let before = (
        f.count("operations"),
        f.count("operation_effects"),
        f.count("journal"),
    );
    let saved = f
        .store
        .record_native_connection_flow(&origin, &read, &first)
        .unwrap();
    let mut again = observed(&origin);
    again.0.stream_revision = 2;
    assert!(
        f.store
            .record_native_connection_flow(&origin, &read, &again)
            .unwrap()
            == saved
    );
    assert_eq!(f.count("native_connection_flow_facts"), 1);
    assert_eq!(
        (f.count("operations"), f.count("operation_effects")),
        (before.0, before.1)
    );
    assert_eq!(f.count("session_started_facts"), 0); // Other accepted Participants are not ready.
    let starts: u64 = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM journal WHERE kind IN ('session_started','session_participant_joined')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(f.count("journal"), before.2 + starts);
    assert!(starts > 0);
    assert!(f
        .store
        .connection()
        .unwrap()
        .execute(
            "UPDATE native_connection_flow_facts SET fact_json=fact_json",
            []
        )
        .is_err());
    assert!(f
        .store
        .connection()
        .unwrap()
        .execute("DELETE FROM native_connection_flow_facts", [])
        .is_err());
    let foreign = Store::open_administration(f.store.path(), Limits::default()).unwrap();
    assert!(foreign
        .record_native_connection_flow(&origin, &read, &first)
        .is_err());
    // SQL state fixture bypasses ordinary writer fencing deliberately: the
    // method must also resolve current generation, not merely trust its reader.
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE connections SET state='retired',generation=generation+1 WHERE id=?1",
            [origin.connection_id()],
        )
        .unwrap();
    assert!(f
        .store
        .record_native_connection_flow(&origin, &read, &first)
        .is_err());
    assert_eq!(f.count("native_connection_flow_facts"), 1);
}
#[test]
fn flow_fact_failure_rolls_back_and_original_source_can_recover_after_storage_returns() {
    let (f, _, _, origin) = parent_permissions_origin(true, true, false, false, true);
    let read = f.store.try_native_media_read(&origin, None).unwrap();
    let first = observed(&origin);
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER fail_flow_fact BEFORE INSERT ON native_connection_flow_facts BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(f
        .store
        .record_native_connection_flow(&origin, &read, &first)
        .is_err());
    assert_eq!(f.count("native_connection_flow_facts"), 0);
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_flow_fact")
        .unwrap();
    assert!(
        f.store
            .record_native_connection_flow(&origin, &read, &first)
            .unwrap()
            == first.0
    );
    let mut wrong = observed(&origin);
    wrong.0.runtime_connection_id = "another-core".into();
    assert!(f
        .store
        .record_native_connection_flow(&origin, &read, &wrong)
        .is_err());
    let mut wrong = observed(&origin);
    wrong.0.streams = vec!["audio".into(), "audio".into()];
    assert!(f
        .store
        .record_native_connection_flow(&origin, &read, &wrong)
        .is_err());
    assert_eq!(f.count("native_connection_flow_facts"), 1);
}

#[test]
fn session_start_failure_rolls_back_readiness_phase_and_journal_then_recovers_one_original_fact() {
    let (f, request, _, origin) = parent_permissions_origin(true, true, false, false, true);
    // The other accepted Participants have observed Connections. This last
    // source is the one that can now atomically start the Session.
    for actor in 1..4 {
        let peer = activation::completed_peer(&f, actor);
        activation::record(&f, &peer);
    }
    let read = f.store.try_native_media_read(&origin, None).unwrap();
    let first = observed(&origin);
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER fail_started BEFORE INSERT ON journal WHEN NEW.kind='session_started' BEGIN SELECT RAISE(ABORT,'isolated start failure'); END;").unwrap();
    let before = f.count("journal");
    assert!(f
        .store
        .record_native_connection_flow(&origin, &read, &first)
        .is_err());
    assert_eq!(f.count("native_connection_flow_facts"), 3);
    assert_eq!(f.count("session_started_facts"), 0);
    assert_eq!(f.count("journal"), before);
    assert_ne!(
        f.store
            .session_view(&f.keys[0], origin.conversation_id(), origin.session_id())
            .unwrap()
            .state,
        "active"
    );
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_started")
        .unwrap();
    f.store
        .record_native_connection_flow(&origin, &read, &first)
        .unwrap();
    let (original, audience) = f
        .store
        .session_started_template(origin.conversation_id(), origin.session_id())
        .unwrap();
    assert_eq!(original.session.state, "active");
    assert!(audience.len() > 1);
    assert_eq!(original.active_connections.len(), 4);
    assert!(original
        .active_connections
        .iter()
        .any(|c| c.connid == origin.connection_id()));
    let mut delivery = request.clone();
    delivery.command = PeerCommand::SessionStartedRead {
        conversation_id: origin.conversation_id().into(),
        session_id: origin.session_id().into(),
    };
    let proof = admit(&f, &delivery).unwrap();
    assert_eq!(
        f.store
            .session_started_with_peer(&f.keys[0], &proof)
            .unwrap(),
        original
    );
    drop(proof);
    let after = f.count("journal");
    f.store
        .record_native_connection_flow(&origin, &read, &first)
        .unwrap();
    assert_eq!(f.count("journal"), after);
    let current = f
        .store
        .session_view(&f.keys[0], origin.conversation_id(), origin.session_id())
        .unwrap();
    assert_eq!(
        current.topology_revision,
        original.session.topology_revision
    );
    assert!(f
        .store
        .connection()
        .unwrap()
        .execute("UPDATE session_started_facts SET fact_json=fact_json", [])
        .is_err());
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE sessions SET state='ended',ended_reason='fixture-ended' WHERE id=?1",
            [origin.session_id()],
        )
        .unwrap();
    assert!(f
        .store
        .record_native_connection_flow(&origin, &read, &first)
        .is_err());
    assert_eq!(
        f.store
            .session_started_template(origin.conversation_id(), origin.session_id())
            .unwrap()
            .0,
        original
    );
    assert!(admit(&f, &delivery).is_err());
}

#[test]
fn participant_join_failure_is_atomic_and_original_facts_survive_role_changes_and_terminal_loss() {
    let (f, mut request, _, origin) = parent_permissions_origin(true, true, false, false, true);
    let read = f.store.try_native_media_read(&origin, None).unwrap();
    let fact = observed(&origin);
    f.store.connection().unwrap().execute_batch("CREATE TRIGGER fail_join BEFORE INSERT ON journal WHEN NEW.kind='session_participant_joined' BEGIN SELECT RAISE(ABORT,'join storage fixture'); END;").unwrap();
    let before = f.count("journal");
    assert!(f
        .store
        .record_native_connection_flow(&origin, &read, &fact)
        .is_err());
    assert_eq!(f.count("session_participant_facts"), 0);
    assert_eq!(f.count("session_started_facts"), 0);
    assert_eq!(f.count("native_connection_flow_facts"), 0);
    assert_eq!(f.count("session_lifecycle_hints"), 0);
    assert_eq!(f.count("journal"), before);
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER fail_join")
        .unwrap();
    f.store
        .record_native_connection_flow(&origin, &read, &fact)
        .unwrap();
    let page = f.store.session_lifecycle_hint_page(0).unwrap();
    assert_eq!(page.len(), 1);
    // An actual Participant join precedes activation while other accepted
    // Participants still lack ready Connections.
    let connect_protocol::events::JournalData::SessionParticipantJoined(joined) = &page[0].2 else {
        panic!("joined source");
    };
    assert_eq!(joined.via_connection, origin.connection_id());
    assert_eq!(joined.membership_epoch, origin.membership_epoch());
    assert_eq!(joined.participant.participant_id, origin.participant_id());
    let original = page[0].2.clone();
    let id = joined.fact_id.clone();
    request.command = PeerCommand::SessionParticipantFactRead {
        conversation_id: origin.conversation_id().into(),
        session_id: origin.session_id().into(),
        fact_id: id.clone(),
    };
    let proof = admit(&f, &request).unwrap();
    assert_eq!(
        f.store
            .session_participant_fact_with_peer(&f.keys[0], &proof)
            .unwrap(),
        original
    );
    drop(proof);
    f.store
        .record_native_connection_flow(&origin, &read, &fact)
        .unwrap();
    assert_eq!(f.count("session_participant_facts"), 1);
    let mut c = f.store.connection().unwrap();
    c.execute("UPDATE session_participants SET role='changed-after-join' WHERE session_id=?1 AND participant_id=?2",params![origin.session_id(),origin.participant_id()]).unwrap();
    c.execute(
        "UPDATE connections SET state='retired',generation=generation+1 WHERE id=?1",
        [origin.connection_id()],
    )
    .unwrap();
    c.execute_batch("CREATE TRIGGER fail_left BEFORE INSERT ON journal WHEN NEW.kind='session_participant_left' BEGIN SELECT RAISE(ABORT,'left fixture'); END;").unwrap();
    {
        let tx = c.transaction().unwrap();
        assert!(crate::session_participant_lifecycle::reconcile(
            &tx,
            &f.store,
            origin.conversation_id(),
            origin.session_id(),
            None
        )
        .is_err());
    }
    assert_eq!(
        c.query_row("SELECT state FROM session_connected_presence", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "joined"
    );
    c.execute_batch("DROP TRIGGER fail_left").unwrap();
    {
        let tx = c.transaction().unwrap();
        crate::session_participant_lifecycle::reconcile(
            &tx,
            &f.store,
            origin.conversation_id(),
            origin.session_id(),
            None,
        )
        .unwrap();
        tx.commit().unwrap();
    }
    drop(c);
    let page = f.store.session_lifecycle_hint_page(0).unwrap();
    assert_eq!(page.len(), 2);
    assert_eq!(page[0].2, original);
    let connect_protocol::events::JournalData::SessionParticipantLeft(left) = &page[1].2 else {
        panic!("left source");
    };
    assert_eq!(left.reason, "all-connections-ended");
    assert_eq!(left.participant_id, origin.participant_id());
    assert!(
        admit(&f, &request).is_err(),
        "late joined output cannot outlive a newer left fact"
    );
    assert!(f
        .store
        .connection()
        .unwrap()
        .execute(
            "UPDATE session_participant_facts SET fact_json=fact_json",
            []
        )
        .is_err());
}

#[test]
fn participant_departure_keeps_owner_correlation_and_original_removed_recipient_epoch() {
    let (f, _, _, origin) = parent_permissions_origin(true, true, false, false, true);
    let read = f.store.try_native_media_read(&origin, None).unwrap();
    f.store
        .record_native_connection_flow(&origin, &read, &observed(&origin))
        .unwrap();
    drop(read);
    let owner = f.store.revalidate(&f.keys[0]).unwrap().identity_id;
    let mut c = f.store.connection().unwrap();
    let member = crate::audience::member(&c, &owner, origin.conversation_id(), true).unwrap();
    let tx = c.transaction().unwrap();
    tx.execute(
        "INSERT INTO journal_context VALUES(1,?1,'controlled-membership-removal')",
        [&owner],
    )
    .unwrap();
    tx.execute(
        "UPDATE participants SET active=0 WHERE participant_id=?1",
        [origin.participant_id()],
    )
    .unwrap();
    crate::session_participant_lifecycle::participant_departed(&tx, &f.store, &member, "kicked")
        .unwrap();
    let (pid,epoch,correlation):(String,u64,String)=tx.query_row("SELECT recipient_participant_id,recipient_epoch,json_extract(data_json,'$.operation_id') FROM journal WHERE kind='session_participant_left' AND recipient=?1",[&owner],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(pid, member.participant_id);
    assert_eq!(epoch, member.membership_epoch);
    assert_eq!(correlation, "controlled-membership-removal");
    let other:u64=tx.query_row("SELECT COUNT(*) FROM journal WHERE kind='session_participant_left' AND recipient!=?1 AND json_extract(data_json,'$.operation_id') IS NOT NULL",[&owner],|r|r.get(0)).unwrap();
    assert_eq!(other, 0);
    tx.execute("DELETE FROM journal_context", []).unwrap();
    tx.commit().unwrap();
}

#[test]
fn lifecycle_hint_metadata_poll_does_not_take_the_foreground_connection_mutex() {
    let (f, _, _, origin) = parent_permissions_origin(true, true, false, false, true);
    let read = f.store.try_native_media_read(&origin, None).unwrap();
    f.store
        .record_native_connection_flow(&origin, &read, &observed(&origin))
        .unwrap();
    drop(read);
    let store = &f.store;
    let held = store.connection().unwrap();
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    std::thread::scope(|scope| {
        scope.spawn(move || {
            let _ = send.send(store.session_lifecycle_hint_page(0));
        });
        let result = receive.recv_timeout(Duration::from_secs(1));
        drop(held);
        assert!(result.is_ok(),"background hint metadata must not occupy or wait for the foreground media/control SQL mutex");
        assert_eq!(result.unwrap().unwrap().len(), 1);
    });
}

fn grace_loss(f: &Fixture, origin: &crate::NativeMediaOrigin, expired: bool) {
    let c = f.store.connection().unwrap();
    c.execute(
        "UPDATE connections SET state='retired',generation=generation+1 WHERE id=?1",
        [origin.connection_id()],
    )
    .unwrap();
    crate::session_reconnect::observe_loss(
        &c,
        &f.store,
        origin.session_id(),
        chrono::Utc::now().timestamp_millis() - if expired { 31_000 } else { 0 },
    )
    .unwrap();
}
fn started_fixture() -> (
    Fixture,
    crate::PeerAuthorityRequest,
    std::sync::Arc<crate::NativeMediaOrigin>,
) {
    let (f, request, _, origin) = parent_permissions_origin(true, true, true, false, true);
    for actor in 1..4 {
        let peer = activation::completed_peer(&f, actor);
        activation::record(&f, &peer);
    }
    let read = f.store.try_native_media_read(&origin, None).unwrap();
    f.store
        .record_native_connection_flow(&origin, &read, &observed(&origin))
        .unwrap();
    drop(read);
    // Controlled prior departures leave this established fixture with one
    // remaining live Connection for the last-Connection grace tests.
    f.store.connection().unwrap().execute("UPDATE connections SET state='retired',generation=generation+1 WHERE session_id='session' AND id!=?1",[origin.connection_id()]).unwrap();
    (f, request, origin)
}
fn pending_offer(f: &Fixture, request: &crate::PeerAuthorityRequest) -> String {
    let mut request = request.clone();
    request.command =
        crate::PeerCommand::NativeConnectionOffer(f.request(0, "grace-pending").intent);
    let proof = f.admit(&request);
    let record = f
        .store
        .prepare_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap();
    record.record_id
}
fn state(f: &Fixture, origin: &crate::NativeMediaOrigin) -> String {
    f.store
        .session_view(&f.keys[0], origin.conversation_id(), origin.session_id())
        .unwrap()
        .state
}
#[test]
fn reconnect_policy_defaults_to_thirty_seconds_and_freezes_at_invitation() {
    let (f, _, origin) = started_fixture();
    f.store.configure_session_reconnect_grace(1).unwrap();
    grace_loss(&f, &origin, false);
    let c = f.store.connection().unwrap();
    let (seconds,delta):(u32,i64)=c.query_row("SELECT grace_seconds,deadline_millis-lost_at_millis FROM session_reconnect_boundaries WHERE session_id=?1",[origin.session_id()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!((seconds, delta), (30, 30_000));
    let deadline: i64 = c
        .query_row(
            "SELECT deadline_millis FROM session_reconnect_boundaries WHERE session_id=?1",
            [origin.session_id()],
            |r| r.get(0),
        )
        .unwrap();
    crate::session_reconnect::observe_loss(
        &c,
        &f.store,
        origin.session_id(),
        chrono::Utc::now().timestamp_millis() + 10_000,
    )
    .unwrap();
    assert_eq!(
        c.query_row(
            "SELECT deadline_millis FROM session_reconnect_boundaries WHERE session_id=?1",
            [origin.session_id()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        deadline
    );
    assert!(c
        .execute(
            "UPDATE session_reconnect_boundaries SET grace_seconds=1",
            []
        )
        .is_err());
    drop(c);
    f.store
        .reconcile_session_reconnect(origin.conversation_id(), origin.session_id())
        .unwrap();
    assert_eq!(state(&f, &origin), "active");
    assert!(f.store.configure_session_reconnect_grace(301).is_err());
}
#[test]
fn reconnect_scan_is_read_only_and_connected_session_never_expires_from_silence() {
    let (f, _, origin) = started_fixture();
    for _ in 0..3 {
        f.store
            .reconcile_session_reconnect(origin.conversation_id(), origin.session_id())
            .unwrap();
        assert_eq!(state(&f, &origin), "active");
    }
    let store = &f.store;
    let held = store.connection().unwrap();
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    std::thread::scope(|scope| {
        scope.spawn(move || {
            send.send(store.session_reconnect_page("")).unwrap();
        });
        let page = receive.recv_timeout(Duration::from_secs(1));
        drop(held);
        assert!(page.unwrap().unwrap().1.is_empty());
    });
}
#[test]
fn reconnect_current_pending_offer_preserves_session_after_grace_until_permission_is_revoked() {
    let (f, request, origin) = started_fixture();
    pending_offer(&f, &request);
    grace_loss(&f, &origin, true);
    f.store
        .reconcile_session_reconnect(origin.conversation_id(), origin.session_id())
        .unwrap();
    assert_eq!(state(&f, &origin), "active");
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE peer_signing_keys SET revoked=1 WHERE key_id=?1",
            [&request.key_id],
        )
        .unwrap();
    f.store
        .reconcile_session_reconnect(origin.conversation_id(), origin.session_id())
        .unwrap();
    assert_eq!(state(&f, &origin), "ended");
    assert_eq!(f.count("session_terminal_facts"), 1);
}
#[test]
fn reconnect_revoked_credential_ceiling_endpoint_and_stale_runtime_cannot_extend_grace() {
    for change in 0..5 {
        let (mut f, request, origin) = started_fixture();
        pending_offer(&f, &request);
        grace_loss(&f, &origin, true);
        match change {
            0 => {
                f.store
                    .connection()
                    .unwrap()
                    .execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&f.keys[0]])
                    .unwrap();
            }
            1 => {
                f.store
                    .connection()
                    .unwrap()
                    .execute(
                        "UPDATE credentials SET recipients='[]' WHERE id=?1",
                        [&f.keys[0]],
                    )
                    .unwrap();
            }
            2 => {
                f.store
                    .connection()
                    .unwrap()
                    .execute(
                        "UPDATE endpoints SET verification_state='revoked' WHERE id=?1",
                        [&f.endpoints[0]],
                    )
                    .unwrap();
            }
            3 => {
                f.store
                    .connection()
                    .unwrap()
                    .execute(
                        "UPDATE credentials SET operations='[\"session_get\"]' WHERE id=?1",
                        [&f.keys[0]],
                    )
                    .unwrap();
            }
            _ => {
                f.store.runtime_epoch = "controlled-new-runtime".into();
            }
        }
        f.store
            .reconcile_session_reconnect(origin.conversation_id(), origin.session_id())
            .unwrap();
        assert_eq!(
            f.store
                .connection()
                .unwrap()
                .query_row(
                    "SELECT state FROM sessions WHERE id=?1",
                    [origin.session_id()],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "ended",
            "change {change}"
        );
    }
}
#[test]
fn reconnect_failed_pending_offer_ends_once_and_blocks_new_offers_after_deadline() {
    let (f, request, origin) = started_fixture();
    let pending = pending_offer(&f, &request);
    grace_loss(&f, &origin, true);
    let c = f.store.connection().unwrap();
    c.execute(
        "UPDATE operations SET state='failed' WHERE record_id=?1",
        [pending],
    )
    .unwrap();
    assert_eq!(
        crate::session_reconnect::permit_offer(&c, &f.store, origin.session_id())
            .unwrap_err()
            .code,
        "conflict"
    );
    drop(c);
    for _ in 0..2 {
        f.store
            .reconcile_session_reconnect(origin.conversation_id(), origin.session_id())
            .unwrap();
    }
    assert_eq!(state(&f, &origin), "ended");
    assert_eq!(f.count("session_terminal_facts"), 1);
    let page = f.store.session_lifecycle_hint_page(0).unwrap();
    assert_eq!(
        page.iter()
            .filter(|(_, _, data, _)| matches!(
                data,
                connect_protocol::events::JournalData::SessionEnded(_)
            ))
            .count(),
        1
    );
}
#[test]
fn reconnect_expiry_storage_failure_rolls_back_terminal_state_and_retry_keeps_one_fact() {
    let (f, _, origin) = started_fixture();
    grace_loss(&f, &origin, true);
    let c = f.store.connection().unwrap();
    c.execute_batch("CREATE TRIGGER reject_end BEFORE INSERT ON session_terminal_facts BEGIN SELECT RAISE(ABORT,'controlled failure'); END;").unwrap();
    drop(c);
    assert_eq!(
        f.store
            .reconcile_session_reconnect(origin.conversation_id(), origin.session_id())
            .unwrap_err()
            .code,
        "storage-unavailable"
    );
    assert_eq!(state(&f, &origin), "active");
    assert_eq!(f.count("session_terminal_facts"), 0);
    f.store
        .connection()
        .unwrap()
        .execute_batch("DROP TRIGGER reject_end")
        .unwrap();
    f.store
        .reconcile_session_reconnect(origin.conversation_id(), origin.session_id())
        .unwrap();
    assert_eq!(state(&f, &origin), "ended");
    assert_eq!(f.count("session_terminal_facts"), 1);
}
#[test]
fn terminal_native_hint_uses_original_fact_and_current_captured_epoch_and_ceiling() {
    let (f, mut request, origin) = started_fixture();
    grace_loss(&f, &origin, true);
    f.store
        .reconcile_session_reconnect(origin.conversation_id(), origin.session_id())
        .unwrap();
    request.command = crate::PeerCommand::SessionEndedRead {
        conversation_id: origin.conversation_id().into(),
        session_id: origin.session_id().into(),
    };
    let proof = f.admit(&request);
    let fact = f.store.session_ended_with_peer(&f.keys[0], &proof).unwrap();
    assert_eq!(fact.reason, "reconnect-grace-expired");
    assert_eq!(fact.reason_code, 408);
    assert!(fact.by_participant.is_none());
    drop(proof);
    let c = f.store.connection().unwrap();
    c.execute(
        "UPDATE sessions SET ended_reason='later mutable metadata' WHERE id=?1",
        [origin.session_id()],
    )
    .unwrap();
    drop(c);
    let proof = f.admit(&request);
    assert_eq!(
        f.store.session_ended_with_peer(&f.keys[0], &proof).unwrap(),
        fact
    );
    drop(proof);
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET recipients='[]' WHERE id=?1",
            [&f.keys[0]],
        )
        .unwrap();
    assert!(admit(&f, &request).is_err());
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE credentials SET recipients='[\"*\"]' WHERE id=?1",
            [&f.keys[0]],
        )
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE participants SET active=0 WHERE participant_id=?1",
            [origin.participant_id()],
        )
        .unwrap();
    f.store
        .connection()
        .unwrap()
        .execute(
            "UPDATE participants SET active=1,membership_epoch=membership_epoch+1 WHERE participant_id=?1",
            [origin.participant_id()],
        )
        .unwrap();
    assert!(admit(&f, &request).is_err());
}

#[test]
fn reconnect_connected_native_activation_preserves_started_session_without_requiring_new_packets() {
    let (f, request, origin) = started_fixture();
    let mut c = f.store.connection().unwrap();
    c.execute("UPDATE session_reconnect_boundaries SET lost_at_millis=?2,deadline_millis=?2+30000 WHERE session_id=?1",params![origin.session_id(),chrono::Utc::now().timestamp_millis()-31_000]).unwrap();
    drop(c);
    // The actual activation was independently committed by the sealed adapter.
    // Historical flow is not re-inferred from packet arrival or packet silence.
    f.store
        .reconcile_session_reconnect(origin.conversation_id(), origin.session_id())
        .unwrap();
    assert_eq!(state(&f, &origin), "active");
    assert!(f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT deadline_millis FROM session_reconnect_boundaries WHERE session_id=?1",
            [origin.session_id()],
            |r| r.get::<_, Option<i64>>(0)
        )
        .unwrap()
        .is_none());
    let _ = request;
}

#[test]
fn reconnect_expired_before_timer_reconciliation_refuses_preparation_without_a_dormant_slot() {
    let (f, mut request, origin) = started_fixture();
    grace_loss(&f, &origin, true);
    request.command =
        crate::PeerCommand::NativeConnectionOffer(f.request(0, "late-before-timer").intent);
    let proof = f.admit(&request);
    let before = (f.count("connections"), f.count("operation_effects"));
    let failed = f
        .store
        .prepare_native_offer_with_peer(&f.keys[0], &proof)
        .unwrap();
    assert_eq!(
        failed.state,
        connect_protocol::transitions::OperationState::Failed
    );
    assert_eq!(
        (f.count("connections"), f.count("operation_effects")),
        before
    );
    assert_eq!(
        state(&f, &origin),
        "active",
        "the timer deliberately has not run yet"
    );
    assert_eq!(
        f.store
            .prepare_native_offer_with_peer(&f.keys[0], &proof)
            .unwrap(),
        failed
    );
}
