//! Actual native AV lifecycle/authority failures, independent of codec claims.
#![allow(clippy::unwrap_used)]
#[path = "gate5_support/video.rs"]
mod video;
use connect_client::{ClientConfig, ClientError, ConnectClient, NativeVideoSession};
use connect_protocol::{Operation, Outcome};
use connect_server::{config::ServerConfig, runtime::RunningServer};
use std::time::{Duration, Instant};

async fn pair(f: &mut video::Fixture) -> (NativeVideoSession, NativeVideoSession) {
    let left = f
        .left
        .open_native_video(&f.conversation, &f.session, &f.right_id)
        .await
        .unwrap();
    let right = f
        .right
        .open_native_video(&f.conversation, &f.session, &f.left_id)
        .await
        .unwrap();
    f.left
        .subscribe_native_video_participant(&left, &f.right_id)
        .await
        .unwrap();
    f.right
        .subscribe_native_video_participant(&right, &f.left_id)
        .await
        .unwrap();
    (left, right)
}

async fn terminal(session: &mut NativeVideoSession, phase: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match session.receive(Duration::from_millis(100)).await {
                Ok(_) | Err(ClientError::Timeout) => {}
                Err(ClientError::MediaEnded | ClientError::Transport) => break,
                Err(error) => panic!("Unexpected media failure: {error}"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "AV terminal deadline exceeded: {phase}; is_ended={}",
            session.is_ended()
        )
    });
    assert!(session.send_pcmu(&[0xff; 160]).await.is_err());
    assert!(session.send_vp8(&[0; 100]).await.is_err());
}

#[tokio::test]
async fn one_native_end_terminates_the_counterpart_and_allows_a_new_live_attachment() {
    let mut f = video::Fixture::new().await;
    let (mut left, mut right) = pair(&mut f).await;
    let start = Instant::now();
    left.end(&mut f.left).await.unwrap();
    // A sending/muted participant must stop even if its application never polls
    // receive(); the SDK owns terminal control observation independently.
    tokio::time::timeout(Duration::from_secs(5), async {
        while !right.is_ended() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    terminal(&mut right, "first counterpart after left end").await;
    assert!(start.elapsed() < Duration::from_secs(5));
    // Ending a connection preserves the accepted session and conversation.
    f.right
        .send(
            "after-native-end",
            &f.conversation,
            "Context continues after media",
        )
        .await
        .unwrap();
    assert!(f
        .right
        .history(&f.conversation, None, 100)
        .await
        .unwrap()
        .messages
        .iter()
        .any(|message| message.body == "Context continues after media"));
    drop(left);
    drop(right);
    let (mut replacement, mut counterpart) = pair(&mut f).await;
    replacement.end(&mut f.left).await.unwrap();
    terminal(&mut counterpart, "replacement counterpart after left end").await;
    f.left.close().await;
    f.right.close().await;
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn current_credential_revocation_ends_av_and_cannot_recreate_media_on_replay() {
    let mut f = video::Fixture::new().await;
    let (mut left, mut right) = pair(&mut f).await;
    let directory = f.temporary.path().join("state");
    let operator = ClientConfig::load(&directory.join("operator.toml")).unwrap();
    let actor = f.server.store.authenticate(&operator.credential).unwrap();
    let revoked = f.server.store.authenticate(&f.left_token).unwrap();
    f.server
        .store
        .revoke(&actor, &revoked.credential_id)
        .unwrap();
    tokio::join!(
        terminal(&mut left, "revoked origin"),
        terminal(&mut right, "revoked counterpart")
    );
    assert!(f
        .left
        .open_native_video(&f.conversation, &f.session, &f.right_id)
        .await
        .is_err());
    // The counterpart's ordinary current authority and history remain usable.
    assert_eq!(
        f.right
            .session_get(&f.conversation, &f.session)
            .await
            .unwrap()
            .conversation_id,
        f.conversation
    );
    f.left.close().await;
    f.right.close().await;
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn durable_session_end_stops_both_streams_and_denies_a_stale_attachment() {
    let mut f = video::Fixture::new().await;
    let (mut left, mut right) = pair(&mut f).await;
    f.left
        .session_end("end-durable-video", &f.conversation, &f.session, None)
        .await
        .unwrap();
    tokio::join!(
        terminal(&mut left, "ended origin"),
        terminal(&mut right, "ended counterpart")
    );
    let error = f
        .right
        .open_native_video(&f.conversation, &f.session, &f.left_id)
        .await
        .err()
        .unwrap();
    assert!(matches!(error, ClientError::Application(ref error) if error.code == "forbidden"));
    f.right
        .send(
            "after-durable-end",
            &f.conversation,
            "Conversation remains open",
        )
        .await
        .unwrap();
    f.left.close().await;
    f.right.close().await;
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn server_restart_reconciles_live_video_without_resurrecting_media_or_losing_context() {
    let mut f = video::Fixture::new().await;
    let (mut left, mut right) = pair(&mut f).await;
    f.left
        .send(
            "before-video-restart",
            &f.conversation,
            "Persistent cross-channel context",
        )
        .await
        .unwrap();
    f.server.shutdown().await.unwrap();
    tokio::join!(
        terminal(&mut left, "restart origin"),
        terminal(&mut right, "restart counterpart")
    );
    f.left.close().await;
    f.right.close().await;
    let directory = f.temporary.path().join("state");
    let config = ServerConfig::load(&directory.join("server.toml")).unwrap();
    let server = RunningServer::start(config).await.unwrap();
    let mut fresh = ClientConfig::load(&directory.join("agent.toml")).unwrap();
    fresh.endpoint = server.address;
    fresh.credential = f.right_token;
    let credential = server.store.authenticate(&fresh.credential).unwrap();
    fresh.credential_id = credential.credential_id.clone();
    let mut client = ConnectClient::open(fresh).await.unwrap();
    assert!(client
        .history(&f.conversation, None, 100)
        .await
        .unwrap()
        .messages
        .iter()
        .any(|message| message.body == "Persistent cross-channel context"));
    let current = match server
        .store
        .execute(
            &credential.credential_id,
            &Operation::SessionGet {
                conversation_id: f.conversation.clone(),
                session_id: f.session.clone(),
            },
        )
        .unwrap()
    {
        Outcome::Session(session) => session,
        _ => panic!("Session outcome"),
    };
    assert_eq!(current.conversation_id, f.conversation);
    // An accepted durable intent is not an automatically resumed socket.
    assert_eq!(current.state, "accepted");
    client.close().await;
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn physical_native_peer_loss_is_transport_failure_and_terminates_counterpart() {
    let mut f = video::Fixture::new().await;
    let (mut left, mut right) = pair(&mut f).await;
    f.left.close().await;
    assert!(matches!(
        left.receive(Duration::from_secs(1)).await,
        Err(ClientError::Transport)
    ));
    terminal(&mut right, "counterpart after physical peer loss").await;
    let current = f
        .right
        .session_get(&f.conversation, &f.session)
        .await
        .unwrap();
    assert!(matches!(current.state.as_str(), "accepted" | "ready"));
    f.right
        .send(
            "after-native-peer-loss",
            &f.conversation,
            "Context survives transport loss",
        )
        .await
        .unwrap();
    assert!(f
        .right
        .history(&f.conversation, None, 100)
        .await
        .unwrap()
        .messages
        .iter()
        .any(|message| message.body == "Context survives transport loss"));
    f.right.close().await;
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn edited_display_metadata_cannot_retarget_teardown_or_lose_owned_routes() {
    let mut f = video::Fixture::new().await;
    let (mut left, mut right) = pair(&mut f).await;
    // Public display snapshots never replace the private negotiated binding.
    left.conversation_id = "edited-display-conversation".into();
    left.session_id = "edited-display-session".into();
    left.connection_id = "edited-display-connection".into();
    left.end(&mut f.left).await.unwrap();
    terminal(
        &mut right,
        "original counterpart after display metadata edit",
    )
    .await;
    assert_eq!(
        f.right
            .session_get(&f.conversation, &f.session)
            .await
            .unwrap()
            .state,
        "accepted"
    );
    let (mut replacement, mut counterpart) = pair(&mut f).await;
    replacement.end(&mut f.left).await.unwrap();
    terminal(&mut counterpart, "replacement after display metadata edit").await;
    f.left.close().await;
    f.right.close().await;
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn another_client_cannot_cancel_or_retarget_the_owned_video_handle() {
    let mut f = video::Fixture::new().await;
    let (mut left, mut right) = pair(&mut f).await;
    assert!(matches!(
        left.end(&mut f.right).await,
        Err(ClientError::Protocol)
    ));
    assert!(!left.is_ended());
    left.end(&mut f.left).await.unwrap();
    terminal(&mut right, "counterpart after correct client teardown").await;
    f.left.close().await;
    f.right.close().await;
    f.server.shutdown().await.unwrap();
}
