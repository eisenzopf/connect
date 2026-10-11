//! Full retained-session ceiling with actual callback expiry and capacity reuse.
#![allow(clippy::unwrap_used)]
#[path = "support/vapi_call_setup.rs"]
mod call_setup;
#[path = "support/native_grants.rs"]
mod grants;
#[path = "support/native_media.rs"]
mod media;
#[path = "support/retirement_metrics.rs"]
mod metrics;
#[path = "support/native_tasks.rs"]
mod native_tasks;
#[path = "support/listener_retirement.rs"]
mod retirement;
mod support;
use connect_client::{tasks::*, ConnectClient};
use connect_connectors::Vapi;
use connect_mcp::MAX_MCP_LISTENERS;
use std::{path::Path, sync::Arc, time::Duration};

const SESSIONS: usize = 10_000;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Full real-clock retirement profile; scripts/check-voice-retirement.py owns its private report and deadline"]
async fn retired_setups_reuse_capacity_through_the_full_session_limit() {
    assert_eq!(
        (std::env::consts::OS, std::env::consts::ARCH),
        ("macos", "aarch64")
    );
    assert_eq!(
        connect_protocol::Limits::default().sessions,
        SESSIONS as u64
    );
    assert_eq!(MAX_MCP_LISTENERS, 32);
    let directory = std::path::PathBuf::from(
        std::env::var("CONNECT_VOICE_RETIREMENT_DIR")
            .expect("Use scripts/check-voice-retirement.py to retain private profile evidence"),
    );
    let api = wiremock::MockServer::start().await;
    let url = api.uri();
    let provider =
        tokio::task::spawn_blocking(move || Arc::new(Vapi::new("local-fixture", &url).unwrap()))
            .await
            .unwrap();
    let mut f = media::Fixture::start().await;
    call_setup::route(&f);
    let mut metrics = metrics::Metrics::new(&directory);
    let (session, path) = tokio::time::timeout(
        Duration::from_secs(2700),
        exercise(&mut f, &directory, &provider, &api, &mut metrics),
    )
    .await
    .expect("Full retirement profile exceeded its 45-minute execution budget");
    let intent = std::fs::read(path.join("intent.json")).unwrap();
    f.h = f.h.restart().await;
    metrics
        .capture(f.h.server.store.path(), SESSIONS, 0, "restarted")
        .await;
    assert!(f
        .h
        .server
        .store
        .vapi_session_requires_task_setup(&session)
        .unwrap());
    assert!(f
        .h
        .server
        .prepare_vapi_task_session(
            &session,
            call_setup::definition(&f),
            &directory.join("replacement"),
            provider.clone()
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("already has"));
    assert_eq!(std::fs::read(path.join("intent.json")).unwrap(), intent);
    f.h.owner.negotiate_task(&f.task.id).await.unwrap();
    f.h.server
        .store
        .backup(&directory.join("retained.sqlite3"))
        .unwrap();
    f.close().await;
    assert!(api.received_requests().await.unwrap().is_empty());
    metrics.finish();
}

async fn exercise(
    f: &mut media::Fixture,
    directory: &Path,
    provider: &Arc<Vapi>,
    api: &wiremock::MockServer,
    metrics: &mut metrics::Metrics,
) -> (String, std::path::PathBuf) {
    metrics
        .capture(f.h.server.store.path(), 0, 0, "baseline")
        .await;
    let mut last = None;
    for start in (0..SESSIONS).step_by(MAX_MCP_LISTENERS) {
        let count = (SESSIONS - start).min(MAX_MCP_LISTENERS);
        let task =
            f.h.owner
                .task(TaskCommand::Create {
                    request_id: format!("retirement-task-{start}"),
                    definition: TaskDefinition {
                        conversation_id: f.h.cid.clone(),
                        scope: TaskScope {
                            operations: media::Fixture::permissions(),
                            recipients: vec![f.h.peer_id.clone()],
                            sms_enrollments: vec![],
                        },
                        expires_at: chrono::Utc::now().timestamp() + 300,
                        initiating_event_id: None,
                    },
                })
                .await
                .unwrap();
        f.task = native_tasks::record(task);
        let (_, config) = f
            .grant(
                &format!("retirement-native-{start}"),
                120,
                media::Fixture::permissions(),
            )
            .await;
        let mut client = ConnectClient::open(config).await.unwrap();
        let mut sessions = Vec::new();
        for index in start..start + count {
            sessions.push(
                f.session(&mut client, &format!("retirement-session-{index}"))
                    .await,
            );
        }
        let expires = chrono::Utc::now().timestamp() + 5;
        let mut endpoints = Vec::new();
        for (offset, session) in sessions.into_iter().enumerate() {
            let path = directory.join(format!("setup-{}", start + offset));
            let mut definition = call_setup::definition(f);
            definition.request_id = format!("retirement-grant-{}", start + offset);
            definition.expires_at = expires;
            endpoints.push(
                f.h.server
                    .prepare_vapi_task_session(&session.id, definition, &path, provider.clone())
                    .await
                    .unwrap(),
            );
            last = Some((session.id, path));
        }
        for endpoint in &endpoints {
            tokio::net::TcpStream::connect(endpoint).await.unwrap_or_else(|error|
                panic!("listener probe failed: {error:?}; endpoint={endpoint}; now={}; expires={expires}", chrono::Utc::now().timestamp()));
        }
        metrics
            .capture(f.h.server.store.path(), start + count, count, "ready")
            .await;
        retirement::await_closed(&endpoints, expires).await;
        metrics
            .capture(f.h.server.store.path(), start + count, 0, "retired")
            .await;
        assert!(api.received_requests().await.unwrap().is_empty());
        client.close().await;
        println!(
            "Retired {} / {SESSIONS} setups; callback capacity reusable",
            start + count
        );
    }
    let error =
        f.h.owner
            .session_invite(
                "over-retained-session-capacity",
                &f.h.cid,
                &f.h.peer_id,
                vec!["audio".into()],
            )
            .await
            .unwrap_err();
    assert_eq!(grants::code(error), "capacity-exceeded");
    last.unwrap()
}
