//! Same logical AI, fresh transports, and a cited historical outcome after restart.
use super::*;
use connect_client::application::{ApplicationClient, ApplicationOperation};
use connect_protocol::{ContextView, Operation, Outcome};

async fn context(f: &Fixture, key: &str, cid: &str) -> ContextView {
    let mut config = canonical_http_config(f);
    config.credential = key.into();
    let mut client = ApplicationClient::open(config).await.unwrap();
    let request = ApplicationOperation::try_from(Operation::ContextView {
        conversation_id: cid.into(),
        limit: 100,
    })
    .unwrap();
    let Outcome::ContextView(view) = client.execute(request).await.unwrap() else {
        panic!("context read returned another outcome")
    };
    view
}

async fn participant(
    f: &Fixture,
    key: &str,
    cid: &str,
    identity: &str,
) -> connect_protocol::membership::MemberView {
    let mut config = canonical_http_config(f);
    config.credential = key.into();
    let mut client = ApplicationClient::open(config).await.unwrap();
    let request = ApplicationOperation::try_from(Operation::MembershipSnapshot {
        conversation_id: cid.into(),
    })
    .unwrap();
    let Outcome::MembershipSnapshot(snapshot) = client.execute(request).await.unwrap() else {
        panic!("membership read returned another outcome")
    };
    snapshot
        .participants
        .into_iter()
        .find(|participant| participant.identity_id == identity)
        .unwrap()
}

pub(super) async fn verify(
    mut f: Fixture,
    public: CanonicalClient,
    clients: Vec<CanonicalClient>,
    mut ai: CanonicalClientConfig,
    key: &str,
    cid: &str,
    returned_revision: u64,
) {
    let before = context(&f, key, cid).await;
    let source = before
        .items
        .iter()
        .filter(|item| item.kind == "session_control" && item.source_id == "native-session")
        .find(|item| {
            serde_json::from_str::<connect_protocol::session_control::SessionView>(&item.body)
                .is_ok_and(|session| session.topology_revision == returned_revision)
        })
        .expect("actual completed return must have an attributed context outcome")
        .clone();
    assert_eq!(source.actor_id, f.client.identity_id);
    assert!(source.sequence > 0 && source.sequence <= before.watermark);
    let session: connect_protocol::session_control::SessionView =
        serde_json::from_str(&source.body).unwrap();
    assert_eq!(session.conversation_id, cid);
    assert_eq!(session.id, source.source_id);
    let original_epoch = f.server.store.runtime_epoch().to_owned();
    let original_participant = participant(&f, key, cid, &ai.identity_id).await;
    public.shutdown().await.unwrap();
    for client in clients {
        client.shutdown().await.unwrap();
    }
    // The init fixture's original remote key only permits the original owner.
    // Configure a current read credential for this now three-party fixture;
    // the original narrow key must not be used to bypass its recipient scope.
    let store = f.server.store.clone();
    let remote_identity = f.peer.identity_id.clone();
    let remote_key = tokio::task::spawn_blocking(move || {
        store.issue(
            &remote_identity,
            Scope::collaboration(vec!["*".into()]),
            chrono::Utc::now().timestamp() + 600,
        )
    })
    .await
    .unwrap()
    .unwrap();
    f.server.shutdown().await.unwrap();
    f.server = RunningCanonicalServer::start(f.config.clone())
        .await
        .unwrap();
    assert_ne!(original_epoch, f.server.store.runtime_epoch());
    f.client.endpoint = f.server.address;
    ai.endpoint = f.server.address;
    let after = context(&f, key, cid).await;
    assert!(after.watermark >= before.watermark);
    assert_eq!(
        after
            .items
            .iter()
            .find(|item| item.sequence == source.sequence),
        Some(&source),
        "restart must retain the exact historical source, actor and body"
    );
    let current_participant = participant(&f, key, cid, &ai.identity_id).await;
    assert_eq!(current_participant, original_participant);
    let mut worker = CanonicalClient::open(ai).await.unwrap();
    // The source describes a completed historical return, not a claim that its
    // old physical call survived this host restart. No private transcript is
    // copied into the later all-audience message.
    let body = format!(
        "The earlier call returned to its owner at Session revision {}. Historical evidence: {}:{} by {}. Voice has ended; this is the messaging follow-up.",
        returned_revision, source.source_id, source.sequence, source.actor_id
    );
    let draft = connect_client::messages::MessageDraft::text_all(&body);
    let first = worker
        .send_message(
            "restart-followup-send",
            cid,
            "restart-followup-message",
            draft.clone(),
        )
        .await
        .unwrap();
    let replay = worker
        .send_message(
            "restart-followup-send",
            cid,
            "restart-followup-message",
            draft,
        )
        .await
        .unwrap();
    assert_eq!(first, replay);
    for token in [&f.client.credential, &remote_key.token] {
        let mut config = canonical_http_config(&f);
        config.credential = token.clone();
        let mut reader = CanonicalHttpClient::open(config).await.unwrap();
        let history = reader
            .history(cid, serde_json::from_value(json!({"limit":100})).unwrap())
            .await
            .unwrap();
        let sent = history
            .messages
            .iter()
            .filter(|message| message.msg_id.as_str() == "restart-followup-message")
            .collect::<Vec<_>>();
        assert_eq!(sent.len(), 1, "logical replay created another follow-up");
        assert_eq!(
            sent[0].r#from.as_ref().map(|id| id.as_str()),
            Some(original_participant.participant_id.as_str())
        );
        assert_eq!(sent[0].body, body);
        assert!(!sent[0].body.contains("private provider transcript marker"));
    }
    worker.shutdown().await.unwrap();
    f.server.shutdown().await.unwrap();
    println!("actual assistant restart follow-up: same logical AI/Participant; fresh signed transport and host epoch; exact historical Session outcome cited; stable message replay; independent owner and remote histories; no repeated provider creation/export");
}
