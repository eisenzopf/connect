#![allow(clippy::unwrap_used)]
use super::*;
use connect_protocol::Limits;
use serde_json::json;
struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
    owner: String,
    credential: String,
    uuid: String,
}
impl Fixture {
    fn new(private: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store =
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap();
        let owner = store.create_identity("owner").unwrap().id;
        let peer = store.create_identity("peer").unwrap().id;
        let credential = store
            .issue(&owner, crate::Scope::operator(), crate::now() + 3600)
            .unwrap()
            .id;
        let c = store.connection().unwrap();
        c.execute("INSERT INTO conversations(id) VALUES('conv_record')", [])
            .unwrap();
        c.execute("INSERT INTO participants(conversation_id,identity_id,participant_id,role) VALUES('conv_record',?1,'part_a','participant'),('conv_record',?2,'part_b','participant')",params![owner,peer]).unwrap();
        c.execute("INSERT INTO sessions(id,conversation_id,initiator_id,participant_id,state,media,created_at,updated_at) VALUES('sess_record','conv_record',?1,?2,'invited','[]',100,100)",params![owner,peer]).unwrap();
        c.execute(
            "UPDATE sessions SET state='cancelled' WHERE id='sess_record'",
            [],
        )
        .unwrap();
        drop(c);
        let uuid = store.pending_record_intents(1).unwrap().pop().unwrap();
        store.finalize_record_intent(&uuid,|intent| {
   let mut rules=vec![json!({"id":"document","metadata_retention":{"kind":"until-deleted"}}),json!({"id":"session.outcome"}),json!({"id":"session.capture"}),json!({"id":"appearance.0"}),json!({"id":"appearance.1"})];
   let mut epochs=vec![RecordSourceEpoch{source_group:"session".into(),participant_id:"part_a".into(),membership_epoch:1},RecordSourceEpoch{source_group:"session".into(),participant_id:"part_b".into(),membership_epoch:1}];
   if private {rules.push(json!({"id":"connection.0"}));epochs.push(RecordSourceEpoch{source_group:"connection.0".into(),participant_id:"part_b".into(),membership_epoch:1});}
   Ok((RecordBytes{uuid:intent.uuid.clone(),body:serde_json::to_vec(&json!({"uuid":intent.uuid,"parties":[{"connect_participant_id":"part_a"},{"connect_participant_id":"part_b"}],"analysis":[{"body":{"state":"cancelled"}},{"body":{"appearance_omissions":0,"connection_omissions":0,"audience_omissions":0}}]})).unwrap(),manifest:serde_json::to_vec(&json!({"policy":"connect-session-metadata-v1","sources":rules})).unwrap()},epochs))
  }).unwrap();
        Self {
            _dir: dir,
            store,
            owner,
            credential,
            uuid,
        }
    }
    fn query(&self) -> RecordFetchIntent {
        RecordFetchIntent {
            conversation_id: "conv_record".into(),
            session_id: "sess_record".into(),
            uuid: Some(self.uuid.clone()),
            url: None,
        }
    }
    fn sources(&self, credential: &str) -> Result<BTreeSet<String>> {
        let c = self.store.connection()?;
        let actor = crate::identity::actor_from_connection(&c, credential)?;
        Ok(policy(&c, &actor, &self.query())?
            .ok_or_else(Error::forbidden)?
            .sources)
    }
}
#[test]
fn source_policy_applies_subject_ceiling_and_hides_private_lineage_and_diagnostics() {
    let f = Fixture::new(true);
    let full = f.sources(&f.credential).unwrap();
    assert!(full.contains("appearance.1"));
    assert!(!full.contains("connection.0"));
    assert!(!full.contains("session.capture"));
    let key = f
        .store
        .issue(
            &f.owner,
            crate::Scope {
                operations: vec!["record_get".into()],
                recipients: vec![],
            },
            crate::now() + 3600,
        )
        .unwrap();
    let masked = f.sources(&key.id).unwrap();
    assert!(masked.contains("document"));
    assert!(masked.contains("session.outcome"));
    assert!(masked.contains("appearance.0"));
    assert!(!masked.contains("appearance.1"));
    assert!(!masked.contains("connection.0"));
    assert!(!masked.contains("session.capture"));
}
#[test]
fn only_complete_authorized_zero_omission_metadata_exposes_capture_diagnostics() {
    let f = Fixture::new(false);
    assert!(f
        .sources(&f.credential)
        .unwrap()
        .contains("session.capture"));
    let c = f.store.connection().unwrap();
    let raw: Vec<u8> = c
        .query_row(
            "SELECT body FROM session_records WHERE uuid=?1",
            [&f.uuid],
            |r| r.get(0),
        )
        .unwrap();
    let mut body: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    body["analysis"][1]["body"]["connection_omissions"] = json!(7);
    let body = serde_json::to_vec(&body).unwrap();
    c.execute(
        "UPDATE session_records SET body=?2,body_hash=?3 WHERE uuid=?1",
        params![f.uuid, body, hash(&body)],
    )
    .unwrap();
    drop(c);
    assert!(!f
        .sources(&f.credential)
        .unwrap()
        .contains("session.capture"));
}
#[test]
fn rejoined_membership_and_foreign_scope_cannot_borrow_original_record_epoch() {
    let f = Fixture::new(false);
    let c = f.store.connection().unwrap();
    c.execute_batch("UPDATE participants SET active=0 WHERE participant_id='part_a';UPDATE participants SET active=1,membership_epoch=2 WHERE participant_id='part_a';").unwrap();
    drop(c);
    assert!(f.sources(&f.credential).is_err());
    let c = f.store.connection().unwrap();
    let actor = crate::identity::actor_from_connection(&c, &f.credential).unwrap();
    let mut wrong = f.query();
    wrong.session_id = "foreign_session".into();
    assert!(origin(&c, &actor, &wrong).is_err());
}

fn records_page(
    f: &Fixture,
    credential: &str,
    page: Option<String>,
    limit: u32,
) -> connect_protocol::events::SnapshotPage {
    f.store
        .conversation_snapshot(
            credential,
            "conv_record",
            connect_protocol::events::SnapshotQuery {
                page,
                limit: Some(limit),
            },
        )
        .unwrap()
}
#[test]
fn discovery_requires_explicit_record_permission_and_omits_private_topology_and_digests() {
    use connect_protocol::events::{RecordAvailability, SnapshotItem};
    let f = Fixture::new(true);
    let key = f
        .store
        .issue(
            &f.owner,
            crate::Scope {
                operations: vec!["conversation.get".into(), "record_get".into()],
                recipients: vec![],
            },
            crate::now() + 300,
        )
        .unwrap();
    let page = records_page(&f, &key.id, None, 100);
    assert!(page.items.iter().any(|i|matches!(i,SnapshotItem::Record(RecordAvailability::Ready {uuid,revision:1,..}) if uuid==&f.uuid)));
    assert!(!page.items.iter().any(|i| matches!(
        i,
        SnapshotItem::Session(_)
            | SnapshotItem::Connection(_)
            | SnapshotItem::SessionParticipant(_)
    )));
    let encoded = serde_json::to_string(&page).unwrap();
    assert!(
        !encoded.contains("sha512-")
            && !encoded.contains("connect:vcon/")
            && !encoded.contains("part_b")
    );
    let absent = f
        .store
        .issue(
            &f.owner,
            crate::Scope {
                operations: vec!["conversation.get".into()],
                recipients: vec!["*".into()],
            },
            crate::now() + 300,
        )
        .unwrap();
    assert!(!records_page(&f, &absent.id, None, 100)
        .items
        .iter()
        .any(|i| matches!(i, SnapshotItem::Record(_))));
}
#[test]
fn cached_discovery_rechecks_deletion_and_fresh_snapshot_explains_unavailability() {
    use connect_protocol::events::{RecordAvailability, SnapshotItem};
    let f = Fixture::new(false);
    let page = records_page(&f, &f.credential, None, 1);
    let next = page.next.unwrap();
    let scope = RecordScope {
        tenant_id: "tnt_default".into(),
        conversation_id: "conv_record".into(),
        session_id: "sess_record".into(),
    };
    let h = f
        .store
        .list_records("tnt_default", "conv_record", Some("sess_record"))
        .unwrap()
        .pop()
        .unwrap();
    f.store
        .mutate_record(
            &scope,
            "delete-for-discovery",
            RecordMutationKind::Delete,
            "delete-v1",
            &h,
            |_| Ok(None),
        )
        .unwrap();
    let mut next = Some(next);
    while let Some(token) = next {
        let page = records_page(&f, &f.credential, Some(token), 1);
        assert!(!page
            .items
            .iter()
            .any(|i| matches!(i, SnapshotItem::Record(RecordAvailability::Ready { .. }))));
        next = page.next;
    }
    assert!(records_page(&f,&f.credential,None,100).items.iter().any(|i|matches!(i,SnapshotItem::Record(RecordAvailability::Unavailable {session_id}) if session_id=="sess_record")));
    let c = f.store.connection().unwrap();
    let actor = crate::identity::actor_from_connection(&c, &f.credential).unwrap();
    assert!(availability(&c, &actor, "conv_record", "sess_record")
        .unwrap()
        .is_some());
}
#[test]
fn discovery_disappears_after_rejoined_membership() {
    let f = Fixture::new(false);
    let c = f.store.connection().unwrap();
    let actor = crate::identity::actor_from_connection(&c, &f.credential).unwrap();
    assert!(availability(&c, &actor, "conv_record", "sess_record")
        .unwrap()
        .is_some());
    c.execute_batch("UPDATE participants SET active=0 WHERE participant_id='part_a';UPDATE participants SET active=1,membership_epoch=2 WHERE participant_id='part_a';").unwrap();
    assert!(availability(&c, &actor, "conv_record", "sess_record")
        .unwrap()
        .is_none());
}

#[test]
fn bearer_record_policy_reuses_only_original_complete_owned_read_authority() {
    let f = Fixture::new(false);
    let wait = crate::AuthorityWait::new(std::time::Duration::from_secs(1));
    let partial = f
        .store
        .acquire_authority(
            &[AuthorityScope::conversation("conv_record", AuthorityMode::Read).unwrap()],
            &wait,
        )
        .unwrap();
    assert!(f
        .store
        .record_read_policy(&partial, &f.credential, &f.query())
        .is_err());
    drop(partial);
    let held = f
        .store
        .read_authority(&f.credential, Some("conv_record"), &wait)
        .unwrap();
    let (origin, policy) = f
        .store
        .record_read_policy(&held, &f.credential, &f.query())
        .unwrap();
    assert_eq!(origin.viewer.participant(), Some(("part_a", 1)));
    assert!(policy.is_some());
    let other = Fixture::new(false);
    assert!(other
        .store
        .record_read_policy(&held, &other.credential, &other.query())
        .is_err());
}

fn role_command(
    identity: &str,
    operation: &str,
    expected: Option<u64>,
    private_sources: bool,
    active: bool,
) -> connect_protocol::provisioning::ProvisioningCommand {
    connect_protocol::provisioning::ProvisioningCommand::RecordReadRole {
        operation_id: operation.into(),
        identity_id: identity.into(),
        role: "auditor".into(),
        private_sources,
        active,
        expected_revision: expected,
    }
}
#[test]
fn independent_tenant_role_is_explicit_source_filtered_and_never_membership_or_erasure() {
    let f = Fixture::new(true);
    let outsider = f.store.create_identity("record-auditor").unwrap().id;
    let key = f
        .store
        .issue(
            &outsider,
            crate::Scope {
                operations: vec![
                    "record_get".into(),
                    "record_redact".into(),
                    "record_delete".into(),
                ],
                recipients: vec!["*".into()],
            },
            crate::now() + 300,
        )
        .unwrap();
    assert!(f.sources(&key.id).is_err());
    f.store
        .execute_provisioning(
            &f.credential,
            role_command(&outsider, "role-shared", None, false, true),
        )
        .unwrap();
    let shared = f.sources(&key.id).unwrap();
    assert!(shared.contains("document") && shared.contains("appearance.1"));
    assert!(!shared.contains("connection.0"));
    f.store
        .execute_provisioning(
            &f.credential,
            role_command(&outsider, "role-private", Some(1), true, true),
        )
        .unwrap();
    assert!(f.sources(&key.id).unwrap().contains("connection.0"));
    let narrow = f
        .store
        .issue(
            &outsider,
            crate::Scope {
                operations: vec!["record_get".into()],
                recipients: vec![],
            },
            crate::now() + 300,
        )
        .unwrap();
    let selected = f.sources(&narrow.id).unwrap();
    assert!(selected.contains("session.outcome"));
    assert!(!selected.contains("connection.0") && !selected.contains("appearance.0"));
    let wrong_scope = f
        .store
        .issue(
            &outsider,
            crate::Scope {
                operations: vec!["health".into()],
                recipients: vec!["*".into()],
            },
            crate::now() + 300,
        )
        .unwrap();
    assert!(f.sources(&wrong_scope.id).is_err());
    let mutation = connect_protocol::record_mutations::Request {
        operation_id: "tenant-read-is-not-erasure".into(),
        conversation_id: "conv_record".into(),
        session_id: "sess_record".into(),
        uuid: f.uuid.clone(),
        expected_revision: 1,
        action: connect_protocol::record_mutations::Action::Delete,
    };
    assert!(f
        .store
        .mutate_authorized_record(&key.id, &mutation, |_, _, _| panic!(
            "read role cannot erase"
        ))
        .is_err());
    let c = f.store.connection().unwrap();
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM participants WHERE identity_id=?1",
            [&outsider],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    let actor = crate::identity::actor_from_connection(&c, &key.id).unwrap();
    let snapshot = policy(&c, &actor, &f.query()).unwrap().unwrap();
    assert!(matches!(
        snapshot.binding.viewer,
        RecordViewer::TenantRole {
            revision: 2,
            private_sources: true,
            ..
        }
    ));
    let mut forged = snapshot.binding;
    forged.viewer = RecordViewer::Participant {
        participant_id: "part_a".into(),
        membership_epoch: 1,
    };
    assert!(sources(&c, &actor, &f.query(), &forged).is_err());
}
#[test]
fn tenant_role_cas_replay_revocation_recovery_and_cross_tenant_refusal() {
    use connect_protocol::provisioning::ProvisioningOutcome;
    let f = Fixture::new(true);
    let identity = f.store.create_identity("role-owner").unwrap().id;
    let key = f
        .store
        .issue(
            &identity,
            crate::Scope {
                operations: vec!["record_get".into()],
                recipients: vec!["*".into()],
            },
            crate::now() + 300,
        )
        .unwrap();
    let create = role_command(&identity, "create-role", None, true, true);
    f.store
        .execute_provisioning(&f.credential, create.clone())
        .unwrap();
    assert!(f
        .store
        .execute_provisioning(
            &key.id,
            role_command(&identity, "self-promote-role", Some(1), true, true)
        )
        .is_err());
    assert!(f
        .store
        .execute_provisioning(
            &f.credential,
            role_command(&identity, "wrong-role-revision", Some(0), true, false)
        )
        .is_err());
    f.store
        .execute_provisioning(
            &f.credential,
            role_command(&identity, "revoke-role", Some(1), true, false),
        )
        .unwrap();
    assert!(f.sources(&key.id).is_err());
    let replay = f.store.execute_provisioning(&f.credential, create).unwrap();
    assert!(matches!(
        replay,
        ProvisioningOutcome::RecordReadRole {
            active: false,
            revision: 2,
            ..
        }
    ));
    let reopened = Store::open_administration(f.store.path(), Limits::default()).unwrap();
    assert!(reopened
        .execute_provisioning(
            &f.credential,
            role_command(&identity, "reactivate-role", Some(2), false, true)
        )
        .is_ok());
    assert!(!f.sources(&key.id).unwrap().contains("connection.0"));
    let c = f.store.connection().unwrap();
    c.execute_batch("INSERT INTO tenants(id) VALUES('tnt_other'); INSERT INTO identities(id,alias,capabilities,tenant_id) VALUES('id_other','other-tenant','[]','tnt_other');INSERT INTO conversations(id,tenant_id) VALUES('conv_other','tnt_other');INSERT INTO sessions(id,conversation_id,initiator_id,participant_id,state,media,created_at,updated_at) VALUES('sess_other','conv_other','id_other','id_other','invited','[]',100,100);").unwrap();
    let actor = crate::identity::actor_from_connection(&c, &key.id).unwrap();
    let mut foreign = f.query();
    foreign.conversation_id = "conv_other".into();
    foreign.session_id = "sess_other".into();
    assert!(origin(&c, &actor, &foreign).is_err());
    drop(c);
    assert!(f
        .store
        .execute_provisioning(
            &f.credential,
            role_command("id_other", "foreign-role", None, true, true)
        )
        .is_err());
}
#[test]
fn queued_role_revocation_waits_for_original_read_and_policy_does_not_reenter_guard() {
    let f = Fixture::new(true);
    let identity = f.store.create_identity("held-role").unwrap().id;
    let key = f
        .store
        .issue(
            &identity,
            crate::Scope {
                operations: vec!["record_get".into()],
                recipients: vec!["*".into()],
            },
            crate::now() + 300,
        )
        .unwrap();
    f.store
        .execute_provisioning(
            &f.credential,
            role_command(&identity, "held-role-create", None, true, true),
        )
        .unwrap();
    let held = f
        .store
        .record_read_authority(
            &key.id,
            &f.query(),
            &AuthorityWait::new(Duration::from_secs(2)),
        )
        .unwrap();
    std::thread::scope(|scope| {
        let (sender, receiver) = std::sync::mpsc::channel();
        let identity = &identity;
        let store = &f.store;
        let admin = &f.credential;
        let task = scope.spawn(move || {
            let result = store.execute_provisioning(
                admin,
                role_command(identity, "held-role-revoke", Some(1), true, false),
            );
            sender.send(result).unwrap();
        });
        assert!(receiver.recv_timeout(Duration::from_millis(100)).is_err());
        let (_, policy) = f
            .store
            .record_read_policy(&held, &key.id, &f.query())
            .unwrap();
        assert!(policy.unwrap().sources.contains("connection.0"));
        drop(held);
        assert!(receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .is_ok());
        task.join().unwrap();
    });
    assert!(f.sources(&key.id).is_err());
}

#[test]
fn shared_tenant_role_preserves_member_sources_and_full_read_role_does_not_elevate_member_erasure()
{
    let f = Fixture::new(true);
    let c = f.store.connection().unwrap();
    let peer: String = c
        .query_row(
            "SELECT identity_id FROM participants WHERE participant_id='part_b'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    drop(c);
    let peer_key = f
        .store
        .issue(
            &peer,
            crate::Scope {
                operations: vec!["record_get".into()],
                recipients: vec!["*".into()],
            },
            crate::now() + 300,
        )
        .unwrap();
    assert!(f.sources(&peer_key.id).unwrap().contains("connection.0"));
    f.store
        .execute_provisioning(
            &f.credential,
            role_command(&peer, "member-shared-role", None, false, true),
        )
        .unwrap();
    assert!(f.sources(&peer_key.id).unwrap().contains("connection.0"));
    let c = f.store.connection().unwrap();
    let actor = crate::identity::actor_from_connection(&c, &peer_key.id).unwrap();
    assert!(matches!(
        origin(&c, &actor, &f.query()).unwrap().viewer,
        RecordViewer::Participant { .. }
    ));
    drop(c);
    f.store
        .execute_provisioning(
            &f.credential,
            role_command(&f.owner, "member-full-role", None, true, true),
        )
        .unwrap();
    let key = f
        .store
        .issue(
            &f.owner,
            crate::Scope {
                operations: vec!["record_get".into(), "record_delete".into()],
                recipients: vec!["*".into()],
            },
            crate::now() + 300,
        )
        .unwrap();
    assert!(f.sources(&key.id).unwrap().contains("connection.0"));
    let deletion = connect_protocol::record_mutations::Request {
        operation_id: "full-role-not-member-erasure".into(),
        conversation_id: "conv_record".into(),
        session_id: "sess_record".into(),
        uuid: f.uuid.clone(),
        expected_revision: 1,
        action: connect_protocol::record_mutations::Action::Delete,
    };
    assert!(
        matches!(f.store.mutate_authorized_record(&key.id,&deletion,|_,_,_|Ok(None)),Err(e) if e.code=="forbidden")
    );
}

#[test]
fn readiness_captures_original_epochs_and_current_record_policy_without_bytes_or_a_grant() {
    let f = Fixture::new(true);
    let c = f.store.connection().unwrap();
    let actor = crate::identity::actor_from_connection(&c, &f.credential).unwrap();
    let query = f.query();
    crate::records::readiness::scopes(&c, &actor, &query).unwrap();
    let (fact, audience) =
        crate::records::readiness::stored(&c, "conv_record", "sess_record", &f.uuid).unwrap();
    assert!(matches!(
        fact,
        connect_protocol::events::RecordAvailability::Ready { revision: 1, .. }
    ));
    assert_eq!(audience.len(), 2);
    let rows: u64 = c
        .query_row(
            "SELECT COUNT(*) FROM session_lifecycle_hints WHERE kind='record_ready'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 1);
    assert!(c
        .execute("UPDATE record_ready_facts SET revision=revision", [])
        .is_err());
    assert!(c.execute("DELETE FROM record_ready_audiences", []).is_err());
    drop(c);
    assert!(!f
        .store
        .finalize_record_intent(&f.uuid, |_| panic!(
            "original ready outcome cannot be rebuilt"
        ))
        .unwrap());
    let no_read = f
        .store
        .issue(
            &f.owner,
            crate::Scope {
                operations: vec!["session_get".into()],
                recipients: vec!["*".into()],
            },
            crate::now() + 600,
        )
        .unwrap();
    let c = f.store.connection().unwrap();
    let actor = crate::identity::actor_from_connection(&c, &no_read.id).unwrap();
    assert_eq!(
        crate::records::readiness::scopes(&c, &actor, &query)
            .unwrap_err()
            .code,
        "forbidden"
    );
}
#[test]
fn readiness_refuses_removed_and_readmitted_recipient_and_retains_original_epoch() {
    let f = Fixture::new(false);
    let c = f.store.connection().unwrap();
    c.execute(
        "UPDATE participants SET active=0 WHERE participant_id='part_a'",
        [],
    )
    .unwrap();
    c.execute(
        "UPDATE participants SET active=1,membership_epoch=2 WHERE participant_id='part_a'",
        [],
    )
    .unwrap();
    let actor = crate::identity::actor_from_connection(&c, &f.credential).unwrap();
    assert!(crate::records::readiness::scopes(&c, &actor, &f.query()).is_err());
    let (_, members) =
        crate::records::readiness::stored(&c, "conv_record", "sess_record", &f.uuid).unwrap();
    assert!(members
        .iter()
        .any(|(_, pid, epoch)| pid == "part_a" && *epoch == 1));
}
#[test]
fn readiness_refuses_stale_revision_deleted_content_foreign_scope_and_unavailable_sources() {
    for change in 0..4 {
        let f = Fixture::new(false);
        let c = f.store.connection().unwrap();
        let actor = crate::identity::actor_from_connection(&c, &f.credential).unwrap();
        let mut query = f.query();
        match change {
            0 => {
                c.execute(
                    "UPDATE session_records SET revision=2 WHERE uuid=?1",
                    [&f.uuid],
                )
                .unwrap();
            }
            1 => {
                c.execute("UPDATE session_records SET revision=2,deleted=1,body=NULL,manifest=NULL WHERE uuid=?1",[&f.uuid]).unwrap();
            }
            2 => {
                query.session_id = "foreign-session".into();
            }
            _ => {
                let manifest =
                    serde_json::to_vec(&json!({"policy":"unavailable-policy","sources":[]}))
                        .unwrap();
                c.execute(
                    "UPDATE session_records SET manifest=?2,manifest_hash=?3 WHERE uuid=?1",
                    params![f.uuid, manifest, hash(&manifest)],
                )
                .unwrap();
            }
        }
        assert!(
            crate::records::readiness::scopes(&c, &actor, &query).is_err(),
            "change {change}"
        );
    }
}

#[test]
fn readiness_document_cutoff_and_unavailable_metadata_refuse_before_retention_worker_mutates_bytes()
{
    let f = Fixture::new(false);
    let c = f.store.connection().unwrap();
    let actor = crate::identity::actor_from_connection(&c, &f.credential).unwrap();
    let original: Vec<u8> = c
        .query_row(
            "SELECT manifest FROM session_records WHERE uuid=?1",
            [&f.uuid],
            |r| r.get(0),
        )
        .unwrap();
    for change in 0..3 {
        let mut manifest: serde_json::Value = serde_json::from_slice(&original).unwrap();
        let until = crate::now() + if change == 0 { 30 } else { -1 };
        manifest["sources"][0]["metadata_retention"] =
            json!({"kind":"until","until":chrono::DateTime::from_timestamp(until,0).unwrap()});
        if change == 2 {
            manifest["sources"][0]["state"] = json!("metadata-unavailable");
        }
        let raw = serde_json::to_vec(&manifest).unwrap();
        c.execute(
            "UPDATE session_records SET manifest=?2,manifest_hash=?3 WHERE uuid=?1",
            params![f.uuid, raw, hash(&raw)],
        )
        .unwrap();
        if change == 0 {
            assert_eq!(
                crate::records::readiness::expiry(&c, &f.query()).unwrap(),
                Some(until)
            );
            crate::records::readiness::scopes(&c, &actor, &f.query()).unwrap();
        } else {
            assert!(crate::records::readiness::scopes(&c, &actor, &f.query()).is_err());
        }
        assert_eq!(
            c.query_row(
                "SELECT revision FROM session_records WHERE uuid=?1",
                [&f.uuid],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            1
        );
    }
}
