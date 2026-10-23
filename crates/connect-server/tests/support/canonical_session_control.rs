//! Production native/HTTP control. No physical media readiness is claimed here.
use super::*;
use connect_client::{
    canonical::CanonicalHttpClient,
    session_control::{Action, Request},
};
fn request(cid: &str, id: &str, revision: Option<u64>, action: Action) -> Request {
    Request {
        operation_id: id.into(),
        conversation_id: cid.into(),
        session_id: "public-main-session".into(),
        by_participant: None,
        expected_revision: revision,
        action,
    }
}
async fn raw_replay(
    f: &Fixture,
    token: &str,
    case: &str,
    request: &Request,
    body: Value,
    status: u16,
) -> Value {
    let mut value = json!({"v":2,"type":request.action.required_scope(),"id":format!("session_http_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),"cid":request.conversation_id,"sid":request.session_id,"operation_id":request.operation_id,"payload":body});
    if let Some(revision) = request.expected_revision {
        value["expected_revision"] = json!(revision);
    }
    let client =
        tokio::task::spawn_blocking(|| reqwest::Client::builder().no_proxy().build().unwrap())
            .await
            .unwrap();
    let response = client
        .post(format!(
            "http://{}/v2/commands",
            f.server.http_address.unwrap()
        ))
        .bearer_auth(token)
        .json(&value)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), status);
    assert_eq!(
        response.headers()[reqwest::header::CACHE_CONTROL],
        "no-store"
    );
    let response: Value = response.json().await.unwrap();
    if let Ok(directory) = std::env::var("CONNECT_G6_CANONICAL_HTTP_SAMPLES") {
        std::fs::write(std::path::Path::new(&directory).join(format!("{case}.json")),serde_json::to_vec_pretty(&json!({"case":case,"valid_request":true,"request":value,"status":status,"response":response})).unwrap()).unwrap();
    }
    response
}
#[tokio::test]
async fn signed_and_http_general_session_lifecycle_share_four_party_receipts_and_current_views() {
    let f = Fixture::with_membership_scope("EdDSA", 12, true).await;
    let admin = f.server.store.authenticate(&f.admin).unwrap();
    let mut identities = vec![f.client.identity_id.clone(), f.peer.identity_id.clone()];
    for (index, kind) in [IdentityKind::External, IdentityKind::Human]
        .into_iter()
        .enumerate()
    {
        let ProvisioningOutcome::Identity { identity_id, .. } = f
            .server
            .store
            .execute_provisioning(
                &admin.credential_id,
                ProvisioningCommand::Identity {
                    operation_id: format!("general-party-{index}"),
                    alias: format!("general-party-{index}"),
                    kind,
                    display_name: None,
                },
            )
            .unwrap()
        else {
            panic!()
        };
        identities.push(identity_id);
    }
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut input =
        connect_client::conversations::CreateRequest::persistent(identities[1..].to_vec());
    input.tenant_id = Some("tnt_default".into());
    let room = native
        .create_conversation("general-session-room", input)
        .await
        .unwrap();
    let participants: Vec<_> = identities
        .iter()
        .map(|id| {
            room.participants
                .iter()
                .find(|p| &p.identity_id == id)
                .unwrap()
                .participant_id
                .clone()
        })
        .collect();
    let offer = json!({"audio":["opus","g.711-mu"],"video":["vp8"]});
    let mut invite = request(
        &room.id,
        "general-invite",
        None,
        Action::Invite {
            participants: participants[1..].to_vec(),
            medium: "voice".into(),
            intent: "conference".into(),
            capabilities: offer.clone(),
        },
    );
    invite.by_participant = Some(participants[0].clone());
    let receipt = native.session_command(invite.clone()).await.unwrap();
    assert_eq!(
        receipt,
        native.session_command(invite.clone()).await.unwrap()
    );
    let raw=raw_replay(&f,&f.client.credential,"general-invite-replay",&invite,json!({"from":participants[0],"to":participants[1..],"medium":"voice","intent":"conference","capabilities_offer":offer}),200).await;
    assert_eq!(
        raw["envelopes"][0]["payload"]["result"],
        serde_json::to_value(&receipt).unwrap()
    );
    let mut tokens = vec![];
    let mut http_clients = vec![];
    for identity in &identities[1..] {
        let key = f
            .server
            .store
            .issue(
                identity,
                Scope::collaboration(vec![identities[0].clone()]),
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap();
        let mut config = canonical_http_config(&f);
        config.credential = key.token.clone();
        tokens.push(key.token);
        http_clients.push(CanonicalHttpClient::open(config).await.unwrap());
    }
    for (index, client) in http_clients.iter_mut().enumerate() {
        let answer = if index == 1 {
            json!({"audio":["g.711-mu"]})
        } else {
            json!({"audio":["opus"],"video":["vp8"]})
        };
        let mut accept = request(
            &room.id,
            &format!("general-accept-{index}"),
            Some(index as u64),
            Action::Accept {
                capabilities: answer.clone(),
            },
        );
        accept.by_participant = Some(participants[index + 1].clone());
        let receipt = client.session_command(accept.clone()).await.unwrap();
        assert_eq!(
            client.session_command(accept.clone()).await.unwrap(),
            receipt
        );
        if index == 0 {
            let raw = raw_replay(
                &f,
                &tokens[0],
                "general-accept-replay",
                &accept,
                json!({"by":participants[1],"capabilities_answer":answer}),
                200,
            )
            .await;
            assert_eq!(
                raw["envelopes"][0]["payload"]["result"],
                serde_json::to_value(receipt).unwrap()
            );
        }
    }
    let snapshot = native
        .conversation_snapshot(&room.id, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    let snapshot: Vec<connect_client::events::SnapshotItem> = snapshot
        .items
        .into_iter()
        .map(|item| serde_json::from_value(item).unwrap())
        .collect();
    assert_eq!(snapshot.iter().filter(|item|matches!(item,connect_client::events::SnapshotItem::SessionParticipant(p)if p.state=="present")).count(),4);
    let phone = http_clients[1]
        .conversation_snapshot(&room.id, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    let phone: Vec<connect_client::events::SnapshotItem> = phone
        .items
        .into_iter()
        .map(|item| serde_json::from_value(item).unwrap())
        .collect();
    assert!(phone.iter().any(|item|matches!(item,connect_client::events::SnapshotItem::Session(s)if s.id=="public-main-session"&&s.is_main&&s.state=="accepted")));
    assert!(!phone.iter().any(|item|matches!(item,connect_client::events::SnapshotItem::Participant(p)if p.identity_id==identities[3])));
    let camera = request(
        &room.id,
        "general-phone-camera",
        Some(3),
        Action::Upgrade {
            media: vec!["video".into()],
        },
    );
    assert!(http_clients[1]
        .session_command(camera.clone())
        .await
        .is_err());
    let raw = raw_replay(
        &f,
        &tokens[1],
        "general-camera-refusal",
        &camera,
        json!({"kind":"medium-upgrade","details":{"media":["video"]}}),
        501,
    )
    .await;
    assert_eq!(raw["envelopes"][0]["type"], "error");
    let failed = http_clients[1]
        .operation_get(
            "general-phone-camera",
            CommandScope {
                conversation_id: Some(room.id.clone()),
                session_id: Some("public-main-session".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(failed.state).unwrap(), json!("failed"));
    assert!(failed.result.is_none());
    let role = request(
        &room.id,
        "general-role",
        Some(3),
        Action::Role {
            participant_id: participants[3].clone(),
            role: "observer".into(),
        },
    );
    let changed = native.session_command(role.clone()).await.unwrap();
    let raw=raw_replay(&f,&f.client.credential,"general-role-replay",&role,json!({"kind":"role-change","details":{"participant_id":participants[3],"role":"observer"}}),200).await;
    assert_eq!(
        raw["envelopes"][0]["payload"]["result"],
        serde_json::to_value(changed).unwrap()
    );
    let leave = request(
        &room.id,
        "general-phone-leave",
        Some(4),
        Action::Leave {
            participant_id: participants[2].clone(),
        },
    );
    http_clients[1]
        .session_command(leave.clone())
        .await
        .unwrap();
    http_clients[1].session_command(leave).await.unwrap();
    let after = native
        .conversation_snapshot(&room.id, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    let after: Vec<connect_client::events::SnapshotItem> = after
        .items
        .into_iter()
        .map(|item| serde_json::from_value(item).unwrap())
        .collect();
    assert_eq!(after.iter().filter(|item|matches!(item,connect_client::events::SnapshotItem::SessionParticipant(p)if p.state=="present")).count(),3);
    let end = request(
        &room.id,
        "general-end",
        Some(5),
        Action::End {
            reason_code: 42,
            reason: "done".into(),
        },
    );
    let ended = native.session_command(end.clone()).await.unwrap();
    let raw = raw_replay(
        &f,
        &f.client.credential,
        "general-end-replay",
        &end,
        json!({"reason_code":42,"reason":"done"}),
        200,
    )
    .await;
    assert_eq!(
        raw["envelopes"][0]["payload"]["result"],
        serde_json::to_value(ended).unwrap()
    );
    native
        .send_message(
            "general-after-end",
            &room.id,
            "after_session_end",
            connect_client::MessageDraft::text_all("conversation remains open"),
        )
        .await
        .unwrap();
    native.close();
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn signed_session_projection_uses_actual_physical_peers_and_replay_cannot_resurrect_end() {
    let f = Fixture::new("EdDSA", 8).await;
    let mut first = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut replacement = CanonicalClient::open(f.client.clone()).await.unwrap();
    let cid = create(&f, &mut first, "physical-projection-room").await;
    let members = f
        .server
        .store
        .membership_snapshot(
            &f.server
                .store
                .authenticate(&f.client.credential)
                .unwrap()
                .credential_id,
            &cid,
        )
        .unwrap()
        .participants;
    let target = members
        .iter()
        .find(|p| p.identity_id == f.peer.identity_id)
        .unwrap();
    let invite = request(
        &cid,
        "physical-projection-invite",
        None,
        Action::Invite {
            participants: vec![target.participant_id.clone()],
            medium: "voice".into(),
            intent: "call".into(),
            capabilities: json!({"audio":["opus"]}),
        },
    );
    let original = first.session_command(invite.clone()).await.unwrap();
    let snapshot = f.server.service.canonical_runtime_snapshot().unwrap();
    assert_eq!(snapshot.live_sessions, 1);
    assert_eq!(snapshot.peer_session_views, 1);
    assert_eq!(snapshot.connections, 0);
    assert!(snapshot.media_readers >= 1);
    assert_eq!(
        replacement.session_command(invite.clone()).await.unwrap(),
        original
    );
    assert_eq!(
        f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .peer_session_views,
        2
    );
    first.close();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while f
            .server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .peer_session_views
            != 1
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let end = request(
        &cid,
        "physical-projection-end",
        Some(0),
        Action::End {
            reason_code: 200,
            reason: "done".into(),
        },
    );
    replacement.session_command(end).await.unwrap();
    let ended = f.server.service.canonical_runtime_snapshot().unwrap();
    assert_eq!(ended.live_sessions, 0);
    assert_eq!(ended.peer_session_views, 0);
    assert_eq!(ended.connections, 0);
    assert_eq!(replacement.session_command(invite).await.unwrap(), original);
    assert_eq!(
        f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .live_sessions,
        0
    );
    let mut next = request(
        &cid,
        "physical-http-invite",
        None,
        Action::Invite {
            participants: vec![target.participant_id.clone()],
            medium: "voice".into(),
            intent: "call".into(),
            capabilities: json!({"audio":["opus"]}),
        },
    );
    next.session_id = "physical-http-session".into();
    let next_receipt = replacement.session_command(next.clone()).await.unwrap();
    assert_eq!(
        f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .live_sessions,
        1
    );
    let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
        .await
        .unwrap();
    let mut http_end = request(
        &cid,
        "physical-http-end",
        Some(0),
        Action::End {
            reason_code: 200,
            reason: "done over HTTP".into(),
        },
    );
    http_end.session_id = next.session_id.clone();
    let ended_receipt = http.session_command(http_end.clone()).await.unwrap();
    assert_eq!(http.session_command(http_end).await.unwrap(), ended_receipt);
    assert_eq!(
        f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .live_sessions,
        0
    );
    assert_eq!(
        f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .peer_session_views,
        0
    );
    assert_eq!(
        replacement.session_command(next).await.unwrap(),
        next_receipt
    );
    assert_eq!(
        f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .live_sessions,
        0
    );
    replacement.close();
    f.server.shutdown().await.unwrap();
}

#[tokio::test]
async fn http_only_session_creates_empty_checked_core_and_later_native_attaches_without_recreation()
{
    let f = Fixture::with_membership_scope("EdDSA", 12, true).await;
    let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
        .await
        .unwrap();
    let mut input =
        connect_client::conversations::CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
    input.tenant_id = Some("tnt_default".into());
    let room = http
        .create_conversation("http-first-room", input)
        .await
        .unwrap();
    let target = room
        .participants
        .iter()
        .find(|p| p.identity_id == f.peer.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let before = f.server.service.canonical_runtime_snapshot().unwrap();
    assert_eq!(
        (
            before.physical_peers,
            before.peer_session_views,
            before.live_sessions,
            before.connections
        ),
        (0, 0, 0, 0)
    );
    let invite = request(
        &room.id,
        "http-first-invite",
        None,
        Action::Invite {
            participants: vec![target.clone()],
            medium: "voice".into(),
            intent: "public-conference".into(),
            capabilities: json!({"audio":["g.711-mu"]}),
        },
    );
    let receipt = http.session_command(invite.clone()).await.unwrap();
    assert_eq!(http.session_command(invite.clone()).await.unwrap(), receipt);
    let empty = f.server.service.canonical_runtime_snapshot().unwrap();
    assert_eq!(
        (
            empty.physical_peers,
            empty.peer_session_views,
            empty.live_sessions,
            empty.connections,
            empty.media_readers
        ),
        (0, 0, 1, 0, 0)
    );
    let key = f
        .server
        .store
        .issue(
            &f.peer.identity_id,
            Scope::collaboration(vec![f.client.identity_id.clone()]),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut config = canonical_http_config(&f);
    config.credential = key.token;
    let mut callee = CanonicalHttpClient::open(config).await.unwrap();
    let mut accept = request(
        &room.id,
        "http-first-accept",
        Some(0),
        Action::Accept {
            capabilities: json!({"audio":["g.711-mu"]}),
        },
    );
    accept.by_participant = Some(target);
    callee.session_command(accept).await.unwrap();
    let empty = f.server.service.canonical_runtime_snapshot().unwrap();
    assert_eq!(
        (
            empty.physical_peers,
            empty.peer_session_views,
            empty.live_sessions,
            empty.connections
        ),
        (0, 0, 1, 0)
    );
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    assert_eq!(
        native.session_command(invite.clone()).await.unwrap(),
        receipt
    );
    let attached = f.server.service.canonical_runtime_snapshot().unwrap();
    assert_eq!(
        (
            attached.physical_peers,
            attached.peer_session_views,
            attached.live_sessions,
            attached.connections
        ),
        (1, 1, 1, 0)
    );
    let owner = f.server.store.authenticate(&f.client.credential).unwrap();
    let session = f
        .server
        .store
        .session_view(&owner.credential_id, &room.id, "public-main-session")
        .unwrap();
    let end = request(
        &room.id,
        "http-first-end",
        Some(session.topology_revision),
        Action::End {
            reason_code: 1000,
            reason: "complete".into(),
        },
    );
    http.session_command(end).await.unwrap();
    let ended = f.server.service.canonical_runtime_snapshot().unwrap();
    assert_eq!(ended.live_sessions, 0);
    assert_eq!(ended.connections, 0);
    assert_eq!(http.session_command(invite.clone()).await.unwrap(), receipt);
    assert_eq!(native.session_command(invite).await.unwrap(), receipt);
    assert_eq!(
        f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .live_sessions,
        0
    );
    native.close();
    f.server.shutdown().await.unwrap();
}
