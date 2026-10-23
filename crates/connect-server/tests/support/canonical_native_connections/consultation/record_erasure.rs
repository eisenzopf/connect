//! Public record operations after actual isolated private duplex and Session end.
//! No synthetic record bytes, manifest, source audience or ready state is inserted.
use super::*;
use connect_client::{
    application::{ApplicationClient, ApplicationClientConfig},
    canonical::CanonicalRecordSelector,
};
use connect_protocol::record_mutations::{
    Action as MutationAction, Mode, Outcome as MutationOutcome, Request as Mutation, Source, Target,
};

pub(super) struct Replay {
    credential: String,
    redaction: Mutation,
    receipt: MutationOutcome,
    deletion: Mutation,
    deleted: MutationOutcome,
}
async fn reader(f: &Fixture, key: &str) -> CanonicalHttpClient {
    let mut config = canonical_http_config(f);
    config.credential = key.into();
    CanonicalHttpClient::open(config).await.unwrap()
}
async fn writer(f: &Fixture, key: &str) -> ApplicationClient {
    ApplicationClient::open(ApplicationClientConfig {
        endpoint: f.server.http_address.unwrap(),
        credential: key.into(),
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        timeout_ms: 8000,
    })
    .await
    .unwrap()
}
fn selector(uuid: &str) -> CanonicalRecordSelector {
    CanonicalRecordSelector {
        uuid: Some(uuid.into()),
        ..Default::default()
    }
}
fn key(f: &Fixture, identity: &str, erase: bool) -> connect_store::IssuedCredential {
    f.server
        .store
        .issue(
            identity,
            Scope {
                operations: if erase {
                    vec![
                        "record_get".into(),
                        "record_redact".into(),
                        "record_delete".into(),
                    ]
                } else {
                    vec!["record_get".into()]
                },
                recipients: vec!["*".into()],
            },
            chrono::Utc::now().timestamp() + 300,
        )
        .unwrap()
}
async fn record_context(
    f: &Fixture,
    original: &str,
    cid: &str,
    uuid: &str,
) -> connect_protocol::ContextItem {
    let actor = f.server.store.authenticate(original).unwrap();
    let credential = f
        .server
        .store
        .issue(
            &actor.identity_id,
            Scope {
                operations: vec!["context_view".into(), "record_get".into()],
                recipients: vec!["*".into()],
            },
            chrono::Utc::now().timestamp() + 300,
        )
        .unwrap();
    let mut client = writer(f, &credential.token).await;
    let connect_protocol::Outcome::ContextView(view) = client
        .execute(
            connect_protocol::Operation::ContextView {
                conversation_id: cid.into(),
                limit: 100,
            }
            .try_into()
            .unwrap(),
        )
        .await
        .unwrap()
    else {
        panic!("record context")
    };
    view.items
        .into_iter()
        .find(|i| i.source_id == uuid)
        .unwrap()
}
pub(super) async fn verify(
    f: &Fixture,
    cid: &str,
    consultant: &str,
    participant: &str,
    private_key: &str,
    remote_key: &str,
) -> Replay {
    let sid = "native-session";
    let mut private = reader(f, private_key).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let (uuid, revision) = loop {
        let page = private
            .conversation_records(cid, serde_json::from_value(json!({"limit":100})).unwrap())
            .await
            .unwrap();
        if let Some(record) = page.records.into_iter().find_map(|r| match r {
            connect_protocol::events::RecordAvailability::Ready {
                session_id,
                uuid,
                revision,
            } if session_id == sid => Some((uuid, revision)),
            _ => None,
        }) {
            break record;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "actual ended Session must produce a public outcome"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let original = private
        .fetch_record(cid, sid, selector(&uuid))
        .await
        .unwrap();
    assert!(original.body.contains("private-consultant"));
    assert!(original.body.contains("public-consultant"));
    let mut remote = reader(f, remote_key).await;
    let remote_before = remote
        .fetch_record(cid, sid, selector(&uuid))
        .await
        .unwrap();
    assert!(remote_before.body.contains(participant));
    assert!(!remote_before.body.contains("private-consultant"));
    assert!(!remote_before.body.contains("public-consultant"));
    assert!(remote_before.body.contains("codec-source"));
    let private_context = record_context(f, private_key, cid, &uuid).await;
    assert!(private_context.body.contains("private-consultant"));
    assert!(private_context.body.contains("public-consultant"));
    let remote_context = record_context(f, remote_key, cid, &uuid).await;
    assert!(!remote_context.body.contains("private-consultant"));
    assert!(!remote_context.body.contains("public-consultant"));
    assert!(remote_context.body.contains("codec-source"));
    // A tenant auditor is neither this Session's Participant nor an eraser.
    // Private metadata is selected only by the administrator's explicit role
    // policy, and still falls inside the credential's recipient ceiling.
    let auditor = f
        .server
        .store
        .create_identity("physical-record-auditor")
        .unwrap()
        .id;
    let auditor_key = key(f, &auditor, true);
    let mut admin = writer(f, &f.admin).await;
    let grant = |operation: &str, revision, private_sources| {
        connect_protocol::provisioning::ProvisioningCommand::RecordReadRole {
            operation_id: operation.into(),
            identity_id: auditor.clone(),
            role: "auditor".into(),
            private_sources,
            active: true,
            expected_revision: revision,
        }
    };
    admin
        .provision(grant("physical-shared-auditor", None, false))
        .await
        .unwrap();
    let mut audit = reader(f, &auditor_key.token).await;
    assert!(!audit
        .fetch_record(cid, sid, selector(&uuid))
        .await
        .unwrap()
        .body
        .contains("private-consultant"));
    admin
        .provision(grant("physical-private-auditor", Some(1), true))
        .await
        .unwrap();
    assert!(audit
        .fetch_record(cid, sid, selector(&uuid))
        .await
        .unwrap()
        .body
        .contains("private-consultant"));
    let narrow = f
        .server
        .store
        .issue(
            &auditor,
            Scope {
                operations: vec!["record_get".into()],
                recipients: vec![f.peer.identity_id.clone()],
            },
            chrono::Utc::now().timestamp() + 300,
        )
        .unwrap();
    assert!(!reader(f, &narrow.token)
        .await
        .fetch_record(cid, sid, selector(&uuid))
        .await
        .unwrap()
        .body
        .contains("private-consultant"));
    let tenant_erasure = Mutation {
        operation_id: "tenant-auditor-cannot-erase".into(),
        conversation_id: cid.into(),
        session_id: sid.into(),
        uuid: uuid.clone(),
        expected_revision: revision,
        action: MutationAction::Delete,
    };
    assert!(
        matches!(writer(f,&auditor_key.token).await.mutate_record(tenant_erasure).await,Err(ClientError::Application(e)) if e.code=="forbidden")
    );
    let mut primary = reader(f, &f.client.credential).await;
    let primary_before = primary
        .fetch_record(cid, sid, selector(&uuid))
        .await
        .unwrap();
    assert!(!primary_before.body.contains("private-consultant"));
    assert!(primary_before.body.contains("codec-target"));

    // The retained remote can see the shared consultant appearance. Erasing
    // that appearance also affects its actual private Connection dependencies.
    let remote_eraser = key(f, &f.peer.identity_id, true);
    let mut outsider = writer(f, &remote_eraser.token).await;
    let mut request = Mutation {
        operation_id: "physical-private-cascade-denied".into(),
        conversation_id: cid.into(),
        session_id: sid.into(),
        uuid: uuid.clone(),
        expected_revision: revision,
        action: MutationAction::Redact {
            sources: vec![Source {
                target: Target::Participant {
                    participant_id: participant.into(),
                    membership_epoch: 1,
                },
                mode: Mode::Metadata,
            }],
        },
    };
    assert!(
        matches!(outsider.mutate_record(request.clone()).await,Err(ClientError::Application(e)) if e.code=="forbidden")
    );
    request.operation_id = "physical-private-direct-denied".into();
    request.action = MutationAction::Redact {
        sources: vec![Source {
            target: Target::Connection {
                connection_id: "private-consultant".into(),
                generation: 1,
            },
            mode: Mode::Metadata,
        }],
    };
    assert!(
        matches!(outsider.mutate_record(request).await,Err(ClientError::Application(e)) if e.code=="forbidden")
    );
    assert_eq!(
        private
            .fetch_record(cid, sid, selector(&uuid))
            .await
            .unwrap()
            .content_hash,
        original.content_hash
    );

    let eraser = key(f, consultant, true);
    let mut authorized = writer(f, &eraser.token).await;
    let redaction = Mutation {
        operation_id: "physical-private-owner-erasure".into(),
        conversation_id: cid.into(),
        session_id: sid.into(),
        uuid: uuid.clone(),
        expected_revision: revision,
        action: MutationAction::Redact {
            sources: vec![Source {
                target: Target::Connection {
                    connection_id: "private-consultant".into(),
                    generation: 1,
                },
                mode: Mode::Metadata,
            }],
        },
    };
    let receipt = authorized.mutate_record(redaction.clone()).await.unwrap();
    assert_eq!(receipt.revision, revision + 1);
    assert!(authorized.mutate_record(redaction.clone()).await.unwrap() == receipt);
    let current = private
        .fetch_record(cid, sid, selector(&uuid))
        .await
        .unwrap();
    assert_ne!(current.content_hash, original.content_hash);
    assert!(!current.body.contains("private-consultant"));
    assert!(current.body.contains("public-consultant"));
    let changed_context = record_context(f, private_key, cid, &uuid).await;
    assert!(!changed_context.body.contains("private-consultant"));
    assert!(changed_context.body.contains("public-consultant"));
    assert_ne!(
        changed_context.record.as_ref().unwrap().content_hash,
        private_context.record.as_ref().unwrap().content_hash
    );
    assert!(
        matches!(private.fetch_record(cid,sid,CanonicalRecordSelector{content_hash:Some(original.content_hash),..selector(&uuid)}).await,Err(ClientError::Application(e)) if e.code=="conflict")
    );
    let retained = remote
        .fetch_record(cid, sid, selector(&uuid))
        .await
        .unwrap();
    assert!(!retained.body.contains("private-consultant"));
    assert!(retained.body.contains("codec-source"));

    // Authority is current even after source bytes disappear. A same-Identity
    // credential without the mutation permission cannot borrow the receipt.
    let narrowed = key(f, consultant, false);
    assert!(
        matches!(writer(f,&narrowed.token).await.mutate_record(redaction.clone()).await,Err(ClientError::Application(e)) if e.code=="forbidden")
    );
    let operator = f.server.store.authenticate(&f.admin).unwrap();
    f.server.store.revoke(&operator, &eraser.id).unwrap();
    assert!(authorized.mutate_record(redaction.clone()).await.is_err());
    let successor = key(f, consultant, true);
    let mut authorized = writer(f, &successor.token).await;
    assert!(authorized.mutate_record(redaction.clone()).await.unwrap() == receipt);

    let deletion = Mutation {
        operation_id: "physical-private-admin-deletion".into(),
        expected_revision: receipt.revision,
        action: MutationAction::Delete,
        ..redaction.clone()
    };
    assert!(
        matches!(outsider.mutate_record(deletion.clone()).await,Err(ClientError::Application(e)) if e.code=="forbidden")
    );
    let deleted = writer(f, &f.admin)
        .await
        .mutate_record(deletion.clone())
        .await
        .unwrap();
    assert_eq!(deleted.revision, revision + 2);
    assert!(authorized.mutate_record(redaction.clone()).await.unwrap() == receipt);
    for read in [&mut private, &mut remote, &mut primary] {
        assert!(
            matches!(read.fetch_record(cid,sid,selector(&uuid)).await,Err(ClientError::Application(e)) if e.code=="not-found")
        );
    }
    Replay {
        credential: successor.token,
        redaction,
        receipt,
        deletion,
        deleted,
    }
}
pub(super) async fn after_restart(f: &Fixture, cid: &str, replay: Replay) {
    assert!(
        writer(f, &replay.credential)
            .await
            .mutate_record(replay.redaction.clone())
            .await
            .unwrap()
            == replay.receipt
    );
    assert!(
        writer(f, &f.admin)
            .await
            .mutate_record(replay.deletion)
            .await
            .unwrap()
            == replay.deleted
    );
    let mut read = reader(f, &replay.credential).await;
    assert!(
        matches!(read.fetch_record(cid,"native-session",selector(&replay.redaction.uuid)).await,Err(ClientError::Application(e)) if e.code=="not-found")
    );
    let mut discovery = reader(f, &f.client.credential).await;
    let page = discovery
        .conversation_records(cid, serde_json::from_value(json!({"limit":100})).unwrap())
        .await
        .unwrap();
    assert!(page.records.iter().any(|r|matches!(r,connect_protocol::events::RecordAvailability::Unavailable{session_id} if session_id=="native-session")));
}
