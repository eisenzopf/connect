//! Actual local native and HTTP views; connection rows are isolated adapter
//! fixtures, not physical media or provider normalization qualification.
#![allow(clippy::unwrap_used)]
#[path = "support/agent_mcp_fixture.rs"]
mod agent;
use connect_client::{events::*, ConnectClient};
use connect_protocol::{Operation, Outcome, Response};
use connect_store::Scope;
use serde_json::{json, Value};
async fn post(http: &reqwest::Client, base: &str, token: &str, operation: &Operation) -> Value {
    http.post(format!("{base}/v1/operations"))
        .bearer_auth(token)
        .json(operation)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_and_http_share_snapshot_catchup_private_observations_and_restart_boundaries() {
    let mut f = agent::Fixture::start().await;
    let mut config = f.issue_as_operator(&f.agent, Scope::collaboration(vec!["*".into()]));
    let peer_config = f.issue_as_operator(&f.peer, Scope::collaboration(vec!["*".into()]));
    let publisher = f.issue_as_operator(&f.agent, Scope::operator());
    let mut client = ConnectClient::open(config.clone()).await.unwrap();
    let mut peer = ConnectClient::open(peer_config.clone()).await.unwrap();
    let cid = client
        .create("events-room", vec![f.peer.identity_id.clone()])
        .await
        .unwrap()
        .id;
    let snapshot = client
        .conversation_snapshot(
            &cid,
            SnapshotQuery {
                limit: Some(1),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(snapshot.has_more);
    let original_page = snapshot.next.clone().unwrap();
    let mut next = snapshot.next.clone();
    let mut parts = vec![];
    while let Some(page) = next {
        let page = client
            .conversation_snapshot(
                &cid,
                SnapshotQuery {
                    page: Some(page),
                    limit: Some(1),
                },
            )
            .await
            .unwrap();
        assert_eq!(page.cursor, snapshot.cursor);
        parts.extend(page.items.into_iter().filter_map(|item| {
            if let SnapshotItem::Participant(p) = item {
                Some(p)
            } else {
                None
            }
        }));
        next = page.next;
    }
    assert_eq!(parts.len(), 2);
    let accepted = client
        .send("message", &cid, "durable catch-up")
        .await
        .unwrap();
    let native = client
        .conversation_events(
            &cid,
            EventQuery {
                after: Some(snapshot.cursor.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(native.events.len(), 1);
    assert!(matches!(&native.events[0].data,JournalData::Message(m)if m.id==accepted.message_id));
    let base = format!("http://{}", f.server.http_address.unwrap());
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let snapshot_op = Operation::ConversationSnapshot {
        conversation_id: cid.clone(),
        query: SnapshotQuery {
            limit: Some(1),
            ..Default::default()
        },
    };
    let snapshot_json = post(&http, &base, &config.credential, &snapshot_op).await;
    let parsed: Response = serde_json::from_value(snapshot_json.clone()).unwrap();
    assert!(
        matches!(parsed.result.unwrap(),Outcome::ConversationSnapshot(page)if page.conversation_id==cid&&page.revision==0)
    );
    let events_op = Operation::ConversationEvents {
        conversation_id: cid.clone(),
        query: EventQuery {
            after: Some(snapshot.cursor.clone()),
            ..Default::default()
        },
    };
    let events_json = post(&http, &base, &config.credential, &events_op).await;
    let parsed: Response = serde_json::from_value(events_json.clone()).unwrap();
    assert!(
        matches!(parsed.result.unwrap(),Outcome::ConversationEvents(page)if page.events[0].event_id==native.events[0].event_id)
    );
    let waiting_op = Operation::ConversationEvents {
        conversation_id: cid.clone(),
        query: EventQuery {
            after: Some(native.cursor),
            wait_ms: 300,
            ..Default::default()
        },
    };
    let waiting_http = http.clone();
    let waiting_base = base.clone();
    let token = config.credential.clone();
    let waiting =
        tokio::spawn(async move { post(&waiting_http, &waiting_base, &token, &waiting_op).await });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let during = client
        .send("during-wait", &cid, "arrived while polling")
        .await
        .unwrap();
    let waited = tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
        .await
        .unwrap()
        .unwrap();
    let parsed: Response = serde_json::from_value(waited).unwrap();
    assert!(
        matches!(parsed.result.unwrap(),Outcome::ConversationEvents(page)if page.events.iter().any(|e|matches!(&e.data,JournalData::Message(m)if m.id==during.message_id)))
    );
    let sid = client
        .session_invite("invite", &cid, &f.peer.identity_id, vec!["audio".into()])
        .await
        .unwrap()
        .id;
    peer.session_accept("accept", &cid, &sid).await.unwrap();
    let pid = parts
        .iter()
        .find(|p| p.identity_id == config.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let server = connect_server::config::ServerConfig::load(&f.server_config_path()).unwrap();
    {
        let c = rusqlite::Connection::open(&server.database).unwrap();
        c.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        c.execute("INSERT INTO devices(id,identity_id,tenant_id,kind,platform) VALUES('observation-device',?1,'tnt_default','fixture','fixture')",[&config.identity_id]).unwrap();
        c.execute("INSERT INTO endpoints(id,identity_id,tenant_id,device_id,transport,address,capabilities_json,verification_state,verification_evidence,verified_at) VALUES('observation-endpoint',?1,'tnt_default','observation-device','fixture','fixture://source','[\"audio\"]','verified','isolated-fixture',?2)",rusqlite::params![config.identity_id,chrono::Utc::now().timestamp()]).unwrap();
        c.execute("INSERT INTO session_participants(session_id,conversation_id,tenant_id,participant_id,membership_epoch,role,state) VALUES(?1,?2,'tnt_default',?3,1,'participant','present') ON CONFLICT(session_id,participant_id,membership_epoch) DO UPDATE SET role=excluded.role,state=excluded.state",rusqlite::params![sid,cid,pid]).unwrap();
        c.execute("INSERT INTO connections(id,session_id,participant_id,membership_epoch,identity_id,tenant_id,endpoint_id,generation,state,observed_runtime_id) VALUES('observation-connection',?1,?2,1,?3,'tnt_default','observation-endpoint',1,'active',?4)",rusqlite::params![sid,pid,config.identity_id,f.server.store.runtime_epoch()]).unwrap();
    }
    let source = f
        .server
        .store
        .bind_session_observation_source(
            &publisher.credential_id,
            "observation-connection",
            "fixture",
            "actual-fixture-route",
            &[pid],
            ObservationCapabilities {
                transcript: true,
                speech: false,
                actions: vec![],
            },
        )
        .unwrap();
    let observation = f
        .server
        .service
        .publish_session_observation(
            &source,
            "fragment-one",
            ObservationDraft::Transcript {
                text: "private fragment".into(),
                completeness: TranscriptCompleteness::Final,
                turn_id: None,
                fragment_id: Some("one".into()),
            },
        )
        .unwrap();
    let own = client
        .session_observations(&cid, &sid, ObservationQuery::default())
        .await
        .unwrap();
    assert_eq!(own.observations[0].observation_id, observation);
    assert!(!own.routes[0].capabilities.speech);
    let foreign = peer
        .session_observations(&cid, &sid, ObservationQuery::default())
        .await
        .unwrap();
    assert!(foreign.observations.is_empty());
    assert!(foreign.routes.is_empty());
    let observation_op = Operation::SessionObservations {
        conversation_id: cid.clone(),
        session_id: sid.clone(),
        query: ObservationQuery::default(),
    };
    let observations_json = post(&http, &base, &config.credential, &observation_op).await;
    let parsed: Response = serde_json::from_value(observations_json.clone()).unwrap();
    assert!(
        matches!(parsed.result.unwrap(),Outcome::SessionObservations(page)if page.observations[0].observation_id==observation)
    );
    for forged in [
        json!({"op":"publish_session_observation","connection_id":"observation-connection"}),
        json!({"op":"conversation_events","conversation_id":cid,"query":{"source_identity":"forged"}}),
    ] {
        assert_eq!(
            http.post(format!("{base}/v1/operations"))
                .bearer_auth(&config.credential)
                .json(&forged)
                .send()
                .await
                .unwrap()
                .status(),
            422
        );
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf();
    let data=json!({"base":base,"token":config.credential,"foreign_token":peer_config.credential,"cid":cid,"sid":sid,"cursor":snapshot.cursor,"message_id":accepted.message_id,"observation_id":observation}).to_string();
    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        for (program, script) in [
            ("node", "sdk/tests/events_probe.mjs"),
            ("python3", "sdk/tests/events_probe.py"),
        ] {
            let mut child = std::process::Command::new(program)
                .current_dir(&root)
                .env("PYTHONPATH", root.join("sdk/python"))
                .arg(script)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(data.as_bytes())
                .unwrap();
            let result = child.wait_with_output().unwrap();
            assert!(
                result.status.success(),
                "SDK probe: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert_eq!(
                serde_json::from_slice::<Value>(&result.stdout).unwrap(),
                json!({"passed":true})
            );
        }
    })
    .await
    .unwrap();
    if let Some(directory) = std::env::var_os("CONNECT_GATE6_EVENT_SAMPLES") {
        let dir = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, request, response) in [
            ("snapshot", snapshot_op, snapshot_json),
            ("events", events_op, events_json),
            ("observations", observation_op, observations_json),
        ] {
            std::fs::write(
                dir.join(format!("{name}.json")),
                serde_json::to_vec_pretty(&json!({"request":request,"response":response})).unwrap(),
            )
            .unwrap();
        }
    }
    client.close().await;
    peer.close().await;
    f = f.restart().await;
    config.endpoint = f.server.address;
    client = ConnectClient::open(config).await.unwrap();
    assert!(
        matches!(client.conversation_snapshot(&cid,SnapshotQuery{page:Some(original_page),limit:Some(1)}).await.unwrap_err(),connect_client::ClientError::Application(error)if error.code=="resync-required")
    );
    let after_restart = client
        .session_observations(
            &cid,
            &sid,
            ObservationQuery {
                after: Some(own.cursor),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(after_restart.resync_required);
    assert!(after_restart.observations.is_empty());
    client.close().await;
    f.close().await;
}
