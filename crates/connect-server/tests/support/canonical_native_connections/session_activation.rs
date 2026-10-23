//! Actual signed peers and direction-appropriate RTP, never injected readiness.
use super::*;
fn state(f: &Fixture) -> (String, u64, u64) {
    rusqlite::Connection::open(&f.config.database).unwrap().query_row("SELECT (SELECT state FROM sessions WHERE id='native-session'),(SELECT COUNT(*) FROM native_connection_flow_facts),(SELECT COUNT(*) FROM session_started_facts)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap()
}
async fn observed(f: &Fixture, flows: u64, phase: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let current = state(f);
            if current.1 == flows {
                assert_eq!(current.0, phase);
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}
async fn publish(client: &mut CanonicalClient, cid: &str, name: &str) -> u16 {
    let answer=negotiate_av(client,cid,&format!("{name}-offer"),name,json!([{"id":format!("{name}-voice"),"kind":"audio","direction":"sendonly","codec_preferences":["g.711-mu"]}])).await;
    answer.original()["payload"]["substrate_setup"]["streams"][0]["stream_local_id"]
        .as_u64()
        .unwrap() as u16
}
#[tokio::test]
async fn production_session_activation_waits_for_each_accepted_participant_not_connection_count_or_metadata(
) {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 12).await;
        ready_endpoint(&f).await;
        let mut caller = CanonicalClient::open(f.client.clone()).await.unwrap();
        let cid = session(&f, &mut caller).await;
        let config = party_with_streams(&f, &f.peer.identity_id, 238, true);
        let mut peer = CanonicalClient::open(config.clone()).await.unwrap();
        let first = publish(&mut caller, &cid, "activation-first").await;
        wait_native_operation(&f, &f.client.credential, &cid, "activation-first-offer").await;
        assert_eq!(state(&f), ("accepted".into(), 0, 0));
        participants::flow(&caller, first, 1).await;
        observed(&f, 1, "accepted").await;
        let second = publish(&mut caller, &cid, "activation-second").await;
        wait_native_operation(&f, &f.client.credential, &cid, "activation-second-offer").await;
        assert_ne!(first, second, "handles on this physical peer cannot alias");
        participants::flow(&caller, second, 101).await;
        observed(&f, 2, "accepted").await;
        assert_eq!(
            state(&f).2,
            0,
            "two Connections for one Participant do not cover the other accepted Participant"
        );
        let third = publish(&mut peer, &cid, "activation-peer").await;
        wait_native_operation(&f, &config.credential, &cid, "activation-peer-offer").await;
        assert_eq!(
            state(&f),
            ("accepted".into(), 2, 0),
            "confirmed negotiation alone does not create an observed ready flow"
        );
        participants::flow(&peer, third, 201).await;
        observed(&f, 3, "active").await;
        let started = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let frame = peer.receive().await.unwrap();
                if frame.kind() == "session.started" {
                    break frame;
                }
                assert!(
                    matches!(
                        frame.kind(),
                        "session.participant.joined"
                            | "connection.ready"
                            | "stream.opened"
                            | "connection.quality"
                    ),
                    "{:?}",
                    frame.original()
                );
            }
        })
        .await
        .unwrap();
        let body = &started.original()["payload"];
        assert_eq!(body["participants_present"].as_array().unwrap().len(), 2);
        let connections = body["active_connections"].as_array().unwrap();
        assert_eq!(connections.len(), 3);
        assert_eq!(
            connections
                .iter()
                .map(|c| c["connid"].as_str().unwrap())
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from([
                "activation-first",
                "activation-second",
                "activation-peer"
            ])
        );
        let original = f
            .server
            .store
            .session_started_template(&cid, "native-session")
            .unwrap();
        participants::flow(&caller, first, 301).await;
        assert_eq!(
            f.server
                .store
                .session_started_template(&cid, "native-session")
                .unwrap(),
            original
        );
        assert_eq!(state(&f).2, 1);
        if let Some(path) = std::env::var_os("CONNECT_G6_SESSION_ACTIVATION_SAMPLES") {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .unwrap();
            writeln!(file,"{}",json!({"algorithm":algorithm,"started":started.original(),"same_peer_handles":[first,second],"peer_handle":third,"accepted_without_all_ready":true,"negotiation_without_flow_is_not_active":true})).unwrap();
        }
        caller.close();
        peer.close();
        f.server.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn production_session_ended_before_all_accepted_ready_never_emits_a_start() {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 12).await;
        ready_endpoint(&f).await;
        let mut caller = CanonicalClient::open(f.client.clone()).await.unwrap();
        let cid = session(&f, &mut caller).await;
        let handle = publish(&mut caller, &cid, "activation-aborted").await;
        wait_native_operation(&f, &f.client.credential, &cid, "activation-aborted-offer").await;
        participants::flow(&caller, handle, 1).await;
        observed(&f, 1, "accepted").await;
        let current = f.server.store.authenticate(&f.client.credential).unwrap();
        let revision = f
            .server
            .store
            .session_view(&current.credential_id, &cid, "native-session")
            .unwrap()
            .topology_revision;
        caller
            .session_command(Request {
                operation_id: "end-incomplete-activation".into(),
                conversation_id: cid.clone(),
                session_id: "native-session".into(),
                by_participant: None,
                expected_revision: Some(revision),
                action: Action::End {
                    reason_code: 200,
                    reason: "Ended before all accepted Participants were ready".into(),
                },
            })
            .await
            .unwrap();
        assert_eq!(state(&f).0, "ended");
        assert_eq!(state(&f).2, 0);
        caller.close();
        f.server.shutdown().await.unwrap();
    }
}
