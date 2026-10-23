#![allow(clippy::unwrap_used)]
use super::*;
use connect_protocol::{Limits, Operation, Outcome};
use rvoip_core::{
    ids::SessionId,
    records::redaction::{SourceRedaction, SourceRedactionMode},
};
use serde_json::json;
use std::{process::Command, time::Duration};

struct Fixture {
    dir: tempfile::TempDir,
    store: Arc<Store>,
    scope: VconScope,
    credential: String,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::initialize(&dir.path().join("state.sqlite3"), Limits::default()).unwrap(),
        );
        let owner = store.create_identity("owner").unwrap().id;
        let peer = store.create_identity("peer").unwrap().id;
        let credential = store
            .issue(
                &owner,
                connect_store::Scope::operator(),
                chrono::Utc::now().timestamp() + 3600,
            )
            .unwrap();
        let Outcome::Conversation(conversation) = store
            .execute(
                &credential.id,
                &Operation::Create {
                    request_id: "conversation".into(),
                    request: connect_protocol::conversations::CreateRequest::persistent(vec![
                        peer.clone()
                    ]),
                },
            )
            .unwrap()
        else {
            panic!("conversation")
        };
        let Outcome::Session(session) = store
            .execute(
                &credential.id,
                &Operation::SessionInvite {
                    request_id: "invite".into(),
                    conversation_id: conversation.id.clone(),
                    participant_id: peer,
                    media: vec!["audio".into()],
                },
            )
            .unwrap()
        else {
            panic!("session")
        };
        Self {
            credential: credential.id,
            dir,
            store,
            scope: VconScope::new(
                TenantId::from_string("tnt_default"),
                ConversationId::from_string(conversation.id),
                SessionId::from_string(session.id),
            ),
        }
    }
    fn records(&self) -> Records {
        Records::new(self.store.clone())
    }
}
fn value(marker: &str) -> VconRecord {
    let mut id = *uuid::Uuid::new_v4().as_bytes();
    id[6] = (id[6] & 15) | 128;
    let id = uuid::Uuid::from_bytes(id);
    VconRecord::unattributed(
        serde_json::to_vec(
            &json!({"vcon":"0.4.0","uuid":id,"created_at":chrono::Utc::now(),"subject":marker}),
        )
        .unwrap()
        .into(),
    )
    .unwrap()
}
fn redaction(handle: &VconHandle, record: &VconRecord, operation: &str) -> VconRedaction {
    let source = record
        .manifest()
        .sources()
        .iter()
        .find(|s| s.target() == rvoip_core::records::RecordTarget::Document)
        .unwrap();
    VconRedaction::new(
        operation.into(),
        handle.clone(),
        vec![SourceRedaction {
            source_id: source.id().into(),
            mode: SourceRedactionMode::Content,
        }],
    )
    .unwrap()
}
#[tokio::test]
async fn durable_record_reopens_exact_bytes_manifest_and_local_selector_scope() {
    let f = Fixture::new();
    let records = f.records();
    let original = value("private-record-canary");
    let handle = records.put(&f.scope, original.clone()).await.unwrap();
    assert_eq!(
        records
            .put(&f.scope, original.clone())
            .await
            .unwrap()
            .content_hash,
        handle.content_hash
    );
    let backup = f.dir.path().join("backup.sqlite3");
    let snapshot = f.store.backup(&backup).unwrap();
    let reopened = Records::new(Arc::new(
        Store::open_administration(&backup, Limits::default()).unwrap(),
    ));
    assert_eq!(Store::inspect_snapshot(&backup).unwrap(), snapshot);
    let read = reopened.get(&f.scope, &handle).await.unwrap().unwrap();
    assert_eq!(read.body(), original.body());
    assert_eq!(
        read.record().manifest_hash().unwrap(),
        original.manifest_hash().unwrap()
    );
    drop(read);
    let locator = VconLocator::new(Some(original.uuid()), Some(handle.url.clone())).unwrap();
    assert!(reopened.lookup(&f.scope, &locator).await.unwrap().is_some());
    let mismatch = VconLocator::new(Some(uuid::Uuid::new_v4()), Some(handle.url.clone())).unwrap();
    assert!(reopened
        .lookup(&f.scope, &mismatch)
        .await
        .unwrap()
        .is_none());
    let arbitrary = VconLocator::new(None, Some("https://127.0.0.1:1/private".into())).unwrap();
    assert!(reopened
        .lookup(&f.scope, &arbitrary)
        .await
        .unwrap()
        .is_none());
    for foreign in [
        VconScope::new(
            TenantId::from_string("foreign"),
            f.scope.conversation_id().clone(),
            f.scope.session_id().clone(),
        ),
        VconScope::new(
            f.scope.tenant_id().clone(),
            ConversationId::new(),
            f.scope.session_id().clone(),
        ),
        VconScope::new(
            f.scope.tenant_id().clone(),
            f.scope.conversation_id().clone(),
            SessionId::new(),
        ),
    ] {
        assert!(reopened.lookup(&foreign, &locator).await.unwrap().is_none());
        assert!(records.put(&foreign, original.clone()).await.is_err());
    }
    let mut changed = serde_json::from_slice::<serde_json::Value>(original.body()).unwrap();
    changed["subject"] = json!("changed-create-intent");
    assert!(records
        .put(
            &f.scope,
            VconRecord::unattributed(serde_json::to_vec(&changed).unwrap().into()).unwrap()
        )
        .await
        .is_err());
    assert_eq!(records.list_for_session(&f.scope).await.unwrap().len(), 1);
    assert_eq!(
        records
            .list_for_conversation(f.scope.tenant_id(), f.scope.conversation_id())
            .await
            .unwrap()
            .len(),
        1
    );
}
#[tokio::test]
async fn durable_redaction_and_deletion_recover_original_facts_without_resurrection() {
    let f = Fixture::new();
    let records = f.records();
    let original = value("private-record-canary");
    let first = records.put(&f.scope, original.clone()).await.unwrap();
    let redact = redaction(&first, &original, "redact_one");
    let fact = records.redact(&f.scope, &redact).await.unwrap().unwrap();
    assert_eq!(fact.result().revision, 2);
    assert!(records.get(&f.scope, &first).await.is_err());
    let current = records.get(&f.scope, fact.result()).await.unwrap().unwrap();
    assert!(!String::from_utf8_lossy(current.body()).contains("private-record-canary"));
    drop(current);
    let remove = VconDeletion::new("remove_one".into(), fact.result().clone()).unwrap();
    let tombstone = records.delete(&f.scope, &remove).await.unwrap().unwrap();
    assert_eq!(tombstone.revision(), 3);
    assert!(records.get(&f.scope, &first).await.unwrap().is_none());
    assert!(records.put(&f.scope, original.clone()).await.is_err());
    assert!(records.list_for_session(&f.scope).await.unwrap().is_empty());
    let reopened = Records::new(Arc::new(
        Store::open_administration(f.store.path(), Limits::default()).unwrap(),
    ));
    assert_eq!(
        reopened
            .redact(&f.scope, &redact)
            .await
            .unwrap()
            .unwrap()
            .at(),
        fact.at()
    );
    assert_eq!(
        reopened
            .delete(&f.scope, &remove)
            .await
            .unwrap()
            .unwrap()
            .deleted_at(),
        tombstone.deleted_at()
    );
    assert_eq!(
        reopened
            .tombstone(&f.scope, &first.url)
            .await
            .unwrap()
            .unwrap()
            .operation_id(),
        "remove_one"
    );
    assert!(reopened
        .delete(
            &f.scope,
            &VconDeletion::new("redact_one".into(), fact.result().clone()).unwrap()
        )
        .await
        .is_err());
    assert!(reopened
        .redact(&f.scope, &redaction(fact.result(), &original, "redact_one"))
        .await
        .is_err());
    assert!(reopened
        .delete(
            &f.scope,
            &VconDeletion::new("remove_changed".into(), first).unwrap()
        )
        .await
        .is_err());
}
#[tokio::test]
async fn durable_record_read_fences_other_handle_writer_while_independent_record_proceeds() {
    let f = Fixture::new();
    let records = f.records();
    let first = records.put(&f.scope, value("delivery")).await.unwrap();
    let other = records.put(&f.scope, value("independent")).await.unwrap();
    let held = records.get(&f.scope, &first).await.unwrap().unwrap();
    let independent = Records::new(Arc::new(
        Store::open_administration(f.store.path(), Limits::default()).unwrap(),
    ));
    let remove = VconDeletion::new("blocked_delete".into(), first.clone()).unwrap();
    let scope = f.scope.clone();
    let deleting = tokio::spawn(async move { independent.delete(&scope, &remove).await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!deleting.is_finished());
    assert!(records.get(&f.scope, &other).await.unwrap().is_some());
    assert_eq!(held.handle().revision, 1);
    drop(held);
    assert!(tokio::time::timeout(Duration::from_secs(2), deleting)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .is_some());
    assert!(records.get(&f.scope, &first).await.unwrap().is_none());
}
#[test]
#[ignore = "Subprocess helper invoked by the cross-process custody test"]
fn record_delete_process_helper() {
    let Some(path) = std::env::var_os("CONNECT_RECORD_CHILD_PATH") else {
        return;
    };
    let scope: Vec<String> =
        serde_json::from_str(&std::env::var("CONNECT_RECORD_CHILD_SCOPE").unwrap()).unwrap();
    let expected: durable::RecordHandle =
        serde_json::from_str(&std::env::var("CONNECT_RECORD_CHILD_HANDLE").unwrap()).unwrap();
    let store = Store::open_administration(std::path::Path::new(&path), Limits::default()).unwrap();
    std::fs::write(
        std::env::var_os("CONNECT_RECORD_CHILD_READY").unwrap(),
        b"ready",
    )
    .unwrap();
    store
        .mutate_record(
            &durable::RecordScope {
                tenant_id: scope[0].clone(),
                conversation_id: scope[1].clone(),
                session_id: scope[2].clone(),
            },
            "child_delete",
            durable::RecordMutationKind::Delete,
            "delete-v1",
            &expected,
            |_| Ok(None),
        )
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn durable_read_retains_cross_process_custody_until_delivery_drop() {
    let f = Fixture::new();
    let records = f.records();
    let handle = records.put(&f.scope, value("delivery")).await.unwrap();
    let read = records.get(&f.scope, &handle).await.unwrap().unwrap();
    let ready = f.dir.path().join("child-ready");
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "canonical_records::tests::record_delete_process_helper",
            "--ignored",
            "--nocapture",
        ])
        .env("CONNECT_RECORD_CHILD_PATH", f.store.path())
        .env("CONNECT_RECORD_CHILD_READY", &ready)
        .env(
            "CONNECT_RECORD_CHILD_SCOPE",
            serde_json::to_string(&vec![
                f.scope.tenant_id().to_string(),
                f.scope.conversation_id().to_string(),
                f.scope.session_id().to_string(),
            ])
            .unwrap(),
        )
        .env(
            "CONNECT_RECORD_CHILD_HANDLE",
            serde_json::to_string(&expected(&handle)).unwrap(),
        )
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut child = child;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while !ready.exists() {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(child.try_wait().unwrap().is_none());
    drop(read);
    let status = tokio::task::spawn_blocking(move || child.wait().unwrap())
        .await
        .unwrap();
    assert!(status.success());
    assert!(records.get(&f.scope, &handle).await.unwrap().is_none());
}

#[tokio::test]
async fn durable_failed_transform_and_wrong_digests_preserve_original_record_and_snapshot() {
    let f = Fixture::new();
    let records = f.records();
    let original = value("original");
    let before = Store::inspect_snapshot(f.store.path()).unwrap();
    let handle = records.put(&f.scope, original.clone()).await.unwrap();
    let after = Store::inspect_snapshot(f.store.path()).unwrap();
    assert_ne!(before.digest, after.digest);
    for bad_manifest in [false, true] {
        let mut wrong = handle.clone();
        if bad_manifest {
            wrong.manifest_hash = "sha512-wrong".into();
        } else {
            wrong.content_hash = "sha512-wrong".into();
        }
        assert!(records.get(&f.scope, &wrong).await.is_err());
        assert!(records
            .delete(
                &f.scope,
                &VconDeletion::new("wrong_hash".into(), wrong).unwrap()
            )
            .await
            .is_err());
    }
    let bad = f.store.mutate_record(
        &scope(&f.scope),
        "rollback",
        durable::RecordMutationKind::Redact,
        "redact-v1",
        &expected(&handle),
        |_| Err(connect_protocol::Error::storage()),
    );
    assert!(bad.is_err());
    assert_eq!(Store::inspect_snapshot(f.store.path()).unwrap(), after);
    let read = records.get(&f.scope, &handle).await.unwrap().unwrap();
    assert_eq!(read.body(), original.body());
    drop(read);
    // A failed attempt records no original-operation receipt and can be retried
    // with the same intent after its local transformation becomes available.
    f.store
        .mutate_record(
            &scope(&f.scope),
            "rollback",
            durable::RecordMutationKind::Redact,
            "redact-v1",
            &expected(&handle),
            |current| Ok(Some(current.clone())),
        )
        .unwrap()
        .unwrap();
    assert_ne!(Store::inspect_snapshot(f.store.path()).unwrap(), after);
}

#[tokio::test]
async fn terminal_session_without_core_or_media_recovers_one_original_metadata_outcome() {
    let f = Fixture::new();
    f.store
        .execute(
            &f.credential,
            &Operation::SessionCancel {
                request_id: "cancel".into(),
                conversation_id: f.scope.conversation_id().to_string(),
                session_id: f.scope.session_id().to_string(),
            },
        )
        .unwrap();
    let original = f.store.pending_record_intents(1).unwrap().pop().unwrap();
    assert!(f
        .records()
        .list_for_session(&f.scope)
        .await
        .unwrap()
        .is_empty());
    // Reopen as a fresh Service; no Core runtime or captured media exists.
    let reopened = Arc::new(Store::open_administration(f.store.path(), Limits::default()).unwrap());
    let service = crate::Service::new(reopened.clone());
    let batch = service.finalize_record_outcomes(1).await.unwrap();
    assert_eq!((batch.examined, batch.stored, batch.failed), (1, 1, 0));
    assert!(service.canonical_runtime_snapshot().is_none());
    let records = Records::new(reopened);
    let handle = records
        .list_for_session(&f.scope)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let record = records.get(&f.scope, &handle).await.unwrap().unwrap();
    let body: serde_json::Value = serde_json::from_slice(record.body()).unwrap();
    assert_eq!(body["uuid"], original);
    assert_eq!(body["extensions"], json!(["connect-session-metadata"]));
    assert!(body.get("group").is_none());
    assert_eq!(body["analysis"][0]["body"]["state"], "cancelled");
    assert_eq!(body["analysis"][0]["body"]["capture_complete"], false);
    assert_eq!(body["analysis"][0]["body"]["recorded_audio"], false);
    assert_eq!(body["analysis"][0]["body"]["transcript"], "unavailable");
    assert!(body.get("dialog").is_none());
    assert_eq!(body["parties"].as_array().unwrap().len(), 2);
    assert_eq!(
        record.record().manifest().policy(),
        "connect-session-metadata-v1"
    );
    let exact = record.body().clone();
    drop(record);
    assert_eq!(
        service.finalize_record_outcomes(32).await.unwrap().stored,
        0
    );
    assert_eq!(
        records
            .get(&f.scope, &handle)
            .await
            .unwrap()
            .unwrap()
            .body(),
        &exact
    );
    records
        .delete(
            &f.scope,
            &VconDeletion::new("delete_outcome".into(), handle.clone()).unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        service.finalize_record_outcomes(32).await.unwrap().stored,
        0
    );
    assert!(records.get(&f.scope, &handle).await.unwrap().is_none());
}

#[tokio::test]
async fn frozen_named_metadata_retention_expires_canonical_outcome_and_never_resurrects_on_restart()
{
    use connect_protocol::record_retention::{Lifetime, Policy};
    let f = Fixture::new();
    f.store
        .configure_record_retention(&Policy {
            name: "one-second-test".into(),
            metadata: Lifetime::After { seconds: 1 },
            ..Default::default()
        })
        .unwrap();
    f.store
        .execute(
            &f.credential,
            &Operation::SessionCancel {
                request_id: "retention-cancel".into(),
                conversation_id: f.scope.conversation_id().to_string(),
                session_id: f.scope.session_id().to_string(),
            },
        )
        .unwrap();
    f.store
        .configure_record_retention(&Policy::default())
        .unwrap();
    let service = crate::Service::new(f.store.clone());
    assert_eq!(service.finalize_record_outcomes(1).await.unwrap().stored, 1);
    let records = f.records();
    let handle = records
        .list_for_session(&f.scope)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let read = records.get(&f.scope, &handle).await.unwrap().unwrap();
    let body: serde_json::Value = serde_json::from_slice(read.body()).unwrap();
    assert_eq!(
        body["analysis"][0]["body"]["retention_policy"],
        "one-second-test"
    );
    let expires = read
        .record()
        .manifest()
        .sources()
        .iter()
        .find(|s| s.target() == rvoip_core::records::RecordTarget::Document)
        .unwrap()
        .metadata_retention();
    let rvoip_core::records::RecordRetention::Until(at) = expires else {
        panic!("frozen deadline")
    };
    drop(read);
    while chrono::Utc::now() < at {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let batch = service.expire_record_outcomes(32).await.unwrap();
    assert_eq!((batch.examined, batch.stored, batch.failed), (1, 1, 0));
    assert!(records.get(&f.scope, &handle).await.unwrap().is_none());
    let reopened = Arc::new(Store::open_administration(f.store.path(), Limits::default()).unwrap());
    let service = crate::Service::new(reopened.clone());
    assert_eq!(
        service.finalize_record_outcomes(32).await.unwrap().stored,
        0
    );
    assert_eq!(service.expire_record_outcomes(32).await.unwrap().stored, 0);
    assert!(Records::new(reopened)
        .get(&f.scope, &handle)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn pending_content_expiry_keeps_original_selection_then_separately_erases_expired_source_metadata(
) {
    use rvoip_core::records::{
        RecordAudience, RecordCategory, RecordRetention, RecordSource, RecordSourceState,
        RecordTarget,
    };
    let f = Fixture::new();
    let now = chrono::DateTime::from_timestamp(chrono::Utc::now().timestamp(), 0).unwrap();
    let metadata_deadline = now + chrono::Duration::seconds(2);
    let mut id = *uuid::Uuid::new_v4().as_bytes();
    id[6] = (id[6] & 15) | 128;
    let id = uuid::Uuid::from_bytes(id);
    let until = RecordRetention::UntilDeleted;
    let original=VconRecord::new(serde_json::to_vec(&json!({"vcon":"0.4.0","uuid":id,"created_at":now,"analysis":[{"type":"transcript","vendor":"connect-test","encoding":"json","mediatype":"application/json","body":{"text":"retention-private-content-canary"}}]})).unwrap().into(),RecordManifest::new("retention-test".into(),vec![
        RecordSource::new("document".into(),1,RecordTarget::Document,RecordCategory::Metadata,RecordAudience::Policy,"fixture-metadata".into(),until,until),
        RecordSource::new("transcript".into(),1,RecordTarget::Analysis(0),RecordCategory::Transcript,RecordAudience::Policy,"fixture-transcript".into(),RecordRetention::Until(metadata_deadline),RecordRetention::Until(now-chrono::Duration::seconds(1))),
    ]).unwrap()).unwrap();
    let records = f.records();
    let handle = records.put(&f.scope, original.clone()).await.unwrap();
    f.store
        .prepare_record_retention(
            &original.uuid().to_string(),
            now.timestamp(),
            super::retention::select,
        )
        .unwrap()
        .unwrap();
    // The retry happens after a second deadline. It must apply the original
    // content-only decision before a separate metadata expiry is selected.
    while chrono::Utc::now() < metadata_deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let service = crate::Service::new(f.store.clone());
    let batch = service.expire_record_outcomes(32).await.unwrap();
    assert_eq!((batch.stored, batch.failed), (1, 0));
    let current = records
        .list_for_session(&f.scope)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(current.revision, handle.revision + 1);
    let read = records.get(&f.scope, &current).await.unwrap().unwrap();
    assert!(!String::from_utf8_lossy(read.body()).contains("retention-private-content-canary"));
    assert_eq!(
        read.record()
            .manifest()
            .sources()
            .iter()
            .find(|s| s.id() == "transcript")
            .unwrap()
            .state(),
        RecordSourceState::ContentUnavailable
    );
    drop(read);
    let batch = service.expire_record_outcomes(32).await.unwrap();
    assert_eq!((batch.stored, batch.failed), (1, 0));
    let current = records
        .list_for_session(&f.scope)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(current.revision, handle.revision + 2);
    let read = records.get(&f.scope, &current).await.unwrap().unwrap();
    assert_eq!(
        read.record()
            .manifest()
            .sources()
            .iter()
            .find(|s| s.id() == "transcript")
            .unwrap()
            .state(),
        RecordSourceState::MetadataUnavailable
    );
    drop(read);
    assert_eq!(
        service.expire_record_outcomes(32).await.unwrap().examined,
        0
    );
}

async fn context_record_fixture(f: &Fixture) -> (Arc<crate::Service>, String) {
    f.store
        .execute(
            &f.credential,
            &Operation::SessionCancel {
                request_id: "context-cancel".into(),
                conversation_id: f.scope.conversation_id().to_string(),
                session_id: f.scope.session_id().to_string(),
            },
        )
        .unwrap();
    let uuid = f.store.pending_record_intents(1).unwrap().pop().unwrap();
    let service = crate::Service::new(f.store.clone());
    assert_eq!(service.finalize_record_outcomes(1).await.unwrap().stored, 1);
    (service, uuid)
}
fn context_view(f: &Fixture, credential: &str) -> connect_protocol::ContextView {
    match f
        .store
        .execute(
            credential,
            &Operation::ContextView {
                conversation_id: f.scope.conversation_id().to_string(),
                limit: 100,
            },
        )
        .unwrap()
    {
        Outcome::ContextView(view) => view,
        _ => panic!("context view"),
    }
}
#[tokio::test]
async fn record_context_selection_has_exact_projection_and_independent_permission_and_task_ceiling()
{
    use connect_protocol::tasks::*;
    let f = Fixture::new();
    let (_service, uuid) = context_record_fixture(&f).await;
    let view = context_view(&f, &f.credential);
    let item = view.items.iter().find(|i| i.source_id == uuid).unwrap();
    assert_eq!(item.kind, "record_ready");
    let reference = item.record.as_ref().unwrap();
    assert_eq!(
        reference.state,
        connect_protocol::ContextRecordState::Available
    );
    assert_eq!(reference.revision, 1);
    assert!(reference
        .content_hash
        .as_ref()
        .unwrap()
        .starts_with("sha512-"));
    let body: serde_json::Value = serde_json::from_str(&item.body).unwrap();
    assert_eq!(body["uuid"], uuid);
    let encoded = serde_json::to_string(&reference).unwrap();
    assert!(!encoded.contains("manifest_hash") && !encoded.contains("body_hash"));
    let actor = f.store.revalidate(&f.credential).unwrap();
    let restricted = f
        .store
        .issue(
            &actor.identity_id,
            connect_store::Scope {
                operations: vec!["context_view".into()],
                recipients: vec!["*".into()],
            },
            chrono::Utc::now().timestamp() + 300,
        )
        .unwrap();
    assert!(!context_view(&f, &restricted.id)
        .items
        .iter()
        .any(|i| i.record.is_some()));
    for allowed in [false, true] {
        let mut operations = vec![TaskPermission::ContextView];
        if allowed {
            operations.push(TaskPermission::RecordGet);
        }
        let TaskOutcome::Record(task) = f
            .store
            .execute_task(
                &f.credential,
                &TaskRequest::new(TaskCommand::Create {
                    request_id: format!("context-task-{allowed}"),
                    definition: TaskDefinition {
                        conversation_id: f.scope.conversation_id().to_string(),
                        scope: TaskScope {
                            operations,
                            recipients: vec![actor.identity_id.clone()],
                            sms_enrollments: vec![],
                        },
                        expires_at: chrono::Utc::now().timestamp() + 300,
                        initiating_event_id: None,
                    },
                }),
            )
            .unwrap()
        else {
            panic!("task")
        };
        let TaskOutcome::Recall { sources, .. } = f
            .store
            .execute_task(
                &f.credential,
                &TaskRequest::new(TaskCommand::Recall {
                    task_id: task.id.clone(),
                    after: None,
                    limit: 100,
                }),
            )
            .unwrap()
        else {
            panic!("recall")
        };
        assert_eq!(sources.iter().any(|s| s.record.is_some()), allowed);
        let TaskOutcome::Tool(TaskToolResult::Completed { outcome, .. }) = f
            .store
            .execute_task(
                &f.credential,
                &TaskRequest::new(TaskCommand::Invoke {
                    request_id: format!("context-read-{allowed}"),
                    task_id: task.id.clone(),
                    step_id: "context".into(),
                    operation: Box::new(Operation::ContextView {
                        conversation_id: f.scope.conversation_id().to_string(),
                        limit: 100,
                    }),
                    reply_to: None,
                    wait_for_reply: false,
                }),
            )
            .unwrap()
        else {
            panic!("context tool")
        };
        let Outcome::ContextView(view) = *outcome else {
            panic!("context")
        };
        assert_eq!(view.items.iter().any(|s| s.record.is_some()), allowed);
        if allowed {
            for grant_records in [false, true] {
                let binding = InvocationBinding {
                    kind: InvocationKind::Native,
                    adapter: "record-context-fixture".into(),
                    external_id: format!("record-grant-{grant_records}"),
                };
                f.store
                    .register_task_binding(&f.credential, &task.id, &binding)
                    .unwrap();
                let mut operations = vec![TaskPermission::ContextView];
                if grant_records {
                    operations.push(TaskPermission::RecordGet);
                }
                let issued = f
                    .store
                    .issue_task_grant(
                        &f.credential,
                        &format!("record-grant-{grant_records}"),
                        &GrantDefinition {
                            task_id: task.id.clone(),
                            binding: binding.clone(),
                            operations,
                            expires_at: chrono::Utc::now().timestamp() + 120,
                        },
                    )
                    .unwrap();
                let invocation = f
                    .store
                    .authenticate_task_grant(issued.expose_secret(), &binding)
                    .unwrap();
                let TaskOutcome::Recall { sources, .. } = f
                    .store
                    .execute_task_invocation(
                        &invocation,
                        &TaskRequest::new(TaskCommand::Recall {
                            task_id: task.id.clone(),
                            after: None,
                            limit: 100,
                        }),
                    )
                    .unwrap()
                else {
                    panic!("grant recall")
                };
                assert_eq!(sources.iter().any(|s| s.record.is_some()), grant_records);
            }
        }
    }
}
#[tokio::test]
async fn record_context_deleted_bytes_invalidate_copied_selection_before_export() {
    let f = Fixture::new();
    let (_service, uuid) = context_record_fixture(&f).await;
    let Outcome::Session(original) = f
        .store
        .execute(
            &f.credential,
            &Operation::SessionGet {
                conversation_id: f.scope.conversation_id().to_string(),
                session_id: f.scope.session_id().to_string(),
            },
        )
        .unwrap()
    else {
        panic!("session")
    };
    let peer = f
        .store
        .issue(
            &original.participant_id,
            connect_store::Scope::operator(),
            chrono::Utc::now().timestamp() + 300,
        )
        .unwrap();
    let Outcome::Session(live) = f
        .store
        .execute(
            &f.credential,
            &Operation::SessionInvite {
                request_id: "context-live".into(),
                conversation_id: f.scope.conversation_id().to_string(),
                participant_id: original.participant_id,
                media: vec!["audio".into()],
            },
        )
        .unwrap()
    else {
        panic!("live")
    };
    f.store
        .execute(
            &peer.id,
            &Operation::SessionAccept {
                request_id: "context-live-accept".into(),
                conversation_id: live.conversation_id.clone(),
                session_id: live.id.clone(),
            },
        )
        .unwrap();
    let selected = f
        .store
        .context_for_session(&f.credential, &live.id)
        .unwrap();
    assert!(selected.items.iter().any(|i| i.source_id == uuid));
    f.store
        .begin_context_delivery(&f.credential, &live.id, "vapi", &selected)
        .unwrap();
    let handle = f
        .records()
        .list_for_session(&f.scope)
        .await
        .unwrap()
        .pop()
        .unwrap();
    f.records()
        .delete(
            &f.scope,
            &VconDeletion::new("context-delete".into(), handle).unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        f.store
            .begin_context_delivery(&f.credential, &live.id, "vapi", &selected)
            .unwrap_err()
            .code,
        "forbidden"
    );
    let current = context_view(&f, &f.credential);
    let item = current.items.iter().find(|i| i.source_id == uuid).unwrap();
    assert_eq!(
        item.record.as_ref().unwrap().state,
        connect_protocol::ContextRecordState::Deleted
    );
    assert!(item.body.is_empty());
}

fn context_membership(
    f: &Fixture,
    operation: &str,
    change: connect_protocol::membership::MembershipAction,
) -> connect_protocol::membership::MembershipOutcome {
    f.store
        .execute_membership(
            &f.credential,
            connect_protocol::membership::MembershipRequest {
                operation_id: operation.into(),
                conversation_id: f.scope.conversation_id().to_string(),
                expected_revision: f
                    .store
                    .membership_snapshot(&f.credential, f.scope.conversation_id().as_str())
                    .unwrap()
                    .revision,
                change,
            },
        )
        .unwrap()
}

#[tokio::test]
async fn record_context_shares_ready_and_future_records_with_original_revocation_and_no_new_mutation_rights(
) {
    use connect_protocol::membership::*;
    for mode in ["record", "session-ready", "session-pending"] {
        let f = Fixture::new();
        f.store
            .execute(
                &f.credential,
                &Operation::SessionCancel {
                    request_id: "context-cancel".into(),
                    conversation_id: f.scope.conversation_id().to_string(),
                    session_id: f.scope.session_id().to_string(),
                },
            )
            .unwrap();
        let uuid = f.store.pending_record_intents(1).unwrap().pop().unwrap();
        let service = crate::Service::new(f.store.clone());
        if mode != "session-pending" {
            assert_eq!(service.finalize_record_outcomes(1).await.unwrap().stored, 1);
        }
        let late = f.store.create_identity("late-reader").unwrap().id;
        let credential = f
            .store
            .issue(
                &late,
                connect_store::Scope {
                    operations: vec![
                        "context_view".into(),
                        "record_get".into(),
                        "record_delete".into(),
                    ],
                    recipients: vec!["*".into()],
                },
                chrono::Utc::now().timestamp() + 300,
            )
            .unwrap();
        let MembershipOutcome::Participant { participant, .. } = context_membership(
            &f,
            "late-add",
            MembershipAction::Add {
                identity_id: late,
                kind: connect_protocol::provisioning::IdentityKind::Human,
                role: "member".into(),
            },
        ) else {
            panic!("late participant")
        };
        assert!(!context_view(&f, &credential.id)
            .items
            .iter()
            .any(|i| i.record.is_some()));
        let MembershipOutcome::Sharing { sharing_id, .. } = context_membership(
            &f,
            "record-share",
            MembershipAction::ShareHistory {
                resources: vec![ContentRef {
                    kind: if mode == "record" {
                        ContentKind::Record
                    } else {
                        ContentKind::Session
                    },
                    id: if mode == "record" {
                        uuid.clone()
                    } else {
                        f.scope.session_id().to_string()
                    },
                }],
                participants: vec![participant.participant_id],
                provenance: "Explicit record context fixture consent".into(),
            },
        ) else {
            panic!("sharing")
        };
        if mode == "session-pending" {
            assert_eq!(service.finalize_record_outcomes(1).await.unwrap().stored, 1);
        }
        let view = context_view(&f, &credential.id);
        let item = view.items.iter().find(|i| i.source_id == uuid).unwrap();
        assert_eq!(
            item.record.as_ref().unwrap().state,
            connect_protocol::ContextRecordState::Available
        );
        if mode == "record" {
            use connect_protocol::record_mutations::{Action, Request};
            let request = Request {
                operation_id: "read-share-does-not-authorize-erasure".into(),
                conversation_id: f.scope.conversation_id().to_string(),
                session_id: f.scope.session_id().to_string(),
                uuid: uuid.clone(),
                expected_revision: 1,
                action: Action::Delete,
            };
            assert_eq!(
                f.store
                    .mutate_authorized_record(&credential.id, &request, |_, _, _| panic!(
                        "read share admitted erasure"
                    ))
                    .err()
                    .unwrap()
                    .code,
                "forbidden"
            );
        }
        context_membership(
            &f,
            "record-unshare",
            MembershipAction::RevokeHistoryShare { sharing_id },
        );
        assert!(!context_view(&f, &credential.id)
            .items
            .iter()
            .any(|i| i.record.is_some()));
    }
}

#[tokio::test]
async fn record_context_chat_custody_blocks_erasure_and_changed_record_starts_fresh_memory() {
    let f = Fixture::new();
    let (_service, uuid) = context_record_fixture(&f).await;
    let Outcome::Session(session) = f
        .store
        .execute(
            &f.credential,
            &Operation::SessionGet {
                conversation_id: f.scope.conversation_id().to_string(),
                session_id: f.scope.session_id().to_string(),
            },
        )
        .unwrap()
    else {
        panic!("session")
    };
    let bot_key = f
        .store
        .issue(
            &session.participant_id,
            connect_store::Scope::operator(),
            chrono::Utc::now().timestamp() + 300,
        )
        .unwrap();
    f.store
        .configure_route(
            &f.credential,
            &connect_store::Route {
                id: "record-context-chat".into(),
                identity_id: session.participant_id,
                credential_id: bot_key.id,
                conversation_id: f.scope.conversation_id().to_string(),
                connector: "vapi".into(),
                target: "12345678-1234-1234-1234-123456789012".into(),
                source: None,
                enabled: true,
                incoming_to: connect_protocol::messages::MessageAudience::all(),
            },
        )
        .unwrap();
    let role = f.store.acquire_provider_worker().unwrap();
    let send = |id: &str| {
        f.store
            .execute(
                &f.credential,
                &Operation::Send {
                    request_id: id.into(),
                    conversation_id: f.scope.conversation_id().to_string(),
                    message: connect_protocol::messages::MessageDraft::text_all(id),
                },
            )
            .unwrap();
        f.store.claim_provider_job().unwrap().unwrap()
    };
    let first = send("record-chat-one");
    assert!(first.context.as_ref().unwrap().contains(&uuid));
    let permit = f
        .store
        .admit_provider_dispatch(
            &role,
            &first,
            &connect_store::AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    permit.admit_effect().unwrap();
    drop(permit);
    f.store
        .finish_provider_job(&first, "completed", Some("record-memory-one"), None)
        .unwrap();
    let second = send("record-chat-two");
    assert_eq!(
        second.previous_chat_id.as_deref(),
        Some("record-memory-one")
    );
    let permit = f
        .store
        .admit_provider_dispatch(
            &role,
            &second,
            &connect_store::AuthorityWait::new(Duration::from_secs(1)),
        )
        .unwrap();
    permit.admit_effect().unwrap();
    let handle = f
        .records()
        .list_for_session(&f.scope)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let records = f.records();
    let scope = f.scope.clone();
    let mut deletion = tokio::spawn(async move {
        records
            .delete(
                &scope,
                &VconDeletion::new("record-chat-delete".into(), handle).unwrap(),
            )
            .await
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut deletion)
            .await
            .is_err()
    );
    assert!(permit.check().is_ok());
    drop(permit);
    assert!(deletion.await.unwrap().unwrap().is_some());
    f.store
        .finish_provider_job(&second, "completed", Some("record-memory-two"), None)
        .unwrap();
    let third = send("record-chat-three");
    assert!(third.previous_chat_id.is_none());
    let context: connect_protocol::ContextView =
        serde_json::from_str(third.context.as_ref().unwrap()).unwrap();
    assert_eq!(
        context
            .items
            .iter()
            .find(|i| i.source_id == uuid)
            .unwrap()
            .record
            .as_ref()
            .unwrap()
            .state,
        connect_protocol::ContextRecordState::Deleted
    );
}
