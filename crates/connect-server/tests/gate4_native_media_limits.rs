#![allow(clippy::unwrap_used)]
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_media.rs"]
mod media;
#[path = "support/native_tasks.rs"]
mod native_tasks;
mod support;
use connect_client::{tasks::TaskPermission, ConnectClient};
use media::Fixture;

#[tokio::test]
async fn read_only_and_unqualified_provider_routes_fail_before_creating_media_or_context() {
    let mut f = Fixture::start().await;
    let (_, setup_config) = f.grant("setup", 120, Fixture::permissions()).await;
    let mut setup = ConnectClient::open(setup_config).await.unwrap();
    let session = f.session(&mut setup, "invite").await;
    let (_, read_config) = f.grant("read", 120, vec![TaskPermission::SessionGet]).await;
    let mut read = ConnectClient::open(read_config).await.unwrap();
    let before = connect_store::Store::inspect_snapshot(f.h.server.store.path()).unwrap();
    let denied = read
        .open_native_audio(&f.h.cid, &session.id, &f.h.peer_id)
        .await;
    assert_eq!(grants::code(denied.err().unwrap()), "forbidden");
    assert_eq!(
        connect_store::Store::inspect_snapshot(f.h.server.store.path()).unwrap(),
        before
    );
    f.h.server
        .store
        .configure_voice_route(
            &f.h.owner_config.credential_id,
            &connect_store::VoiceRoute {
                id: "voice".into(),
                conversation_id: f.h.cid.clone(),
                identity_id: f.h.peer_id.clone(),
                assistant_id: "12345678-1234-1234-1234-123456789012".into(),
                phone_number_id: "12345678-1234-1234-1234-123456789013".into(),
                destinations: vec!["+15555550101".into()],
                enabled: true,
            },
        )
        .unwrap();
    let before = connect_store::Store::inspect_snapshot(f.h.server.store.path()).unwrap();
    let rejected = setup
        .open_native_audio(&f.h.cid, &session.id, &f.h.peer_id)
        .await;
    assert_eq!(
        grants::code(rejected.err().unwrap()),
        "unsupported-media-profile"
    );
    assert_eq!(
        connect_store::Store::inspect_snapshot(f.h.server.store.path()).unwrap(),
        before
    );
    read.close().await;
    setup.close().await;
    f.close().await;
}
