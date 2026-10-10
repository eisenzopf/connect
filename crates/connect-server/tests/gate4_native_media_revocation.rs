#![allow(clippy::unwrap_used)]
mod gate3_support;
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_media.rs"]
mod media;
#[path = "support/native_tasks.rs"]
mod native_tasks;
mod support;
use connect_client::{tasks::*, ConnectClient};
use connect_protocol::{Operation, Outcome};
use gate3_support::wire_audio::WireAudioPeer;
use media::Fixture;
use std::time::Duration;

#[tokio::test]
async fn current_grant_revoke_cancel_and_expiry_close_actual_task_audio_peers() {
    for cause in ["revoke", "cancel", "expiry", "parent", "session-end"] {
        let mut f = Fixture::start().await;
        // Prepare the session under a durable, long-lived task invocation.
        let (_, setup_config) = f.grant("setup", 120, Fixture::permissions()).await;
        let mut setup = ConnectClient::open(setup_config).await.unwrap();
        let session = f.session(&mut setup, "invite").await;
        let ttl = if cause == "expiry" { 4 } else { 120 };
        let (grant, config) = f.grant("audio", ttl, Fixture::permissions()).await;
        let wire = WireAudioPeer::open(
            &config,
            &f.h.cid,
            &session.id,
            &f.h.peer_id,
            "task-publisher",
        )
        .await;
        let mut audio =
            f.h.peer
                .open_native_audio(&f.h.cid, &session.id, &f.h.owner_config.identity_id)
                .await
                .unwrap();
        f.h.peer
            .subscribe_native_audio(&audio, "task-publisher")
            .await
            .unwrap();
        f.binding(&grant.grant.id, "ready").await;
        wire.send(0, wire.local_id, 0x42);
        tokio::time::sleep(Duration::from_millis(25)).await;
        wire.send(1, wire.local_id, 0x42);
        assert_eq!(
            audio.receive(Duration::from_secs(2)).await.unwrap().payload,
            vec![0x42; 160]
        );
        match cause {
            "revoke" => f.revoke(&grant.grant.id).await,
            "cancel" => {
                f.h.owner
                    .task(TaskCommand::Cancel {
                        request_id: "cancel".into(),
                        task_id: f.task.id.clone(),
                        reason: None,
                    })
                    .await
                    .unwrap();
            }
            "parent" => {
                let actor =
                    f.h.server
                        .store
                        .revalidate(&f.h.owner_config.credential_id)
                        .unwrap();
                f.h.server
                    .store
                    .revoke(&actor, &f.h.owner_config.credential_id)
                    .unwrap();
            }
            "expiry" => {}
            "session-end" => {
                f.h.owner
                    .session_end("end-leg", &f.h.cid, &session.id, None)
                    .await
                    .unwrap();
            }
            _ => unreachable!(),
        }
        tokio::time::timeout(Duration::from_secs(6), wire.peer.connection.closed())
            .await
            .unwrap();
        f.binding(&grant.grant.id, "ended").await;
        assert!(
            wire.peer
                .connection
                .send_datagram(vec![0; 8].into())
                .is_err(),
            "{cause}"
        );
        if cause == "session-end" {
            let mut continued = ConnectClient::open(config).await.unwrap();
            continued.negotiate_task(&f.task.id).await.unwrap();
            let denied = continued
                .open_native_audio(&f.h.cid, &session.id, &f.h.peer_id)
                .await;
            assert_eq!(grants::code(denied.err().unwrap()), "forbidden");
            let result = continued
                .task(grants::wrapped(
                    &f.task.id,
                    "after-leg",
                    Operation::Send {
                        request_id: "after-leg".into(),
                        conversation_id: f.h.cid.clone(),
                        body: "The leg ended; the task and conversation continue".into(),
                    },
                ))
                .await
                .unwrap();
            assert!(
                matches!(result,TaskOutcome::Tool(TaskToolResult::Completed {outcome,..}) if matches!(*outcome,Outcome::Accepted {..}))
            );
            continued.close().await;
        } else {
            assert!(ConnectClient::open(config).await.is_err(), "{cause}");
            f.h.peer.close_native_audio(&audio).await.unwrap();
        }
        setup.close().await;
        drop(wire);
        f.close().await;
    }
}
