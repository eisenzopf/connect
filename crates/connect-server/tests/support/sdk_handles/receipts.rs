//! Actual explicit application reads alongside signed SDK mutations and restart.
use super::*;
use connect_client::application::{
    ApplicationClient, ApplicationClientConfig, ApplicationOperation,
};
fn config(f: &Fixture, control: CanonicalClientConfig) -> sdk::ClientConfig {
    sdk::ClientConfig {
        application: Some(ApplicationClientConfig {
            endpoint: f.server.http_address.unwrap(),
            credential: control.credential.clone(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
            timeout_ms: 5000,
        }),
        control,
        tenant_id: "tnt_default".parse().unwrap(),
    }
}
async fn connect(f: &Fixture, control: CanonicalClientConfig) -> Client {
    Client::connect_configured(config(f, control))
        .await
        .unwrap()
}
async fn application(f: &Fixture, control: &CanonicalClientConfig) -> ApplicationClient {
    ApplicationClient::open(config(f, control.clone()).application.unwrap())
        .await
        .unwrap()
}
async fn execute(app: &mut ApplicationClient, operation: Operation) -> Outcome {
    app.execute(ApplicationOperation::try_from(operation).unwrap())
        .await
        .unwrap()
}
async fn setup(
    f: &Fixture,
) -> (
    Client,
    Client,
    CanonicalClientConfig,
    sdk::Conversation,
    sdk::ParticipantId,
) {
    let peer_config = super::super::canonical_native_connections::party_with_scope(
        f,
        &f.peer.identity_id,
        242,
        vec!["audio".into()],
        Scope::collaboration(vec!["*".into()]),
    );
    let owner = connect(f, f.client.clone()).await;
    let peer = connect(f, peer_config.clone()).await;
    let room = owner
        .create_conversation(
            vec![f.peer.identity_id.parse().unwrap()],
            ConversationPolicy::Persistent,
            mutation("sdk-receipts-create"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    let participant = room
        .refresh(options())
        .await
        .unwrap()
        .participants
        .into_iter()
        .find(|p| p.identity_id.as_str() == f.peer.identity_id)
        .unwrap()
        .id;
    room.send(
        "sdk-receipts-message".parse().unwrap(),
        NonEmpty {
            first: participant.clone(),
            rest: vec![],
        },
        "SDK receipts source".into(),
        mutation("sdk-receipts-send"),
    )
    .await
    .unwrap()
    .wait(options())
    .await
    .unwrap();
    (owner, peer, peer_config, room, participant)
}
#[tokio::test]
async fn public_sdk_configured_receipts_preserve_original_channel_facts_and_survive_restart() {
    for algorithm in ["EdDSA", "ES256"] {
        let mut f = Fixture::with_native_end_scope(algorithm, 16).await;
        let (owner, peer, mut peer_config, room, participant) = setup(&f).await;
        let message = "sdk-receipts-message".parse::<sdk::MessageId>().unwrap();
        let mut app = application(&f, &peer_config).await;
        execute(
            &mut app,
            Operation::MessageReceipt {
                request_id: "sdk-receipts-delivered".into(),
                conversation_id: room.id().as_str().into(),
                message_id: message.as_str().into(),
                state: connect_protocol::receipts::ClientReceiptState::Delivered,
            },
        )
        .await;
        let delivered = room.receipts(message.clone(), options()).await.unwrap();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].participant_id, participant);
        assert_eq!(delivered[0].channel, "native");
        assert_eq!(delivered[0].state, sdk::DeliveryState::Delivered);
        assert_eq!(delivered[0].provenance, "client_reported");
        execute(
            &mut app,
            Operation::ReceiptPreferencesSet {
                request_id: "sdk-receipts-read-on".into(),
                conversation_id: room.id().as_str().into(),
                delivery: true,
                read: true,
            },
        )
        .await;
        let v = json!({"v":2,"id":"local-read","ts":chrono::Utc::now(),"type":"message.read","cid":room.id().as_str(),"operation_id":"sdk-receipts-read","payload":{"msg_id":message.as_str(),"read_at":chrono::Utc::now()}});
        peer.raw()
            .request(peer.raw().validate(v).unwrap(), options())
            .await
            .unwrap();
        let observed = room.receipts(message.clone(), options()).await.unwrap();
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0].state, sdk::DeliveryState::Read);
        assert_eq!(observed[0].provenance, "client_reported");
        assert_eq!(observed[0].membership_epoch, delivered[0].membership_epoch);
        let credential = f
            .server
            .store
            .authenticate(&f.client.credential)
            .unwrap()
            .credential_id;
        let Outcome::MessageReceipts(actual) = f
            .server
            .store
            .execute(
                &credential,
                &Operation::MessageReceipts {
                    conversation_id: room.id().as_str().into(),
                    message_id: message.as_str().into(),
                    after: None,
                    limit: 100,
                },
            )
            .unwrap()
        else {
            panic!("receipt outcome")
        };
        assert_eq!(actual.receipts.len(), 1);
        let actual = &actual.receipts[0];
        let observed = &observed[0];
        assert_eq!(observed.delivery_id, actual.delivery_id);
        assert_eq!(observed.state, actual.state);
        assert_eq!(observed.provenance, actual.provenance);
        assert_eq!(observed.observed_at, actual.observed_at);
        let cid = room.id().clone();
        owner.close(options()).await.unwrap();
        peer.close(options()).await.unwrap();
        drop(app);
        f.server.shutdown().await.unwrap();
        f.server = RunningCanonicalServer::start(f.config.clone())
            .await
            .unwrap();
        f.client.endpoint = f.server.address;
        peer_config.endpoint = f.server.address;
        let configured = config(&f, f.client.clone());
        let path = f
            .config
            .database
            .parent()
            .unwrap()
            .join("sdk-receipts.toml");
        std::fs::write(&path, toml::to_string(&configured).unwrap()).unwrap();
        let fresh = Client::connect_configured(sdk::ClientConfig::load(&path).unwrap())
            .await
            .unwrap();
        let restored = fresh
            .conversation_by_id(cid)
            .receipts(message, options())
            .await
            .unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].delivery_id, observed.delivery_id);
        assert_eq!(restored[0].state, observed.state);
        assert_eq!(restored[0].observed_at, observed.observed_at);
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM messages WHERE id='sdk-receipts-message'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            1
        );
        fresh.close(options()).await.unwrap();
        f.server.shutdown().await.unwrap();
        eprintln!("SDK configured receipts {algorithm}: original epoch/channel/provenance, explicit client report, current signed read, same facts after restart/TOML load, one Message and joined owners");
    }
}
#[tokio::test]
async fn public_sdk_receipts_follow_current_preferences_and_membership_epochs() {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 16).await;
        let (owner, peer, peer_config, room, participant) = setup(&f).await;
        let message = "sdk-receipts-message".parse::<sdk::MessageId>().unwrap();
        let mut app = application(&f, &peer_config).await;
        assert_eq!(
            room.receipts(message.clone(), options())
                .await
                .unwrap()
                .len(),
            1
        );
        for (id, delivery) in [("sdk-receipts-off", false), ("sdk-receipts-on", true)] {
            execute(
                &mut app,
                Operation::ReceiptPreferencesSet {
                    request_id: id.into(),
                    conversation_id: room.id().as_str().into(),
                    delivery,
                    read: false,
                },
            )
            .await;
            assert_eq!(
                room.receipts(message.clone(), options())
                    .await
                    .unwrap()
                    .len(),
                usize::from(delivery)
            );
        }
        let current = room.refresh(options()).await.unwrap();
        room.participant_by_id(participant.clone())
            .remove(current.revision, mutation("sdk-receipts-remove"))
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        assert!(room
            .receipts(message.clone(), options())
            .await
            .unwrap()
            .is_empty());
        let peer_room = peer.conversation_by_id(room.id().clone());
        let e = peer_room
            .receipts(message.clone(), options())
            .await
            .unwrap_err();
        assert_eq!(
            (e.kind, e.certainty),
            (sdk::ErrorKind::Authentication, sdk::Certainty::Refused)
        );
        let current = room.refresh(options()).await.unwrap();
        let replacement = room
            .add_participant(
                f.peer.identity_id.parse().unwrap(),
                sdk::IdentityKind::Human,
                "member".into(),
                current.revision,
                mutation("sdk-receipts-readd"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        let current = room.refresh(options()).await.unwrap();
        let replacement = current
            .participants
            .iter()
            .find(|p| p.id == *replacement.id())
            .unwrap();
        assert!(replacement.id != participant || replacement.membership_epoch > 1);
        assert!(peer_room
            .receipts(message.clone(), options())
            .await
            .is_err());
        assert!(room.receipts(message, options()).await.unwrap().is_empty());
        peer.close(options()).await.unwrap();
        owner.close(options()).await.unwrap();
        drop(app);
        f.server.shutdown().await.unwrap();
        eprintln!("SDK receipt authority {algorithm}: current preferences suppress/restore facts, removal refuses reader and hides old epoch, rejoin cannot inherit old source evidence");
    }
}
