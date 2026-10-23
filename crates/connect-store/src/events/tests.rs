#![allow(clippy::unwrap_used)]
use super::*;
use crate::Scope;
use connect_protocol::{membership::*, messages::*, Limits, Operation, Outcome};
struct Fixture {
    dir: tempfile::TempDir,
    store: Store,
    ids: Vec<String>,
    keys: Vec<String>,
    cid: String,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store =
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap();
        let mut ids = vec![];
        let mut keys = vec![];
        for alias in ["alice", "bob", "carol"] {
            let id = store.create_identity(alias).unwrap().id;
            keys.push(
                store
                    .issue(&id, Scope::operator(), crate::now() + 3600)
                    .unwrap()
                    .id,
            );
            ids.push(id);
        }
        let Outcome::Conversation(conv) = store
            .execute(
                &keys[0],
                &Operation::Create {
                    request_id: "create".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(
                        ids[1..].to_vec(),
                    ),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        Self {
            dir,
            store,
            ids,
            keys,
            cid: conv.id,
        }
    }
    fn snapshot(&self, n: usize, limit: u32) -> SnapshotPage {
        self.store
            .conversation_snapshot(
                &self.keys[n],
                &self.cid,
                SnapshotQuery {
                    page: None,
                    limit: Some(limit),
                },
            )
            .unwrap()
    }
    fn events(&self, n: usize, cursor: &str, limit: u32) -> EventPage {
        self.store
            .conversation_events(
                &self.keys[n],
                &self.cid,
                EventQuery {
                    after: Some(cursor.into()),
                    limit: Some(limit),
                    ..Default::default()
                },
            )
            .unwrap()
    }
    fn participant(&self, n: usize) -> String {
        self.store
            .membership_snapshot(&self.keys[0], &self.cid)
            .unwrap()
            .participants
            .into_iter()
            .find(|p| p.identity_id == self.ids[n])
            .unwrap()
            .participant_id
    }
    fn send(&self, id: &str, to: usize, body: &str) {
        self.store
            .execute(
                &self.keys[0],
                &Operation::Send {
                    request_id: id.into(),
                    conversation_id: self.cid.clone(),
                    message: MessageDraft::text(
                        MessageAudience::Participants(vec![self.participant(to)]),
                        body,
                    ),
                },
            )
            .unwrap();
    }
}
#[test]
fn snapshot_pages_freeze_one_revision_and_watermark_while_new_facts_catch_up() {
    let f = Fixture::new();
    let first = f.snapshot(1, 1);
    let mut next = first.next.clone();
    let mut items = first.items.clone();
    f.store
        .execute_membership(
            &f.keys[0],
            MembershipRequest {
                operation_id: "role".into(),
                conversation_id: f.cid.clone(),
                expected_revision: 0,
                change: MembershipAction::UpdateRole {
                    participant_id: f.participant(2),
                    role: "observer".into(),
                },
            },
        )
        .unwrap();
    while let Some(token) = next {
        let page = f
            .store
            .conversation_snapshot(
                &f.keys[1],
                &f.cid,
                SnapshotQuery {
                    page: Some(token),
                    limit: Some(1),
                },
            )
            .unwrap();
        assert_eq!(page.snapshot_id, first.snapshot_id);
        assert_eq!(page.revision, 0);
        assert_eq!(page.cursor, first.cursor);
        items.extend(page.items);
        next = page.next;
    }
    assert_eq!(items.len(), 4);
    assert!(items.iter().any(|i|matches!(i,SnapshotItem::Participant(m) if m.identity_id==f.ids[2] && m.role=="participant")));
    let events = f.events(1, &first.cursor, 1);
    assert_eq!(events.events.len(), 1);
    assert!(!events.has_more);
    assert!(
        matches!(&events.events[0].data,JournalData::Membership{role:Some(role),..}if role=="observer")
    );
    assert_eq!(events.events[0].revision, 1);
    assert!(events.events[0].observed_at > 0);
    assert_eq!(
        f.events(1, &first.cursor, 1).events[0].event_id,
        events.events[0].event_id
    );
    assert!(f.events(1, &events.cursor, 1).events.is_empty());
    assert_eq!(f.snapshot(1, 1).revision, 1);
}
#[test]
fn private_audiences_and_current_recipient_ceilings_are_filtered_before_page_limits() {
    let f = Fixture::new();
    let first = f.snapshot(1, 1);
    f.send("private", 2, "hidden");
    f.send("visible", 1, "for bob");
    f.send("last-private", 2, "also hidden");
    let page = f.events(1, &first.cursor, 1);
    assert_eq!(page.events.len(), 1);
    assert!(!page.has_more);
    assert!(matches!(&page.events[0].data,JournalData::Message(m)if m.body=="for bob"));
    let narrow = f
        .store
        .issue(
            &f.ids[1],
            Scope {
                operations: vec!["conversation.subscribe".into(), "history".into()],
                recipients: vec![f.ids[2].clone()],
            },
            crate::now() + 300,
        )
        .unwrap();
    let denied = f
        .store
        .conversation_events(
            &narrow.id,
            &f.cid,
            EventQuery {
                after: Some(first.cursor),
                limit: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(denied.events.is_empty());
    assert!(!denied.has_more);
    assert_ne!(page.cursor, denied.cursor); // AEAD nonces expose no sequence/count.
    assert!(page.cursor.starts_with("ev1_"));
}
#[test]
fn queued_snapshot_pages_recheck_revocation_membership_epochs_and_hidden_items() {
    let f = Fixture::new();
    let first = f.snapshot(1, 1);
    let carol = f.participant(2);
    f.store
        .execute_membership(
            &f.keys[0],
            MembershipRequest {
                operation_id: "remove".into(),
                conversation_id: f.cid.clone(),
                expected_revision: 0,
                change: MembershipAction::Remove {
                    participant_id: carol,
                    reason: None,
                },
            },
        )
        .unwrap();
    let page = f
        .store
        .conversation_snapshot(
            &f.keys[1],
            &f.cid,
            SnapshotQuery {
                page: first.next.clone(),
                limit: Some(100),
            },
        )
        .unwrap();
    assert!(!page
        .items
        .iter()
        .any(|i| matches!(i,SnapshotItem::Participant(m)if m.identity_id==f.ids[2])));
    assert!(!page.has_more);
    {
        let c = f.store.connection().unwrap();
        c.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&f.keys[1]])
            .unwrap();
    }
    assert_eq!(
        f.store
            .conversation_snapshot(
                &f.keys[1],
                &f.cid,
                SnapshotQuery {
                    page: first.next,
                    limit: Some(1)
                }
            )
            .unwrap_err()
            .code,
        "unauthorized"
    );
    let alice = f.snapshot(0, 1);
    {
        let c = f.store.connection().unwrap();
        c.execute(
            "UPDATE participants SET active=0 WHERE identity_id=?1 AND conversation_id=?2",
            params![f.ids[0], f.cid],
        )
        .unwrap();
        c.execute("UPDATE participants SET active=1,membership_epoch=membership_epoch+1 WHERE identity_id=?1 AND conversation_id=?2",params![f.ids[0],f.cid]).unwrap();
    }
    assert_eq!(
        f.store
            .conversation_snapshot(
                &f.keys[0],
                &f.cid,
                SnapshotQuery {
                    page: alice.next,
                    limit: Some(1)
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert_eq!(
        f.store
            .conversation_events(
                &f.keys[0],
                &f.cid,
                EventQuery {
                    after: Some(alice.cursor),
                    ..Default::default()
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn durable_retention_gaps_resync_but_filtered_gaps_and_duplicate_reads_do_not() {
    let f = Fixture::new();
    let first = f.snapshot(1, 1);
    f.send("message", 1, "hello");
    assert!(!f.events(1, &first.cursor, 1).resync_required);
    {
        let c = f.store.connection().unwrap();
        c.execute_batch("DROP TRIGGER journal_fact_immutable; UPDATE journal SET observed_at=1;")
            .unwrap();
    }
    assert!(
        f.store
            .prune_conversation_journal(crate::now() - 30 * 86400, 1000)
            .unwrap()
            > 0
    );
    let gap = f.events(1, &first.cursor, 1);
    assert!(gap.resync_required);
    assert!(gap.snapshot.is_some());
    assert!(gap.events.is_empty());
    assert!(!f.events(1, &gap.cursor, 1).resync_required);
    assert_eq!(
        f.store
            .prune_conversation_journal(crate::now(), 1)
            .unwrap_err()
            .code,
        "invalid-request"
    );
}
#[test]
fn eviction_and_restart_invalidate_bounded_snapshots_without_pinning_database_state() {
    let f = Fixture::new();
    let first = f.snapshot(0, 1);
    let _ = f.snapshot(0, 1);
    let _ = f.snapshot(0, 1);
    assert_eq!(f.store.events.snapshots.lock().unwrap().len(), 2);
    assert_eq!(
        f.store
            .conversation_snapshot(
                &f.keys[0],
                &f.cid,
                SnapshotQuery {
                    page: first.next,
                    limit: Some(1)
                }
            )
            .unwrap_err()
            .code,
        "resync-required"
    );
    let fresh = f.snapshot(0, 1);
    let reopened = Store::open(&f.dir.path().join("state.sqlite3"), Limits::default()).unwrap();
    assert_eq!(
        reopened
            .conversation_snapshot(
                &f.keys[0],
                &f.cid,
                SnapshotQuery {
                    page: fresh.next,
                    limit: Some(1)
                }
            )
            .unwrap_err()
            .code,
        "resync-required"
    );
    assert!(
        !reopened
            .conversation_events(
                &f.keys[0],
                &f.cid,
                EventQuery {
                    after: Some(fresh.cursor),
                    ..Default::default()
                }
            )
            .unwrap()
            .resync_required
    );
}
#[test]
fn subscription_lease_expires_explicitly_and_resuming_with_a_fresh_exchange_is_bounded() {
    let f = Fixture::new();
    let first = f.snapshot(1, 1);
    let page = f
        .store
        .conversation_events(
            &f.keys[1],
            &f.cid,
            EventQuery {
                after: Some(first.cursor),
                lease_seconds: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(page.expires_at.unwrap() <= crate::now() + 1);
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let resumed = f
        .store
        .conversation_events(
            &f.keys[1],
            &f.cid,
            EventQuery {
                after: Some(page.cursor),
                lease: page.lease,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(resumed.resync_required);
    assert!(resumed.lease.is_none());
    assert!(resumed.snapshot.is_some());
    let fresh = f
        .store
        .conversation_events(
            &f.keys[1],
            &f.cid,
            EventQuery {
                after: Some(resumed.cursor),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(!fresh.resync_required);
    assert!(fresh.lease.is_some());
}
#[test]
fn encrypted_cursors_reject_cross_identity_conversation_purpose_and_tampering() {
    let f = Fixture::new();
    let first = f.snapshot(0, 1);
    assert_eq!(
        f.store
            .conversation_events(
                &f.keys[1],
                &f.cid,
                EventQuery {
                    after: Some(first.cursor.clone()),
                    ..Default::default()
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    let mut corrupt = first.cursor.clone().into_bytes();
    corrupt[12] = if corrupt[12] == b'a' { b'b' } else { b'a' };
    assert_eq!(
        f.events_error(&String::from_utf8(corrupt).unwrap()),
        "invalid-request"
    );
    assert_eq!(
        f.events_error(first.next.as_deref().unwrap()),
        "invalid-request"
    );
}
impl Fixture {
    fn events_error(&self, token: &str) -> String {
        self.store
            .conversation_events(
                &self.keys[0],
                &self.cid,
                EventQuery {
                    after: Some(token.into()),
                    ..Default::default()
                },
            )
            .unwrap_err()
            .code
    }
}

impl Fixture {
    fn live_source(&self) -> (String, String) {
        let Outcome::Session(session) = self
            .store
            .execute(
                &self.keys[0],
                &Operation::SessionInvite {
                    request_id: "invite".into(),
                    conversation_id: self.cid.clone(),
                    participant_id: self.ids[1].clone(),
                    media: vec!["audio".into()],
                },
            )
            .unwrap()
        else {
            panic!()
        };
        self.store
            .execute(
                &self.keys[1],
                &Operation::SessionAccept {
                    request_id: "accept".into(),
                    conversation_id: self.cid.clone(),
                    session_id: session.id.clone(),
                },
            )
            .unwrap();
        let alice = self.participant(0);
        let bob = self.participant(1);
        let c = self.store.connection().unwrap();
        for pid in [&alice, &bob] {
            c.execute("INSERT INTO session_participants(session_id,conversation_id,tenant_id,participant_id,membership_epoch,role,state) VALUES(?1,?2,'tnt_default',?3,1,'participant','present') ON CONFLICT(session_id,participant_id,membership_epoch) DO UPDATE SET role=excluded.role,state=excluded.state",params![session.id,self.cid,pid]).unwrap();
        }
        // Trusted adapter rows, not a claim of physical media readiness.
        c.execute("INSERT INTO devices(id,identity_id,tenant_id,kind,platform) VALUES('source-device',?1,'tnt_default','fixture','fixture')",[&self.ids[0]]).unwrap();
        c.execute("INSERT INTO endpoints(id,identity_id,tenant_id,device_id,transport,address,capabilities_json,verification_state,verification_evidence,verified_at) VALUES('source-endpoint',?1,'tnt_default','source-device','fixture','fixture://source','[\"audio\"]','verified','fixture',?2)",params![self.ids[0],crate::now()]).unwrap();
        c.execute("INSERT INTO connections(id,session_id,participant_id,membership_epoch,identity_id,tenant_id,endpoint_id,generation,state,observed_runtime_id) VALUES('source-connection',?1,?2,1,?3,'tnt_default','source-endpoint',1,'active',?4)",params![session.id,alice,self.ids[0],self.store.runtime_epoch()]).unwrap();
        (session.id, "source-connection".into())
    }
    fn source(&self, connection: &str) -> crate::SessionObservationSource {
        self.store
            .bind_session_observation_source(
                &self.keys[0],
                connection,
                "fixture",
                "provider-call",
                &[self.participant(0)],
                ObservationCapabilities {
                    transcript: true,
                    speech: false,
                    actions: vec!["request-assistance".into()],
                },
            )
            .unwrap()
    }
    fn transcript(text: &str) -> ObservationDraft {
        ObservationDraft::Transcript {
            text: text.into(),
            completeness: TranscriptCompleteness::Interim,
            turn_id: Some("turn-one".into()),
            fragment_id: Some("fragment-one".into()),
        }
    }
    fn observations(&self, n: usize, sid: &str, after: Option<String>) -> ObservationPage {
        self.store
            .session_observations(
                &self.keys[n],
                &self.cid,
                sid,
                ObservationQuery {
                    after,
                    limit: Some(100),
                    ..Default::default()
                },
            )
            .unwrap()
    }
}
#[test]
fn transient_sources_bind_scope_provenance_private_audience_and_declared_capabilities() {
    let f = Fixture::new();
    let (sid, connection) = f.live_source();
    let source = f.source(&connection);
    let data = Fixture::transcript("private words");
    assert!(!format!("{data:?}").contains("private words"));
    let id = f
        .store
        .publish_session_observation(&source, "fragment", data.clone())
        .unwrap();
    let same_source = f.source(&connection);
    assert_eq!(
        id,
        f.store
            .publish_session_observation(&same_source, "fragment", data)
            .unwrap()
    );
    assert_eq!(
        f.store
            .publish_session_observation(&source, "fragment", Fixture::transcript("changed"))
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(
        f.store
            .publish_session_observation(
                &source,
                "speech",
                ObservationDraft::Speech {
                    state: SpeechState::Started,
                    turn_id: None
                }
            )
            .unwrap_err()
            .code,
        "unsupported-capability"
    );
    let action = ObservationDraft::ActionRequest {
        action: "request-assistance".into(),
        provider_tool_call_id: "tool-one".into(),
        arguments: serde_json::json!({"untrusted":"proposal"}),
        turn_id: None,
    };
    let before: u32 = f
        .store
        .connection()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM operations", [], |r| r.get(0))
        .unwrap();
    f.store
        .publish_session_observation(&source, "action", action)
        .unwrap();
    let alice = f.observations(0, &sid, None);
    assert_eq!(alice.observations.len(), 2);
    assert_eq!(alice.routes.len(), 1);
    assert!(!alice.routes[0].capabilities.speech);
    assert_eq!(alice.observations[0].connection_id, connection);
    assert_eq!(alice.observations[0].generation, 1);
    assert_eq!(alice.observations[0].source.correlation_id, "provider-call");
    let bob = f.observations(1, &sid, None);
    assert!(bob.observations.is_empty());
    assert!(bob.routes.is_empty());
    let after: u32 = f
        .store
        .connection()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM operations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        before, after,
        "an action proposal cannot execute an operation"
    );
    assert!(serde_json::from_value::<ObservationDraft>(serde_json::json!({"kind":"speech","state":"started","turn_id":null,"connection_id":"forged"})).is_err());
}
#[test]
fn slow_observation_consumers_resync_with_bounded_memory_without_private_gap_counts() {
    let f = Fixture::new();
    let (sid, connection) = f.live_source();
    let source = f.source(&connection);
    let alice = f.observations(0, &sid, None);
    let bob = f.observations(1, &sid, None);
    for n in 0..300 {
        f.store
            .publish_session_observation(
                &source,
                &format!("fragment-{n}"),
                Fixture::transcript(&format!("words-{n}")),
            )
            .unwrap();
    }
    {
        let buffer = f.store.events.observations.lock().unwrap();
        let (references, bytes) = buffer.occupancy();
        assert_eq!(references, 256);
        assert!(bytes <= 2 * 1024 * 1024);
    }
    let page = f.observations(0, &sid, Some(alice.cursor));
    assert!(page.resync_required);
    assert_eq!(page.observations.len(), 100);
    assert!(page.has_more);
    let second = f.observations(0, &sid, Some(page.cursor));
    assert!(!second.resync_required);
    assert_eq!(second.observations.len(), 100);
    assert!(second.has_more);
    let third = f.observations(0, &sid, Some(second.cursor));
    assert_eq!(third.observations.len(), 56);
    assert!(!third.has_more);
    let hidden = f.observations(1, &sid, Some(bob.cursor));
    assert!(!hidden.resync_required);
    assert!(hidden.observations.is_empty());
    assert!(!hidden.has_more);
}
#[test]
fn queued_observations_and_old_publishers_fail_after_connection_generation_or_credential_changes() {
    let f = Fixture::new();
    let (sid, connection) = f.live_source();
    let source = f.source(&connection);
    f.store
        .publish_session_observation(&source, "before", Fixture::transcript("before replacement"))
        .unwrap();
    {
        let c = f.store.connection().unwrap();
        c.execute(
            "UPDATE connections SET generation=generation+1 WHERE id=?1",
            [&connection],
        )
        .unwrap();
    }
    assert_eq!(
        f.store
            .publish_session_observation(&source, "late", Fixture::transcript("stale callback"))
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert!(f.observations(0, &sid, None).observations.is_empty());
    let replacement = f.source(&connection);
    f.store
        .publish_session_observation(&replacement, "after", Fixture::transcript("fresh"))
        .unwrap();
    let fresh = f.observations(0, &sid, None);
    assert_eq!(fresh.observations.len(), 1);
    assert_eq!(fresh.observations[0].generation, 2);
    {
        let c = f.store.connection().unwrap();
        c.execute("UPDATE credentials SET revoked=1 WHERE id=?1", [&f.keys[0]])
            .unwrap();
    }
    assert_eq!(
        f.store
            .publish_session_observation(&replacement, "revoked", Fixture::transcript("no"))
            .unwrap_err()
            .code,
        "forbidden"
    );
    assert!(f.observations(1, &sid, None).observations.is_empty());
}

#[test]
fn operation_progress_preserves_original_fact_state_and_is_not_a_public_room_event() {
    use connect_protocol::transitions::*;
    let f = Fixture::new();
    let (sid, connection) = f.live_source();
    let alice = f.snapshot(0, 100);
    let bob = f.snapshot(1, 100);
    let request = TransitionRequest {
        payload_extensions: Default::default(),
        operation_id: "consult".into(),
        conversation_id: f.cid.clone(),
        session_id: sid,
        connection_id: None,
        expected_revision: 0,
        expires_at: crate::now() + 300,
        intent: TransitionIntent::Consult {
            retained_connid: connection,
            // This is an owner-only progress fixture, not a media switch. The
            // retained Alice route is excluded from the two-party private set.
            private_participants: vec![f.participant(1), f.participant(2)],
            hold_treatment: HoldTreatment::Silence,
        },
    };
    f.store.admit_transition(&f.keys[0], request).unwrap();
    let (lease, _) = f
        .store
        .claim_transition(&f.keys[0], "consult", 60)
        .unwrap()
        .unwrap();
    f.store
        .advance_transition(&f.keys[0], &lease, OperationState::Ready)
        .unwrap();
    let page = f.events(0, &alice.cursor, 100);
    let states = page
        .events
        .iter()
        .filter_map(|event| match &event.data {
            JournalData::Operation { state, .. } => Some(*state),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        states,
        vec![
            OperationState::Accepted,
            OperationState::Preparing,
            OperationState::Ready
        ]
    );
    assert!(f.events(1, &bob.cursor, 100).events.is_empty());
    let snapshot = f.snapshot(0, 100);
    assert!(snapshot.items.iter().any(|item|matches!(item,SnapshotItem::Operation(record)if record.operation_id=="consult"&&record.state==OperationState::Ready)));
    assert!(snapshot
        .items
        .iter()
        .any(|item| matches!(item, SnapshotItem::SessionParticipant(_))));
    assert_eq!(alice.cursor.len(), page.cursor.len());
}

#[test]
fn original_resource_versions_provenance_and_owner_correlation_survive_later_changes() {
    let f = Fixture::new();
    let first = f.snapshot(0, 100);
    let bob = f.snapshot(1, 100);
    let (sid, _) = f.live_source();
    f.store
        .execute(
            &f.keys[0],
            &Operation::SessionEnd {
                request_id: "end".into(),
                reason: None,
                conversation_id: f.cid.clone(),
                session_id: sid.clone(),
            },
        )
        .unwrap();
    let page = f.events(0, &first.cursor, 100);
    let sessions = page
        .events
        .iter()
        .filter(|event| {
            matches!(
                event.data,
                JournalData::Session(_) | JournalData::SessionEnded(_)
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        sessions
            .iter()
            .map(|event| event.revision)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert_eq!(sessions[0].operation_id.as_deref(), Some("invite"));
    assert_eq!(
        sessions[1].operation_id, None,
        "another caller's logical ID is not public room metadata"
    );
    assert_eq!(sessions[2].operation_id.as_deref(), Some("end"));
    assert!(sessions
        .iter()
        .all(|event| event.source.correlation_id == sid && event.source.kind == "connect-journal"));
    let other = f.events(1, &bob.cursor, 100);
    assert_eq!(
        other
            .events
            .iter()
            .find(|event| event.kind == sessions[1].kind)
            .unwrap()
            .operation_id
            .as_deref(),
        Some("accept")
    );
    let snapshot = f.snapshot(0, 100);
    assert!(snapshot.items.iter().any(|item|matches!(item,SnapshotItem::Session(value)if value.revision==2&&value.topology_revision==0)));
    let c = f.store.connection().unwrap();
    let context: u32 = c
        .query_row("SELECT COUNT(*) FROM journal_context", [], |r| r.get(0))
        .unwrap();
    assert_eq!(context, 0, "no caller context survives commit");
    for sql in [
        "UPDATE journal SET resource_revision=99 WHERE id=?1",
        "UPDATE journal SET observed_at=1 WHERE id=?1",
        "UPDATE journal SET source_json='{}' WHERE id=?1",
        "UPDATE journal SET session_json=json_set(session_json,'$.state','forged') WHERE id=?1",
        "UPDATE journal SET recipient_epoch=recipient_epoch+1 WHERE id=?1",
    ] {
        assert!(c.execute(sql, [&sessions[0].event_id]).is_err(), "{sql}");
    }
}
#[test]
fn conversation_close_records_its_real_revision_and_failed_mutations_leave_no_context() {
    let f = Fixture::new();
    let first = f.snapshot(0, 100);
    let closed = f
        .store
        .execute(
            &f.keys[0],
            &Operation::Close {
                request_id: "close".into(),
                conversation_id: f.cid.clone(),
                options: Default::default(),
            },
        )
        .unwrap();
    assert!(matches!(closed,Outcome::Conversation(c)if c.closed));
    let page = f.events(0, &first.cursor, 100);
    assert_eq!(page.events.len(), 1);
    assert_eq!(page.events[0].revision, 1);
    assert_eq!(page.events[0].operation_id.as_deref(), Some("close"));
    assert_eq!(f.snapshot(0, 100).revision, 1);
    assert!(f
        .store
        .execute(
            &f.keys[0],
            &Operation::Send {
                request_id: "after-close".into(),
                conversation_id: f.cid.clone(),
                message: MessageDraft::text_all("denied")
            }
        )
        .is_err());
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM journal_context", [], |r| r
                .get::<_, u32>(0))
            .unwrap(),
        0
    );
}

#[test]
fn artifact_and_receipt_events_use_original_resource_versions_and_real_provenance() {
    use connect_protocol::receipts::ClientReceiptState;
    let f = Fixture::new();
    let start = f.snapshot(1, 100);
    let Outcome::Artifact(file) = f
        .store
        .execute(
            &f.keys[0],
            &Operation::ArtifactBegin {
                request_id: "file".into(),
                conversation_id: f.cid.clone(),
                to: MessageAudience::all(),
                name: "test.txt".into(),
                media_type: "text/plain".into(),
                size: 3,
                sha256: hex::encode(Sha256::digest(b"abc")),
                expires_at: None,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    f.store
        .execute(
            &f.keys[0],
            &Operation::ArtifactAppend {
                request_id: "chunk".into(),
                conversation_id: f.cid.clone(),
                artifact_id: file.id.clone(),
                offset: 0,
                data: "YWJj".into(),
            },
        )
        .unwrap();
    f.store
        .execute(
            &f.keys[0],
            &Operation::ArtifactFinish {
                request_id: "finish".into(),
                conversation_id: f.cid.clone(),
                artifact_id: file.id.clone(),
            },
        )
        .unwrap();
    f.send("text", 1, "receipt source");
    let message: String = f
        .store
        .connection()
        .unwrap()
        .query_row(
            "SELECT id FROM messages WHERE body='receipt source'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    f.store
        .execute(
            &f.keys[1],
            &Operation::MessageReceipt {
                request_id: "delivered".into(),
                conversation_id: f.cid.clone(),
                message_id: message.clone(),
                state: ClientReceiptState::Delivered,
            },
        )
        .unwrap();
    f.store
        .execute(
            &f.keys[1],
            &Operation::ReceiptPreferencesSet {
                request_id: "opt-in".into(),
                conversation_id: f.cid.clone(),
                delivery: true,
                read: true,
            },
        )
        .unwrap();
    f.store
        .execute(
            &f.keys[1],
            &Operation::MessageReceipt {
                request_id: "read".into(),
                conversation_id: f.cid.clone(),
                message_id: message,
                state: ClientReceiptState::Read,
            },
        )
        .unwrap();
    let page = f.events(1, &start.cursor, 100);
    assert_eq!(
        page.events
            .iter()
            .filter(|event| matches!(&event.data,JournalData::Artifact(a)if a.id==file.id))
            .map(|event| event.revision)
            .collect::<Vec<_>>(),
        vec![0, 2]
    );
    let receipts = page
        .events
        .iter()
        .filter(|event| matches!(event.data, JournalData::Receipt(_)))
        .collect::<Vec<_>>();
    assert_eq!(
        receipts
            .iter()
            .map(|event| event.revision)
            .collect::<Vec<_>>(),
        vec![2, 3]
    );
    assert!(receipts
        .iter()
        .all(|event| event.source.kind == "client_reported"
            && event.source.correlation_id == f.participant(1)));
}
#[test]
fn encoded_event_pages_resume_every_escaped_message_without_exceeding_the_byte_budget() {
    let f = Fixture::new();
    let start = f.snapshot(1, 1);
    for n in 0..20 {
        f.send(&format!("large-{n}"), 1, &"\n".repeat(16000));
    }
    let mut cursor = start.cursor;
    let mut ids = std::collections::HashSet::new();
    let mut pages = 0;
    loop {
        let page = f.events(1, &cursor, 100);
        assert!(serde_json::to_vec(&page).unwrap().len() <= connect_protocol::MAX_PAGE_BYTES);
        assert!(!page.events.is_empty());
        for event in &page.events {
            assert!(ids.insert(event.event_id.clone()));
        }
        pages += 1;
        cursor = page.cursor;
        if !page.has_more {
            break;
        }
    }
    assert!(pages > 1);
    assert_eq!(ids.len(), 20);
    assert!(f.events(1, &cursor, 100).events.is_empty());
}
#[test]
fn backup_preserves_retention_floors_and_cursors_newer_than_restored_state_require_resync() {
    let f = Fixture::new();
    let start = f.snapshot(1, 1);
    let backup = f.dir.path().join("before.sqlite3");
    f.store.backup(&backup).unwrap();
    f.send("later", 1, "not in backup");
    let future = f.events(1, &start.cursor, 1).cursor;
    let restored = Store::open(&backup, Limits::default()).unwrap();
    let lost = restored
        .conversation_events(
            &f.keys[1],
            &f.cid,
            EventQuery {
                after: Some(future),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(lost.resync_required);
    assert!(lost.snapshot.is_some());
    {
        let c = f.store.connection().unwrap();
        c.execute("INSERT INTO journal(id,recipient,conversation_id,kind,observed_at) VALUES('historical-fixture',?1,?2,'conversation_created',1)",params![f.ids[1],f.cid]).unwrap();
    }
    assert_eq!(
        f.store
            .prune_conversation_journal(crate::now() - 30 * 86400, 1000)
            .unwrap(),
        1
    );
    let after = f.dir.path().join("after.sqlite3");
    let manifest = f.store.backup(&after).unwrap();
    assert_eq!(Store::inspect_snapshot(&after).unwrap(), manifest);
    let restored = Store::open(&after, Limits::default()).unwrap();
    assert!(
        restored
            .conversation_events(
                &f.keys[1],
                &f.cid,
                EventQuery {
                    after: Some(start.cursor),
                    ..Default::default()
                }
            )
            .unwrap()
            .resync_required
    );
}
#[test]
fn identity_churn_cannot_pin_more_than_the_global_snapshot_slot_budget() {
    let f = Fixture::new();
    let oldest = f.snapshot(0, 1);
    for n in 0..35 {
        let identity = f.store.create_identity(&format!("reader-{n}")).unwrap().id;
        let key = f
            .store
            .issue(&identity, crate::Scope::operator(), crate::now() + 300)
            .unwrap()
            .id;
        let Outcome::Conversation(room) = f
            .store
            .execute(
                &key,
                &Operation::Create {
                    request_id: "create".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![f
                        .ids[1]
                        .clone()]),
                },
            )
            .unwrap()
        else {
            panic!()
        };
        f.store
            .conversation_snapshot(
                &key,
                &room.id,
                SnapshotQuery {
                    limit: Some(1),
                    ..Default::default()
                },
            )
            .unwrap();
    }
    assert_eq!(f.store.events.snapshots.lock().unwrap().len(), 32);
    assert_eq!(
        f.store
            .conversation_snapshot(
                &f.keys[0],
                &f.cid,
                SnapshotQuery {
                    page: oldest.next,
                    limit: Some(1)
                }
            )
            .unwrap_err()
            .code,
        "resync-required"
    );
}

#[test]
fn speech_observations_are_explicit_and_membership_readmission_cannot_reopen_a_frozen_route() {
    let f = Fixture::new();
    let (sid, connection) = f.live_source();
    let alice = f.participant(0);
    let bob = f.participant(1);
    let source = f
        .store
        .bind_session_observation_source(
            &f.keys[0],
            &connection,
            "fixture",
            "speech-route",
            &[alice, bob.clone()],
            ObservationCapabilities {
                transcript: false,
                speech: true,
                actions: vec![],
            },
        )
        .unwrap();
    for (id, state) in [
        ("start", SpeechState::Started),
        ("stop", SpeechState::Stopped),
    ] {
        f.store
            .publish_session_observation(
                &source,
                id,
                ObservationDraft::Speech {
                    state,
                    turn_id: Some("turn-one".into()),
                },
            )
            .unwrap();
    }
    let before = f.observations(1, &sid, None);
    assert_eq!(before.observations.len(), 2);
    assert!(before.routes[0].capabilities.speech);
    assert!(matches!(
        before.observations[1].data,
        ObservationDraft::Speech {
            state: SpeechState::Stopped,
            ..
        }
    ));
    {
        let c = f.store.connection().unwrap();
        c.execute(
            "UPDATE participants SET active=0 WHERE participant_id=?1",
            [&bob],
        )
        .unwrap();
        c.execute("UPDATE participants SET active=1,membership_epoch=membership_epoch+1 WHERE participant_id=?1",[&bob]).unwrap();
        // Trusted fixture for a separately authorized new session admission. Its
        // new epoch still cannot inherit the previous private observation route.
        c.execute(
            "INSERT INTO resource_audiences VALUES('session',?1,?2,?3,2)",
            params![sid, f.cid, bob],
        )
        .unwrap();
    }
    assert_eq!(
        f.store
            .session_observations(
                &f.keys[1],
                &f.cid,
                &sid,
                ObservationQuery {
                    after: Some(before.cursor),
                    ..Default::default()
                }
            )
            .unwrap_err()
            .code,
        "forbidden"
    );
    let after = f.observations(1, &sid, None);
    assert!(after.observations.is_empty());
    assert!(after.routes.is_empty());
    f.store
        .publish_session_observation(
            &source,
            "later",
            ObservationDraft::Speech {
                state: SpeechState::Started,
                turn_id: Some("turn-two".into()),
            },
        )
        .unwrap();
    assert!(f.observations(1, &sid, None).observations.is_empty());
}

fn agent_data(sequence: u64, content: AgentObservationContent) -> AgentObservation {
    AgentObservation {
        local_sequence: sequence,
        capture_complete: false,
        missing_before: None,
        provider_session_id: Some("provider reference: data only".into()),
        provider_event_id: Some("repeated provider event".into()),
        captured_at: crate::now(),
        content,
    }
}
#[test]
fn agent_wire_preserves_large_partial_role_and_repeated_provider_ids_without_operations() {
    let f = Fixture::new();
    let (sid, connection) = f.live_source();
    let source = f.source(&connection);
    let before: u64 = f
        .store
        .connection()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM operations", [], |r| r.get(0))
        .unwrap();
    for sequence in 1..=2 {
        let observation = agent_data(
            sequence,
            AgentObservationContent::Transcript {
                text: Some("\"".repeat(12_000)),
                speaker_role: Some(AgentSpeakerRole::User),
                completeness: AgentCompleteness::Partial,
            },
        );
        observation.validate().unwrap();
        let data = ObservationDraft::Agent { observation };
        assert!(serde_json::to_vec(&data).unwrap().len() > 16 * 1024);
        assert!(!format!("{data:?}").contains("provider reference"));
        f.store
            .publish_session_observation(&source, &format!("frame-{sequence}"), data)
            .unwrap();
    }
    let page = f.observations(0, &sid, None);
    assert_eq!(page.observations.len(), 2);
    assert_ne!(
        page.observations[0].observation_id,
        page.observations[1].observation_id
    );
    for entry in &page.observations {
        assert_eq!(entry.participant_id, f.participant(0));
        let ObservationDraft::Agent { observation } = &entry.data else {
            panic!()
        };
        assert_eq!(
            observation.provider_event_id.as_deref(),
            Some("repeated provider event")
        );
        assert!(!observation.capture_complete);
    }
    assert!(f.observations(1, &sid, None).observations.is_empty());
    assert_eq!(
        before,
        f.store
            .connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM operations", [], |r| r
                .get::<_, u64>(0))
            .unwrap()
    );
}
#[test]
fn agent_capabilities_and_reported_results_do_not_execute_or_confirm_effects() {
    let f = Fixture::new();
    let (sid, connection) = f.live_source();
    let source = f.source(&connection);
    for content in [
        AgentObservationContent::Speech {
            state: Some("started".into()),
            speaker_role: None,
            turn: None,
            completeness: AgentCompleteness::Final,
        },
        AgentObservationContent::Action {
            name: Some("not-declared".into()),
            call_id: None,
            phase: AgentActionPhase::Requested,
            arguments: Some(serde_json::json!({"task_id":"forged"})),
            result: None,
            completeness: AgentCompleteness::Final,
        },
    ] {
        assert_eq!(
            f.store
                .publish_session_observation(
                    &source,
                    "unsupported",
                    ObservationDraft::Agent {
                        observation: agent_data(1, content)
                    }
                )
                .unwrap_err()
                .code,
            "unsupported-capability"
        );
    }
    let body = agent_data(
        2,
        AgentObservationContent::Action {
            name: None,
            call_id: Some("call data".into()),
            phase: AgentActionPhase::ResultObserved,
            arguments: None,
            result: Some(serde_json::json!({"completed":true,"task_id":"forged"})),
            completeness: AgentCompleteness::Final,
        },
    );
    f.store
        .publish_session_observation(
            &source,
            "result-data",
            ObservationDraft::Agent { observation: body },
        )
        .unwrap();
    assert_eq!(f.observations(0, &sid, None).observations.len(), 1);
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM operation_effects", [], |r| r
                .get::<_, u64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn agent_metadata_rejects_forged_bindings_incomplete_gaps_and_oversized_encoded_content() {
    let valid = agent_data(
        4,
        AgentObservationContent::Transcript {
            text: None,
            speaker_role: None,
            completeness: AgentCompleteness::Unavailable,
        },
    );
    valid.validate().unwrap();
    for bad in [0, MAX_AGENT_SEQUENCE + 1] {
        let mut o = valid.clone();
        o.local_sequence = bad;
        assert!(o.validate().is_err());
    }
    let mut gap = valid.clone();
    gap.missing_before = Some(AgentObservationGap {
        first_sequence: 1,
        last_sequence: 3,
        observations: 3,
    });
    gap.validate().unwrap();
    gap.missing_before.as_mut().unwrap().observations = 2;
    assert!(gap.validate().is_err());
    let mut complete = valid.clone();
    complete.capture_complete = true;
    assert!(complete.validate().is_err());
    for reference in ["é".repeat(257), "\u{0085}".into(), " ".into()] {
        let mut o = valid.clone();
        o.provider_event_id = Some(reference);
        assert!(o.validate().is_err());
    }
    let mut large = valid.clone();
    large.content = AgentObservationContent::Transcript {
        text: Some("\"".repeat(34_000)),
        speaker_role: None,
        completeness: AgentCompleteness::Partial,
    };
    assert_eq!(large.validate().unwrap_err().code, "capacity-exceeded");
    let mut forged = serde_json::to_value(valid).unwrap();
    forged["participant_id"] = serde_json::json!("forged-human");
    assert!(serde_json::from_value::<AgentObservation>(forged).is_err());
}

#[test]
fn committed_conversation_closure_is_original_immutable_and_scoped_for_each_recipient() {
    let f = Fixture::new();
    let cursors = (0..3)
        .map(|n| f.snapshot(n, 100).cursor)
        .collect::<Vec<_>>();
    let Outcome::Conversation(closed) = f
        .store
        .execute(
            &f.keys[0],
            &Operation::Close {
                request_id: "original-close".into(),
                conversation_id: f.cid.clone(),
                options: connect_protocol::conversations::CloseOptions {
                    reason_code: Some(200),
                    reason: Some("explicit-close".into()),
                    ..Default::default()
                },
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let mut facts = Vec::new();
    for (n, cursor) in cursors.iter().enumerate() {
        let page = f.events(n, cursor, 100);
        let events = page
            .events
            .iter()
            .filter(|e| e.kind == "conversation_closed")
            .collect::<Vec<_>>();
        assert_eq!(events.len(), 1);
        let JournalData::ConversationClosed(fact) = &events[0].data else {
            panic!()
        };
        assert_eq!(fact.conversation_id, f.cid);
        assert_eq!(fact.reason_code, 200);
        assert_eq!(fact.reason, "explicit-close");
        assert_eq!(Some(fact.closed_at), closed.closed_at);
        assert!(fact.closed_at <= events[0].observed_at);
        facts.push(fact.clone());
    }
    assert!(facts.windows(2).all(|p| p[0] == p[1]));
    let c = f.store.connection().unwrap();
    assert!(c
        .execute(
            "UPDATE conversation_closure_facts SET reason='changed' WHERE conversation_id=?1",
            [&f.cid]
        )
        .is_err());
    assert!(c
        .execute(
            "DELETE FROM conversation_closure_facts WHERE conversation_id=?1",
            [&f.cid]
        )
        .is_err());
    assert!(c
        .execute(
            "UPDATE conversations SET reason='changed' WHERE id=?1",
            [&f.cid]
        )
        .is_err());
    assert!(c
        .execute("UPDATE conversations SET closed=0 WHERE id=?1", [&f.cid])
        .is_err());
    c.execute(
        "UPDATE credentials SET operations='[\"conversation.subscribe\"]' WHERE id=?1",
        [&f.keys[1]],
    )
    .unwrap();
    drop(c);
    match f.store.conversation_events(
        &f.keys[1],
        &f.cid,
        EventQuery {
            after: Some(cursors[1].clone()),
            limit: Some(100),
            ..Default::default()
        },
    ) {
        Ok(page) => assert!(!page.events.iter().any(|e| e.kind == "conversation_closed")),
        Err(e) => assert_eq!(e.code, "forbidden"),
    }
}
