//! Exact auxiliary clauses through the installed production listener/public SDK.
use super::canonical_native_connections::ControlReception;
use super::*;
use connect_client::conversations::*;
use connect_protocol::events::JournalData;

fn scope(cid: Option<&str>, operation: Option<&str>) -> CommandScope {
    CommandScope {
        conversation_id: cid.map(str::to_owned),
        operation_id: operation.map(str::to_owned),
        ..Default::default()
    }
}
async fn subscribe(client: &mut CanonicalClient, cid: &str) -> String {
    let initial = client
        .subscribe_conversation(
            cid,
            serde_json::from_value(json!({"live":true,"lease_seconds":30})).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(initial.frames[0].kind(), "conversation.snapshot");
    initial.ack().unwrap();
    initial.request_id
}
async fn closure(
    client: &mut CanonicalClient,
    cid: &str,
    request: &str,
    reason: &str,
    code: u64,
) -> (Value, connect_protocol::events::ConversationClosed) {
    let mut hint = None;
    for _ in 0..64 {
        let frame = client.receive().await.unwrap();
        assert_eq!(frame.scope(rvoip_uctp::v2::Scope::Conversation), Some(cid));
        assert_eq!(frame.scope(rvoip_uctp::v2::Scope::Reply), Some(request));
        if frame.kind() == "conversation.closed" {
            assert!(hint.is_none(), "duplicate closure hint in one page");
            let p = frame.original()["payload"].clone();
            assert_eq!(p["reason"], reason);
            assert_eq!(p["reason_code"], code);
            chrono::DateTime::parse_from_rfc3339(p["closed_at"].as_str().unwrap()).unwrap();
            hint = Some(p);
        }
        if frame.kind() == "conversation.event" {
            let event: connect_protocol::events::JournalEvent =
                serde_json::from_value(frame.original()["payload"]["data"].clone()).unwrap();
            if event.kind == "conversation_closed" {
                let JournalData::ConversationClosed(fact) = event.data else {
                    panic!("closure lost original metadata")
                };
                assert_eq!(fact.conversation_id, cid);
                assert_eq!(fact.reason, reason);
                assert_eq!(fact.reason_code, code);
                assert!(fact.closed_at > 0 && fact.closed_at <= event.observed_at);
                let hint = hint.expect("typed closure must precede its cursor-bearing fact");
                assert_eq!(
                    chrono::DateTime::parse_from_rfc3339(hint["closed_at"].as_str().unwrap())
                        .unwrap()
                        .timestamp(),
                    fact.closed_at
                );
                return (hint, fact);
            }
        }
    }
    panic!("closure multicast not delivered")
}
fn witness(algorithm: &str, case: u32, assertions: &[&str]) {
    if let Some(directory) = std::env::var_os("CONNECT_G6_AUXILIARY_WITNESSES") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(format!("{algorithm}-u6-req-{case:04}.json")),serde_json::to_vec_pretty(&json!({"case_id":format!("u6-req-{case:04}"),"algorithm":algorithm,"test":"auxiliary_clauses::production_auxiliary_conversation_clauses_0069_through_0080","assertions":assertions,"profile":"production Connect local native signed QUIC; public SDK; no injected domain rows"})).unwrap()).unwrap();
    }
}
#[tokio::test]
async fn production_auxiliary_conversation_clauses_0069_through_0080() {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_membership_scope(algorithm, 12, true).await;
        let mut writer = CanonicalClient::open(f.client.clone()).await.unwrap();
        let since = chrono::Utc::now() - chrono::Duration::seconds(1);
        // The installed explicit-create profile must not silently open a room.
        let refused=writer.request_value("message.send",json!({"msg_id":"aux_implicit_message","to":"all","content_type":"text/plain","body":"must not create a Conversation"}),scope(Some("aux_never_created"),Some("aux-implicit-denied"))).await.unwrap();
        assert!(
            matches!(refused.ack(),Err(ClientError::Application(ref e)) if e.code=="forbidden")
        );
        assert!(writer
            .list_conversations(ListQuery::default())
            .await
            .unwrap()
            .conversations
            .is_empty());
        // Raw public requests exercise server defaults, not only SDK defaults.
        let default = writer
            .request_value(
                "conversation.create",
                json!({"tenant_id":"tnt_default","policy":"ephemeral"}),
                scope(None, Some("aux-default-ephemeral")),
            )
            .await
            .unwrap();
        let p = &default.frames[0].original()["payload"];
        assert_eq!(p["policy"], "ephemeral");
        assert_eq!(p["idle_close_secs"], 60);
        default.ack().unwrap();
        let default_cid = default.frames[0]
            .scope(rvoip_uctp::v2::Scope::Conversation)
            .unwrap()
            .to_owned();
        let metadata = json!({"qualification":"exact auxiliary clauses","nested":{"kept":true}});
        let input = json!({"tenant_id":"tnt_default","policy":"persistent","idle_close_secs":1,"metadata":metadata,"initial_participants":[{"identity_id":f.peer.identity_id,"role":"guest"}]});
        let created = writer
            .request_value(
                "conversation.create",
                input.clone(),
                scope(None, Some("aux-persistent-room")),
            )
            .await
            .unwrap();
        created.ack().unwrap();
        assert_eq!(created.frames.len(), 2);
        let opened = &created.frames[0];
        assert_eq!(opened.kind(), "conversation.opened");
        assert_eq!(
            opened.scope(rvoip_uctp::v2::Scope::Reply),
            Some(created.request_id.as_str())
        );
        let cid = opened
            .scope(rvoip_uctp::v2::Scope::Conversation)
            .unwrap()
            .to_owned();
        assert_ne!(cid, "aux-persistent-room");
        let body = &opened.original()["payload"];
        assert_eq!(body["tenant_id"], "tnt_default");
        assert_eq!(body["policy"], "persistent");
        assert_eq!(body["idle_close_secs"], 1);
        assert_eq!(body["metadata"], metadata);
        chrono::DateTime::parse_from_rfc3339(body["opened_at"].as_str().unwrap()).unwrap();
        let people = body["participants"].as_array().unwrap();
        assert_eq!(people.len(), 2);
        for person in people {
            assert!(!person["participant_id"].as_str().unwrap().is_empty());
            assert!(matches!(
                person["kind"].as_str(),
                Some("human" | "ai" | "system" | "external")
            ));
            assert!(!person["role"].as_str().unwrap().is_empty());
        }
        let guest = people
            .iter()
            .find(|p| p["identity_id"] == f.peer.identity_id)
            .unwrap();
        assert_eq!(guest["role"], "guest");
        let owner = people
            .iter()
            .find(|p| p["identity_id"] == f.client.identity_id)
            .unwrap()["participant_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let replay = writer
            .request_value(
                "conversation.create",
                input,
                scope(None, Some("aux-persistent-room")),
            )
            .await
            .unwrap();
        assert_eq!(
            replay.frames[0].scope(rvoip_uctp::v2::Scope::Conversation),
            Some(cid.as_str())
        );
        // Production tenant validation is observed at the actual HTTP parser.
        let http =
            tokio::task::spawn_blocking(|| reqwest::Client::builder().no_proxy().build().unwrap())
                .await
                .unwrap();
        let bad = json!({"v":2,"type":"conversation.create","id":"aux_missing_tenant","ts":chrono::Utc::now(),"operation_id":"aux-missing-tenant","payload":{"policy":"persistent"}});
        let response = http
            .post(format!(
                "http://{}/v2/commands",
                f.server.http_address.unwrap()
            ))
            .bearer_auth(&f.client.credential)
            .json(&bad)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 400);
        let update = json!({"v":2,"type":"conversation.update","id":"aux_policy_update","ts":chrono::Utc::now(),"cid":default_cid,"operation_id":"aux-policy-update","payload":{"policy":"persistent"}});
        let response = http
            .post(format!(
                "http://{}/v2/commands",
                f.server.http_address.unwrap()
            ))
            .bearer_auth(&f.client.credential)
            .json(&update)
            .send()
            .await
            .unwrap();
        assert!(matches!(response.status().as_u16(), 400 | 501));
        let extra = writer
            .create_conversation(
                "aux-second-persistent",
                CreateRequest {
                    tenant_id: Some("tnt_default".into()),
                    idle_close_secs: Some(1),
                    ..CreateRequest::persistent(vec![])
                },
            )
            .await
            .unwrap();
        let mut query = json!({"filter":{"tenant_id":"tnt_default","identity_id":f.client.identity_id,"state":"open","since":since,"until":chrono::Utc::now()+chrono::Duration::seconds(30)},"limit":1});
        let mut ids = std::collections::BTreeSet::new();
        let mut pages = 0;
        loop {
            let batch = writer
                .request_value("conversation.list", query.clone(), CommandScope::default())
                .await
                .unwrap();
            let ack = batch.ack().unwrap();
            assert_eq!(batch.frames.last().unwrap().kind(), "ack");
            let opened: Vec<_> = batch
                .frames
                .iter()
                .filter(|f| f.kind() == "conversation.opened")
                .collect();
            assert_eq!(opened.len(), 1);
            for frame in &batch.frames {
                assert_eq!(
                    frame.scope(rvoip_uctp::v2::Scope::Reply),
                    Some(batch.request_id.as_str())
                );
            }
            assert!(ids.insert(
                opened[0]
                    .scope(rvoip_uctp::v2::Scope::Conversation)
                    .unwrap()
                    .to_owned()
            ));
            pages += 1;
            if let Some(cursor) = &ack.next_cursor {
                query["cursor"] = json!(cursor);
            } else {
                break;
            }
        }
        assert_eq!(pages, 3);
        assert_eq!(
            ids,
            std::collections::BTreeSet::from([cid.clone(), default_cid.clone(), extra.id.clone()])
        );
        let filtered = writer
            .list_conversations(ListQuery {
                filter: ListFilter {
                    participant_id: Some(owner),
                    state: ConversationState::Open,
                    ..Default::default()
                },
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(filtered.conversations.len(), 1);
        assert_eq!(filtered.conversations[0].id, cid);
        for filter in [
            json!({"state":"closed"}),
            json!({"since":chrono::Utc::now()+chrono::Duration::seconds(30)}),
            json!({"until":since-chrono::Duration::seconds(30)}),
        ] {
            let result = writer
                .request_value(
                    "conversation.list",
                    json!({"filter":filter}),
                    CommandScope::default(),
                )
                .await
                .unwrap();
            result.ack().unwrap();
            assert!(!result
                .frames
                .iter()
                .any(|frame| frame.kind() == "conversation.opened"));
        }
        let peer_only = writer
            .list_conversations(ListQuery {
                filter: ListFilter {
                    identity_id: Some(f.peer.identity_id.clone()),
                    ..Default::default()
                },
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(peer_only.conversations.len(), 1);
        assert_eq!(peer_only.conversations[0].id, cid);
        let foreign = writer
            .request_value(
                "conversation.list",
                json!({"filter":{"tenant_id":"tnt_aux_absent"}}),
                CommandScope::default(),
            )
            .await
            .unwrap();
        assert!(
            matches!(foreign.ack(),Err(ClientError::Application(ref e)) if e.code=="forbidden")
        );
        let mut first = CanonicalClient::open(f.client.clone()).await.unwrap();
        let peer_config = canonical_native_connections::party_with_scope(
            &f,
            &f.peer.identity_id,
            123,
            vec!["data".into()],
            Scope::collaboration(vec!["*".into()]),
        );
        let mut second = CanonicalClient::open(peer_config).await.unwrap();
        let first_request = subscribe(&mut first, &cid).await;
        let second_request = subscribe(&mut second, &cid).await;
        let closed = writer
            .close_conversation(
                "aux-explicit-close",
                &cid,
                CloseOptions {
                    reason_code: Some(200),
                    reason: Some("explicit-close".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(closed.closed);
        let (one, one_fact) =
            closure(&mut first, &cid, &first_request, "explicit-close", 200).await;
        let (two, two_fact) =
            closure(&mut second, &cid, &second_request, "explicit-close", 200).await;
        assert_eq!(one, two);
        assert_eq!(one_fact, two_fact);
        assert_eq!(Some(one_fact.closed_at), closed.closed_at);
        let again = writer
            .close_conversation(
                "aux-explicit-close",
                &cid,
                CloseOptions {
                    reason_code: Some(200),
                    reason: Some("explicit-close".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(again, closed);
        // Active accepted Sessions suppress idle closure until they end.
        let held = writer
            .create_conversation(
                "aux-held-idle",
                CreateRequest {
                    tenant_id: Some("tnt_default".into()),
                    policy: ConversationPolicy::Ephemeral,
                    idle_close_secs: Some(3),
                    metadata: None,
                    initial_participants: vec![InitialParticipant {
                        identity_id: f.peer.identity_id.clone(),
                        role: "guest".into(),
                    }],
                },
            )
            .await
            .unwrap();
        let held_peer = held
            .participants
            .iter()
            .find(|p| p.identity_id == f.peer.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let request = |operation: &str, action| connect_client::session_control::Request {
            operation_id: operation.into(),
            conversation_id: held.id.clone(),
            session_id: "aux-held-session".into(),
            by_participant: None,
            expected_revision: match &action {
                connect_client::session_control::Action::Invite { .. } => None,
                connect_client::session_control::Action::Accept { .. } => Some(0),
                _ => Some(1),
            },
            action,
        };
        writer
            .session_command(request(
                "aux-held-invite",
                connect_client::session_control::Action::Invite {
                    participants: vec![held_peer.clone()],
                    medium: "voice".into(),
                    intent: "synchronous-engagement".into(),
                    capabilities: json!({"audio":["g.711-mu"]}),
                },
            ))
            .await
            .unwrap();
        second
            .session_command(request(
                "aux-held-accept",
                connect_client::session_control::Action::Accept {
                    capabilities: json!({"audio":["g.711-mu"]}),
                },
            ))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(4100)).await;
        let page = writer
            .list_conversations(ListQuery::default())
            .await
            .unwrap();
        assert!(page
            .conversations
            .iter()
            .any(|v| v.id == held.id && !v.closed));
        assert!(page
            .conversations
            .iter()
            .any(|v| v.id == extra.id && !v.closed));
        writer
            .session_command(request(
                "aux-held-end",
                connect_client::session_control::Action::End {
                    reason_code: 200,
                    reason: "normal-clearing".into(),
                },
            ))
            .await
            .unwrap();
        // Messages keep an otherwise ended engagement alive past its original
        // idle boundary; only the final quiet interval may close it.
        for n in 0..4 {
            tokio::time::sleep(Duration::from_millis(900)).await;
            writer
                .send_message(
                    &format!("aux-idle-message-{n}"),
                    &held.id,
                    &format!("aux_idle_message_{n}"),
                    connect_client::MessageDraft::text_all("reset idle interval"),
                )
                .await
                .unwrap();
        }
        let page = writer
            .list_conversations(ListQuery::default())
            .await
            .unwrap();
        assert!(page
            .conversations
            .iter()
            .any(|v| v.id == held.id && !v.closed));
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let page = writer
                    .list_conversations(ListQuery {
                        filter: ListFilter {
                            state: ConversationState::All,
                            ..Default::default()
                        },
                        ..Default::default()
                    })
                    .await
                    .unwrap();
                if let Some(v) = page
                    .conversations
                    .iter()
                    .find(|v| v.id == held.id && v.closed)
                {
                    assert_eq!(v.reason.as_deref(), Some("idle timeout"));
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
        // The otherwise empty default-60s room remains open; creation policy is immutable.
        let page = writer
            .list_conversations(ListQuery::default())
            .await
            .unwrap();
        assert!(page.conversations.iter().any(|v| v.id == default_cid
            && !v.closed
            && v.policy == ConversationPolicy::Ephemeral));
        let changed = writer
            .request_value(
                "conversation.create",
                json!({"tenant_id":"tnt_default","policy":"persistent"}),
                scope(None, Some("aux-default-ephemeral")),
            )
            .await
            .unwrap();
        assert!(matches!(changed.ack(), Err(ClientError::Application(_))));
        first.close();
        second.close();
        writer.close();
        f.server.shutdown().await.unwrap();
        for (id,checks) in [
            (69,vec!["explicit-create profile; unknown-cid message refuses without creating a room"]),
            (70,vec!["create/open correlation; explicit close; both subscribed Participants receive original closure"]),
            (71,vec!["accepted Session suppresses idle; Messages reset the ended-Session idle boundary; final quiet interval closes; persistent ignores idle interval"]),
            (72,vec!["policy set at create; changed logical create conflicts; no policy mutation"]),
            (73,vec!["tenant, policy, idle, metadata and explicit initial guest role; server-assigned cid"]),
            (74,vec!["missing tenant rejected by actual HTTP; persistent idle ignored; omitted initial participants"]),
            (75,vec!["typed opened shape, participant identity/kind/role, timestamp and exact metadata"]),
            (76,vec!["open correlation and per-list-request opened correlation"]),
            (77,vec!["typed closed shape with immutable reason/code/time identical to journal and SDK result"]),
            (78,vec!["typed closure multicast to two separately signed subscribed Participant identities"]),
            (79,vec!["tenant/identity/state/time filters, participant filter, bounded cursor pagination"]),
            (80,vec!["one matching opened per page; all correlate to list id; terminal ACK and next_cursor"]),
        ] {witness(algorithm,id,&checks);}
    }
}

#[tokio::test]
async fn production_auxiliary_session_identity_targets_keep_current_authority_and_original_intent()
{
    use connect_client::session_control::{Action, Request};
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_membership_scope(algorithm, 12, true).await;
        let mut writer = CanonicalClient::open(f.client.clone()).await.unwrap();
        let mut create = CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
        create.tenant_id = Some("tnt_default".into());
        let room = writer
            .create_conversation("aux-identity-room", create)
            .await
            .unwrap();
        let peer_id = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.peer.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let make = |operation: &str, sid: &str, targets: Vec<String>| Request {
            operation_id: operation.into(),
            conversation_id: room.id.clone(),
            session_id: sid.into(),
            by_participant: None,
            expected_revision: None,
            action: Action::Invite {
                participants: targets,
                medium: "voice".into(),
                intent: "synchronous-engagement".into(),
                capabilities: json!({"audio":["g.711-mu"]}),
            },
        };
        let request = make(
            "aux-identity-invite",
            "aux-identity-session",
            vec![f.peer.identity_id.clone()],
        );
        let first = writer.session_command(request.clone()).await.unwrap();
        assert_eq!(
            writer.session_command(request.clone()).await.unwrap(),
            first
        );
        let snapshot = writer
            .conversation_snapshot(&room.id, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap();
        assert!(snapshot
            .items
            .iter()
            .any(|v| v["kind"] == "session_participant"
                && v["data"]["session_id"] == "aux-identity-session"
                && v["data"]["participant_id"] == peer_id
                && v["data"]["state"] == "invited"));
        let mut changed = request.clone();
        if let Action::Invite { participants, .. } = &mut changed.action {
            *participants = vec![peer_id.clone()];
        }
        assert!(
            matches!(writer.session_command(changed).await,Err(ClientError::Application(ref e)) if e.code=="conflict")
        );
        let duplicate = make(
            "aux-duplicate-target",
            "aux-rejected-session",
            vec![f.peer.identity_id.clone(), peer_id.clone()],
        );
        assert!(
            matches!(writer.session_command(duplicate).await,Err(ClientError::Application(ref e)) if e.code=="forbidden")
        );
        let mut http =
            connect_client::canonical::CanonicalHttpClient::open(canonical_http_config(&f))
                .await
                .unwrap();
        let http_request = make(
            "aux-http-identity-invite",
            "aux-http-identity-session",
            vec![f.peer.identity_id.clone()],
        );
        let http_first = http.session_command(http_request.clone()).await.unwrap();
        assert_eq!(
            writer.session_command(http_request.clone()).await.unwrap(),
            http_first
        );
        let mut changed_http = http_request.clone();
        if let Action::Invite { participants, .. } = &mut changed_http.action {
            *participants = vec![peer_id.clone()];
        }
        assert!(
            matches!(http.session_command(changed_http).await,Err(ClientError::Application(ref e)) if e.code=="conflict")
        );
        let duplicate_http = make(
            "aux-http-duplicate-target",
            "aux-rejected-session",
            vec![f.peer.identity_id.clone(), peer_id.clone()],
        );
        assert!(
            matches!(http.session_command(duplicate_http).await,Err(ClientError::Application(ref e)) if e.code=="invalid-request")
        );
        http.session_command(Request {
            operation_id: "aux-http-identity-cancel".into(),
            conversation_id: room.id.clone(),
            session_id: http_request.session_id,
            by_participant: None,
            expected_revision: None,
            action: Action::Cancel {
                reason_code: 487,
                reason: "request-cancelled".into(),
            },
        })
        .await
        .unwrap();
        let outside = f
            .server
            .store
            .create_identity("aux-not-admitted")
            .unwrap()
            .id;
        assert!(
            matches!(writer.session_command(make("aux-unadmitted-target","aux-rejected-session",vec![outside])).await,Err(ClientError::Application(ref e)) if e.code=="forbidden")
        );
        let narrow = canonical_native_connections::party_with_scope(
            &f,
            &f.client.identity_id,
            124,
            vec!["data".into()],
            Scope::collaboration(vec![f.client.identity_id.clone()]),
        );
        let mut limited = CanonicalClient::open(narrow).await.unwrap();
        assert!(
            matches!(limited.session_command(make("aux-narrow-identity","aux-rejected-session",vec![f.peer.identity_id.clone()])).await,Err(ClientError::Application(ref e)) if e.code=="forbidden")
        );
        let snapshot = writer
            .conversation_snapshot(&room.id, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap();
        assert!(!snapshot
            .items
            .iter()
            .any(|v| v["kind"] == "session" && v["data"]["id"] == "aux-rejected-session"));
        let mut peer = CanonicalClient::open(canonical_native_connections::party_with_scope(
            &f,
            &f.peer.identity_id,
            125,
            vec!["data".into()],
            Scope::collaboration(vec!["*".into()]),
        ))
        .await
        .unwrap();
        let accepted = peer
            .session_command(Request {
                operation_id: "aux-identity-accept".into(),
                conversation_id: room.id.clone(),
                session_id: request.session_id.clone(),
                by_participant: Some(peer_id),
                expected_revision: Some(0),
                action: Action::Accept {
                    capabilities: json!({"audio":["g.711-mu"]}),
                },
            })
            .await
            .unwrap();
        assert!(
            matches!(accepted,Outcome::SessionControl(ref value) if value.session.state=="accepted")
        );
        assert_eq!(
            writer.session_command(request.clone()).await.unwrap(),
            first
        );
        writer
            .session_command(Request {
                operation_id: "aux-identity-end".into(),
                conversation_id: room.id,
                session_id: request.session_id,
                by_participant: None,
                expected_revision: Some(1),
                action: Action::End {
                    reason_code: 200,
                    reason: "normal-clearing".into(),
                },
            })
            .await
            .unwrap();
        limited.close();
        peer.close();
        writer.close();
        f.server.shutdown().await.unwrap();
    }
}

async fn session_end_observation(
    client: &mut CanonicalClient,
    cid: &str,
    sid: &str,
    request: &str,
    by: &str,
) -> connect_protocol::events::SessionEnded {
    let mut hint = None;
    for _ in 0..64 {
        let frame = client.receive().await.unwrap();
        assert_eq!(frame.scope(rvoip_uctp::v2::Scope::Conversation), Some(cid));
        if frame.scope(rvoip_uctp::v2::Scope::Reply).is_none() {
            assert_eq!(frame.scope(rvoip_uctp::v2::Scope::Session), Some(sid));
            assert!(frame.scope(rvoip_uctp::v2::Scope::Operation).is_none());
            assert!(frame.scope(rvoip_uctp::v2::Scope::Connection).is_none());
            match frame.kind() {
                "session.ended" => {
                    assert_eq!(frame.original()["payload"]["by"], by);
                    assert_eq!(frame.original()["payload"]["reason_code"], 200);
                    assert_eq!(frame.original()["payload"]["reason"], "normal-clearing");
                }
                "recording.vcon-ready" => {
                    assert!(uuid::Uuid::parse_str(
                        frame.original()["payload"]["vcon_handle"]["uuid"]
                            .as_str()
                            .unwrap()
                    )
                    .is_ok());
                }
                _ => panic!("unexpected independent hint: {:?}", frame.original()),
            }
            continue;
        }
        assert_eq!(frame.scope(rvoip_uctp::v2::Scope::Reply), Some(request));
        if frame.kind() == "session.ended" {
            assert_eq!(frame.scope(rvoip_uctp::v2::Scope::Session), Some(sid));
            assert!(frame.scope(rvoip_uctp::v2::Scope::Connection).is_none());
            let data = frame.original()["payload"].clone();
            assert_eq!(data["by"], by);
            assert_eq!(data["reason_code"], 200);
            assert_eq!(data["reason"], "normal-clearing");
            assert!(data.get("vcon_handle").is_none());
            assert!(hint.replace(data).is_none());
        }
        if frame.kind() == "conversation.event" {
            let event: connect_protocol::events::JournalEvent =
                serde_json::from_value(frame.original()["payload"]["data"].clone()).unwrap();
            if let JournalData::SessionEnded(fact) = event.data {
                assert_eq!(fact.session.id, sid);
                assert_eq!(fact.session.conversation_id, cid);
                assert_eq!(fact.session.state, "ended");
                assert_eq!(fact.by_participant.as_deref(), Some(by));
                assert_eq!(fact.by_membership_epoch, Some(1));
                assert_eq!(fact.reason_code, 200);
                assert_eq!(fact.reason, "normal-clearing");
                assert!(fact.ended_at <= event.observed_at);
                let hint = hint.expect("typed ended hint must precede its original cursor fact");
                assert_eq!(
                    chrono::DateTime::parse_from_rfc3339(hint["ended_at"].as_str().unwrap())
                        .unwrap()
                        .timestamp(),
                    fact.ended_at
                );
                return fact;
            }
        }
    }
    panic!("original Session terminal multicast not delivered")
}
#[tokio::test]
async fn production_auxiliary_session_terminal_multicast_survives_record_finalization() {
    use connect_client::{
        canonical::{CanonicalHttpClient, CanonicalRecordSelector},
        session_control::{Action, Request},
    };
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_membership_scope(algorithm, 12, true).await;
        let mut writer = CanonicalClient::open(f.client.clone()).await.unwrap();
        let mut input = CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
        input.tenant_id = Some("tnt_default".into());
        let room = writer
            .create_conversation("aux-terminal-room", input)
            .await
            .unwrap();
        let owner = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.client.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let sid = "aux-terminal-session";
        writer
            .session_command(Request {
                operation_id: "aux-terminal-invite".into(),
                conversation_id: room.id.clone(),
                session_id: sid.into(),
                by_participant: Some(owner.clone()),
                expected_revision: None,
                action: Action::Invite {
                    participants: vec![f.peer.identity_id.clone()],
                    medium: "voice".into(),
                    intent: "conference".into(),
                    capabilities: json!({"audio":["g.711-mu"]}),
                },
            })
            .await
            .unwrap();
        let config = canonical_native_connections::party_with_scope(
            &f,
            &f.peer.identity_id,
            220,
            vec!["data".into()],
            Scope::collaboration(vec!["*".into()]),
        );
        let mut peer = CanonicalClient::open(config.clone()).await.unwrap();
        peer.session_command(Request {
            operation_id: "aux-terminal-accept".into(),
            conversation_id: room.id.clone(),
            session_id: sid.into(),
            by_participant: None,
            expected_revision: Some(0),
            action: Action::Accept {
                capabilities: json!({"audio":["g.711-mu"]}),
            },
        })
        .await
        .unwrap();
        let baseline = writer
            .conversation_snapshot(&room.id, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap()
            .cursor;
        let peer_baseline = peer
            .conversation_snapshot(&room.id, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap()
            .cursor;
        let mut own = CanonicalClient::open(f.client.clone()).await.unwrap();
        let own_request = subscribe(&mut own, &room.id).await;
        let peer_request = subscribe(&mut peer, &room.id).await;
        let request = Request {
            operation_id: "aux-terminal-end".into(),
            conversation_id: room.id.clone(),
            session_id: sid.into(),
            by_participant: Some(owner.clone()),
            expected_revision: Some(1),
            action: Action::End {
                reason_code: 200,
                reason: "normal-clearing".into(),
            },
        };
        let ended = writer.session_command(request.clone()).await.unwrap();
        let one = session_end_observation(&mut own, &room.id, sid, &own_request, &owner).await;
        let two = session_end_observation(&mut peer, &room.id, sid, &peer_request, &owner).await;
        assert_eq!(one, two);
        assert!(matches!(&ended,Outcome::SessionControl(result) if result.session==one.session));
        let uuid = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let page = writer
                    .conversation_records(&room.id, serde_json::from_value(json!({})).unwrap())
                    .await
                    .unwrap();
                if let Some(connect_protocol::events::RecordAvailability::Ready { uuid, .. }) =
                    page.records.into_iter().find(|v| v.session_id() == sid)
                {
                    break uuid;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let mut ready_hint = false;
        let mut ready_fact = false;
        for _ in 0..64 {
            let frame = own.receive().await.unwrap();
            if frame.scope(rvoip_uctp::v2::Scope::Reply).is_none() {
                assert_eq!(frame.scope(rvoip_uctp::v2::Scope::Session), Some(sid));
                assert!(frame.scope(rvoip_uctp::v2::Scope::Operation).is_none());
                assert!(frame.scope(rvoip_uctp::v2::Scope::Connection).is_none());
                match frame.kind() {
                    "session.ended" => {
                        assert_eq!(frame.original()["payload"]["by"], owner);
                    }
                    "recording.vcon-ready" => {
                        assert_eq!(frame.original()["payload"]["vcon_handle"]["uuid"], uuid);
                    }
                    _ => panic!("unexpected independent hint: {:?}", frame.original()),
                }
                continue;
            }
            assert_eq!(
                frame.scope(rvoip_uctp::v2::Scope::Reply),
                Some(own_request.as_str())
            );
            if frame.kind() == "recording.vcon-ready" {
                assert_eq!(frame.scope(rvoip_uctp::v2::Scope::Session), Some(sid));
                assert_eq!(frame.original()["payload"]["vcon_handle"]["uuid"], uuid);
                ready_hint = true;
            }
            if frame.kind() == "conversation.event"
                && frame.original()["payload"]["kind"] == "record_ready"
            {
                assert!(ready_hint);
                ready_fact = true;
                break;
            }
        }
        assert!(ready_hint && ready_fact);
        let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
            .await
            .unwrap();
        let replay = http
            .subscribe_conversation(
                &room.id,
                serde_json::from_value(json!({"after":baseline,"live":false})).unwrap(),
            )
            .await
            .unwrap();
        replay.ack().unwrap();
        let hints = replay
            .frames
            .iter()
            .filter(|v| v.kind() == "session.ended")
            .collect::<Vec<_>>();
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].original()["payload"]["by"], owner);
        let replay_fact = replay
            .frames
            .iter()
            .filter(|v| v.kind() == "conversation.event")
            .map(|v| {
                serde_json::from_value::<connect_protocol::events::JournalEvent>(
                    v.original()["payload"]["data"].clone(),
                )
                .unwrap()
            })
            .find_map(|e| {
                if let JournalData::SessionEnded(f) = e.data {
                    Some(f)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(replay_fact, one);
        let _record = writer
            .fetch_record(
                &room.id,
                sid,
                CanonicalRecordSelector {
                    uuid: Some(uuid),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let current = writer
            .conversation_snapshot(&room.id, serde_json::from_value(json!({})).unwrap())
            .await
            .unwrap()
            .cursor;
        assert_eq!(writer.session_command(request).await.unwrap(), ended);
        let after = writer
            .subscribe_conversation(
                &room.id,
                serde_json::from_value(json!({"after":current,"live":false})).unwrap(),
            )
            .await
            .unwrap();
        after.ack().unwrap();
        assert!(!after.frames.iter().any(|v| v.kind() == "session.ended"));
        let limited = canonical_native_connections::party_with_scope(
            &f,
            &f.peer.identity_id,
            221,
            vec!["data".into()],
            Scope {
                operations: vec!["conversation.get".into(), "conversation.subscribe".into()],
                recipients: vec!["*".into()],
            },
        );
        let mut denied = CanonicalClient::open(limited).await.unwrap();
        let page = denied
            .subscribe_conversation(
                &room.id,
                serde_json::from_value(json!({"after":peer_baseline,"live":false})).unwrap(),
            )
            .await
            .unwrap();
        page.ack().unwrap();
        assert!(!page
            .frames
            .iter()
            .any(|v| v.kind() == "session.ended" || v.kind() == "recording.vcon-ready"));
        denied.close();
        own.close();
        peer.close();
        writer.close();
        f.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn production_auxiliary_invitation_sources_preserve_current_target_slots() {
    use connect_client::{
        canonical::CanonicalHttpClient,
        session_control::{Action, Request},
    };
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_membership_scope(algorithm, 12, true).await;
        let mut writer = CanonicalClient::open(f.client.clone()).await.unwrap();
        let mut input = CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
        input.tenant_id = Some("tnt_default".into());
        let room = writer
            .create_conversation("invitation-source-room", input)
            .await
            .unwrap();
        let peer_id = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.peer.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let request = Request {
            operation_id: "invitation-source-original".into(),
            conversation_id: room.id.clone(),
            session_id: "invitation-source-session".into(),
            by_participant: None,
            expected_revision: None,
            action: Action::Invite {
                participants: vec![f.peer.identity_id.clone()],
                medium: "voice".into(),
                intent: "conference".into(),
                capabilities: json!({"audio":["g.711-mu"],"metadata":{"claimed_actor":"not-authority"}}),
            },
        };
        let first = writer.session_command(request.clone()).await.unwrap();
        let record = writer
            .operation_get(&request.operation_id, scope(Some(&room.id), None))
            .await
            .unwrap();
        let sources = f
            .server
            .store
            .session_invitation_sources(record.record_id.as_ref().unwrap().as_str())
            .unwrap();
        assert_eq!(sources.len(), 1);
        let config = canonical_native_connections::party_with_scope(
            &f,
            &f.peer.identity_id,
            240,
            vec!["data".into()],
            Scope::collaboration(vec!["*".into()]),
        );
        let credential = f
            .server
            .store
            .authenticate(&config.credential)
            .unwrap()
            .credential_id;
        let read = f
            .server
            .store
            .try_session_invitation_read(&sources[0], &credential)
            .unwrap();
        let original = read.metadata().clone();
        assert_eq!(read.identity_id(), f.peer.identity_id);
        assert_eq!(original.from_identity_id, f.client.identity_id);
        assert_eq!(original.target_participant_id, peer_id);
        assert_eq!(original.target_membership_epoch, 1);
        drop(read);
        assert_eq!(
            writer.session_command(request.clone()).await.unwrap(),
            first
        );
        let replay = f
            .server
            .store
            .session_invitation_sources(record.record_id.as_ref().unwrap().as_str())
            .unwrap();
        let read = f
            .server
            .store
            .try_session_invitation_read(&replay[0], &credential)
            .unwrap();
        assert_eq!(read.metadata(), &original);
        drop(read);
        let mut peer = CanonicalClient::open(config).await.unwrap();
        peer.session_command(Request {
            operation_id: "invitation-source-accept".into(),
            conversation_id: room.id.clone(),
            session_id: request.session_id.clone(),
            by_participant: None,
            expected_revision: Some(0),
            action: Action::Accept {
                capabilities: json!({"audio":["g.711-mu"]}),
            },
        })
        .await
        .unwrap();
        assert_eq!(
            f.server
                .store
                .try_session_invitation_read(&sources[0], &credential)
                .unwrap_err()
                .code,
            "forbidden"
        );
        peer.session_command(Request {
            operation_id: "invitation-source-leave".into(),
            conversation_id: room.id.clone(),
            session_id: request.session_id.clone(),
            by_participant: None,
            expected_revision: Some(1),
            action: Action::Leave {
                participant_id: peer_id,
            },
        })
        .await
        .unwrap();
        let mut next = request;
        next.operation_id = "invitation-source-new".into();
        next.expected_revision = Some(2);
        let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
            .await
            .unwrap();
        http.session_command(next.clone()).await.unwrap();
        let record = writer
            .operation_get(&next.operation_id, scope(Some(&room.id), None))
            .await
            .unwrap();
        let fresh = f
            .server
            .store
            .session_invitation_sources(record.record_id.as_ref().unwrap().as_str())
            .unwrap();
        assert_eq!(fresh.len(), 1);
        let read = f
            .server
            .store
            .try_session_invitation_read(&fresh[0], &credential)
            .unwrap();
        assert_ne!(read.metadata().invitation_id, original.invitation_id);
        assert_eq!(read.metadata().session_id, original.session_id);
        drop(read);
        assert_eq!(
            f.server
                .store
                .try_session_invitation_read(&sources[0], &credential)
                .unwrap_err()
                .code,
            "forbidden"
        );
        peer.close();
        writer.close();
        f.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn production_auxiliary_session_end_by_current_noninitiator_0087() {
    use connect_client::{
        canonical::CanonicalHttpClient,
        session_control::{Action, Request},
    };
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_membership_scope(algorithm, 12, true).await;
        let mut writer = CanonicalClient::open(f.client.clone()).await.unwrap();
        let mut input = CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
        input.tenant_id = Some("tnt_default".into());
        let room = writer
            .create_conversation("participant-end-room", input)
            .await
            .unwrap();
        let owner = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.client.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let participant = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.peer.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let config = canonical_native_connections::party_with_scope(
            &f,
            &f.peer.identity_id,
            241,
            vec!["data".into()],
            Scope {
                operations: vec![
                    "session.accept".into(),
                    "session.end".into(),
                    "session_get".into(),
                    "conversation.get".into(),
                    "conversation.subscribe".into(),
                ],
                recipients: vec!["*".into()],
            },
        );
        let mut peer = CanonicalClient::open(config.clone()).await.unwrap();
        let mut own = CanonicalClient::open(f.client.clone()).await.unwrap();
        let mut observer = CanonicalClient::open(config.clone()).await.unwrap();
        for via in ["native", "http"] {
            let sid = format!("participant-end-{via}");
            writer
                .session_command(Request {
                    operation_id: format!("{sid}-invite"),
                    conversation_id: room.id.clone(),
                    session_id: sid.clone(),
                    by_participant: Some(owner.clone()),
                    expected_revision: None,
                    action: Action::Invite {
                        participants: vec![participant.clone()],
                        medium: "voice".into(),
                        intent: "conference".into(),
                        capabilities: json!({"audio":["g.711-mu"]}),
                    },
                })
                .await
                .unwrap();
            peer.session_command(Request {
                operation_id: format!("{sid}-accept"),
                conversation_id: room.id.clone(),
                session_id: sid.clone(),
                by_participant: Some(participant.clone()),
                expected_revision: Some(0),
                action: Action::Accept {
                    capabilities: json!({"audio":["g.711-mu"]}),
                },
            })
            .await
            .unwrap();
            // Each subscription ends after the terminal page on close below;
            // use fresh physical observers for the next Session in this room.
            let own_query = subscribe(&mut own, &room.id).await;
            let peer_query = subscribe(&mut observer, &room.id).await;
            let request = Request {
                operation_id: format!("{sid}-end"),
                conversation_id: room.id.clone(),
                session_id: sid.clone(),
                by_participant: Some(participant.clone()),
                expected_revision: Some(1),
                action: Action::End {
                    reason_code: 200,
                    reason: "normal-clearing".into(),
                },
            };
            let mut http_config = canonical_http_config(&f);
            http_config.credential = config.credential.clone();
            let mut http = CanonicalHttpClient::open(http_config).await.unwrap();
            let ended = if via == "native" {
                peer.session_command(request.clone()).await.unwrap()
            } else {
                http.session_command(request.clone()).await.unwrap()
            };
            let one =
                session_end_observation(&mut own, &room.id, &sid, &own_query, &participant).await;
            let two =
                session_end_observation(&mut observer, &room.id, &sid, &peer_query, &participant)
                    .await;
            assert_eq!(one, two);
            assert!(
                matches!(&ended,Outcome::SessionControl(result) if result.session==one.session)
            );
            // Recover exactly the same operation on the other public transport.
            let replay = if via == "native" {
                http.session_command(request).await.unwrap()
            } else {
                peer.session_command(request).await.unwrap()
            };
            assert_eq!(replay, ended);
            own.close();
            observer.close();
            own = CanonicalClient::open(f.client.clone()).await.unwrap();
            observer = CanonicalClient::open(config.clone()).await.unwrap();
        }
        observer.close();
        own.close();
        peer.close();
        writer.close();
        f.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn production_auxiliary_owned_invitations_and_correlated_decisions() {
    use connect_client::{
        canonical::SessionInvitationAnswer,
        session_control::{Action, Request},
    };
    use connect_protocol::events::SnapshotItem;
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_membership_scope(algorithm, 12, true).await;
        let mut owner = CanonicalClient::open(f.client.clone()).await.unwrap();
        let mut input = CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
        input.tenant_id = Some("tnt_default".into());
        let room = owner
            .create_conversation("owned-invitation-room", input)
            .await
            .unwrap();
        let from = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.client.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let to = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.peer.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let config = canonical_native_connections::party_with_scope(
            &f,
            &f.peer.identity_id,
            242,
            vec!["data".into()],
            Scope {
                operations: vec![
                    "session.accept".into(),
                    "session.reject".into(),
                    "session_get".into(),
                    "conversation.get".into(),
                    "conversation.subscribe".into(),
                ],
                recipients: vec!["*".into()],
            },
        );
        let mut peer = CanonicalClient::open(config.clone()).await.unwrap();
        let capabilities = json!({"audio":["g.711-mu"],"video":["vp8"],"screen":["vp8"],"data":["reliable-ordered"],"future":{"claimed_actor":"never-authority"}});
        for (index, medium) in [
            "voice",
            "video",
            "voice+video",
            "screen-share",
            "text-chat",
            "mixed",
        ]
        .into_iter()
        .enumerate()
        {
            let sid = format!("owned-invitation-{index}");
            let request = Request {
                operation_id: format!("{sid}-original"),
                conversation_id: room.id.clone(),
                session_id: sid.clone(),
                by_participant: Some(from.clone()),
                expected_revision: None,
                action: Action::Invite {
                    participants: vec![f.peer.identity_id.clone()],
                    medium: medium.into(),
                    intent: "synchronous-engagement".into(),
                    capabilities: capabilities.clone(),
                },
            };
            let original = owner.session_command(request.clone()).await.unwrap();
            // The target has no media Connection or subscription. Delivery uses
            // its authenticated idle physical binding and fresh metadata grant.
            let invitation = peer.receive_session_invitation().await.unwrap();
            assert_eq!(invitation.kind(), "session.invite");
            assert_eq!(
                invitation.scope(rvoip_uctp::v2::Scope::Conversation),
                Some(room.id.as_str())
            );
            assert_eq!(
                invitation.scope(rvoip_uctp::v2::Scope::Session),
                Some(sid.as_str())
            );
            assert!(invitation
                .scope(rvoip_uctp::v2::Scope::Connection)
                .is_none());
            assert_eq!(invitation.original()["payload"]["from"], from);
            assert_eq!(invitation.original()["payload"]["to"], json!([to]));
            assert_eq!(invitation.original()["payload"]["medium"], medium);
            assert_eq!(
                invitation.original()["payload"]["intent"],
                "synchronous-engagement"
            );
            assert_eq!(
                invitation.original()["payload"]["capabilities_offer"],
                capabilities
            );
            if index == 0 {
                // A separately authenticated physical peer cannot consume this
                // already registered waiter even with the same identity/key.
                let mut other = CanonicalClient::open(config.clone()).await.unwrap();
                other
                    .answer_session_invitation(
                        &invitation,
                        SessionInvitationAnswer::Accept {
                            capabilities: capabilities.clone(),
                        },
                    )
                    .await
                    .unwrap();
                assert_eq!(other.receive().await.unwrap().kind(), "error");
                other.close();
                // HTTP has no authority to route a reply into a native waiter.
                let http = tokio::task::spawn_blocking(|| {
                    reqwest::Client::builder().no_proxy().build().unwrap()
                })
                .await
                .unwrap();
                let refused=http.post(format!("http://{}/v2/commands",f.server.http_address.unwrap())).bearer_auth(&config.credential).json(&json!({"v":2,"type":"session.accept","id":format!("env_{}",uuid::Uuid::new_v4().simple()),"ts":chrono::Utc::now(),"cid":room.id,"sid":sid,"operation_id":invitation.original()["operation_id"],"in_reply_to":invitation.id(),"payload":{"by":to,"capabilities_answer":capabilities}})).send().await.unwrap();
                assert!(refused.status().is_client_error());
                let mut wrong = invitation.original().clone();
                wrong["sid"] = json!("wrong-session");
                let wrong = connect_client::canonical::Envelope::from_value(wrong).unwrap();
                peer.answer_session_invitation(
                    &wrong,
                    SessionInvitationAnswer::Accept {
                        capabilities: capabilities.clone(),
                    },
                )
                .await
                .unwrap();
                assert_eq!(peer.receive().await.unwrap().kind(), "error");
            }
            peer.answer_session_invitation(
                &invitation,
                SessionInvitationAnswer::Accept {
                    capabilities: capabilities.clone(),
                },
            )
            .await
            .unwrap();
            let accepted = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let page = owner
                        .conversation_snapshot(&room.id, serde_json::from_value(json!({})).unwrap())
                        .await
                        .unwrap();
                    if let Some(session) = page.items.into_iter().find_map(|item| {
                        match serde_json::from_value::<SnapshotItem>(item).unwrap() {
                            SnapshotItem::Session(s) if s.id == sid && s.state == "accepted" => {
                                Some(s)
                            }
                            _ => None,
                        }
                    }) {
                        break session;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            assert_eq!(accepted.topology_revision, 1);
            assert_eq!(accepted.capabilities_offer, Some(capabilities.clone()));
            assert_eq!(owner.session_command(request).await.unwrap(), original);
            // A late duplicate signed observation has no waiter and cannot
            // enter the command path or advance the Session a second time.
            peer.answer_session_invitation(
                &invitation,
                SessionInvitationAnswer::Accept {
                    capabilities: capabilities.clone(),
                },
            )
            .await
            .unwrap();
            assert_eq!(peer.receive().await.unwrap().kind(), "error");
            let page = owner
                .conversation_snapshot(&room.id, serde_json::from_value(json!({})).unwrap())
                .await
                .unwrap();
            assert!(page.items.into_iter().any(|item|matches!(serde_json::from_value::<SnapshotItem>(item).unwrap(),SnapshotItem::Session(s) if s.id==sid && s.topology_revision==1 && s.state=="accepted")));
        }
        for cancelled in [false, true] {
            let sid = if cancelled {
                "owned-invitation-cancelled"
            } else {
                "owned-invitation-rejected"
            };
            owner
                .session_command(Request {
                    operation_id: format!("{sid}-original"),
                    conversation_id: room.id.clone(),
                    session_id: sid.into(),
                    by_participant: None,
                    expected_revision: None,
                    action: Action::Invite {
                        participants: vec![to.clone()],
                        medium: "voice".into(),
                        intent: "synchronous-engagement".into(),
                        capabilities: capabilities.clone(),
                    },
                })
                .await
                .unwrap();
            let invitation = peer.receive_session_invitation().await.unwrap();
            assert_eq!(invitation.kind(), "session.invite");
            if cancelled {
                owner
                    .session_command(Request {
                        operation_id: format!("{sid}-cancel"),
                        conversation_id: room.id.clone(),
                        session_id: sid.into(),
                        by_participant: None,
                        expected_revision: Some(0),
                        action: Action::Cancel {
                            reason_code: 487,
                            reason: "request-cancelled".into(),
                        },
                    })
                    .await
                    .unwrap();
            }
            peer.answer_session_invitation(
                &invitation,
                SessionInvitationAnswer::Reject {
                    reason_code: 486,
                    reason: "busy".into(),
                },
            )
            .await
            .unwrap();
            let expected = if cancelled { "cancelled" } else { "rejected" };
            let ended = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let page = owner
                        .conversation_snapshot(&room.id, serde_json::from_value(json!({})).unwrap())
                        .await
                        .unwrap();
                    if let Some(s) = page.items.into_iter().find_map(|item| {
                        match serde_json::from_value::<SnapshotItem>(item).unwrap() {
                            SnapshotItem::Session(s) if s.id == sid && s.state == expected => {
                                Some(s)
                            }
                            _ => None,
                        }
                    }) {
                        break s;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            assert_eq!(ended.topology_revision, 1);
            assert_eq!(
                ended.ended_reason.as_deref(),
                Some(if cancelled {
                    "request-cancelled"
                } else {
                    "busy"
                })
            );
        }
        peer.close();
        owner.close();
        f.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn production_owned_invitation_decisions_require_configured_human_consent_and_new_waiter() {
    use connect_client::{
        application::ApplicationClient,
        canonical::SessionInvitationAnswer,
        session_control::{Action, Request},
    };
    use connect_protocol::{
        events::SnapshotItem,
        user_delegation::{UserAssuranceRule, UserDelegationRequest},
    };
    for algorithm in ["EdDSA", "ES256"] {
        let mut f = Fixture::with_membership_scope(algorithm, 12, true).await;
        let mut owner = CanonicalClient::open(f.client.clone()).await.unwrap();
        let mut input = CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
        input.tenant_id = Some("tnt_default".into());
        let room = owner
            .create_conversation("owned-consent-room", input)
            .await
            .unwrap();
        owner.close();
        drop(owner);
        let user = f
            .server
            .store
            .create_identity("owned-invitation-human-consenter")
            .unwrap()
            .id;
        let operations = vec![
            "credential.delegate".into(),
            "session.accept".into(),
            "session.reject".into(),
            "session_get".into(),
            "conversation.get".into(),
            "conversation.subscribe".into(),
        ];
        let human = f
            .server
            .store
            .issue(
                &user,
                Scope {
                    operations: operations.clone(),
                    recipients: vec!["*".into()],
                },
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap();
        f.config.bind = f.server.address;
        f.config.http_bind = f.server.http_address;
        f.config.user_assurance = ["session.accept", "session.reject"]
            .into_iter()
            .map(|command| UserAssuranceRule {
                conversation_id: room.id.clone(),
                command: command.into(),
                user_id: user.clone(),
            })
            .collect();
        f.server.shutdown().await.unwrap();
        f.server = RunningCanonicalServer::start(f.config.clone())
            .await
            .unwrap();
        f.client.endpoint = f.server.address;
        let mut owner = CanonicalClient::open(f.client.clone()).await.unwrap();
        let scope = Scope {
            operations: operations[1..].to_vec(),
            recipients: vec![f.client.identity_id.clone(), f.peer.identity_id.clone()],
        };
        let config = canonical_native_connections::party_with_scope(
            &f,
            &f.peer.identity_id,
            243,
            vec!["data".into()],
            scope.clone(),
        );
        let mut peer = CanonicalClient::open(config).await.unwrap();
        let mut original_invites = Vec::new();
        for reject in [false, true] {
            let sid = if reject {
                "owned-consent-reject"
            } else {
                "owned-consent-accept"
            };
            let request = Request {
                operation_id: format!("{sid}-invite"),
                conversation_id: room.id.clone(),
                session_id: sid.into(),
                by_participant: None,
                expected_revision: None,
                action: Action::Invite {
                    participants: vec![f.peer.identity_id.clone()],
                    medium: "voice".into(),
                    intent: "synchronous-engagement".into(),
                    capabilities: json!({"audio":["g.711-mu"]}),
                },
            };
            owner.session_command(request.clone()).await.unwrap();
            let invitation = peer.receive_session_invitation().await.unwrap();
            let answer = if reject {
                SessionInvitationAnswer::Reject {
                    reason_code: 486,
                    reason: "busy".into(),
                }
            } else {
                SessionInvitationAnswer::Accept {
                    capabilities: json!({"audio":["g.711-mu"]}),
                }
            };
            peer.answer_session_invitation(&invitation, answer)
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(80)).await;
            let page = owner
                .conversation_snapshot(&room.id, serde_json::from_value(json!({})).unwrap())
                .await
                .unwrap();
            assert!(page.items.into_iter().any(|item|matches!(serde_json::from_value::<SnapshotItem>(item).unwrap(),SnapshotItem::Session(s) if s.id==sid && s.state=="invited" && s.topology_revision==0)),"An identified observation must not bypass the configured human-consent floor");
            original_invites.push((request, invitation, reject));
        }
        peer.close();
        drop(peer);
        let mut application_config = canonical_http_config(&f);
        application_config.credential = human.token;
        let mut application = ApplicationClient::open(application_config).await.unwrap();
        let consent = application
            .issue_user_delegation(
                &user,
                UserDelegationRequest {
                    operation_id: "owned-invitation-consent".into(),
                    identity_id: f.peer.identity_id.clone(),
                    operations: scope.operations.clone(),
                    recipients: scope.recipients.clone(),
                    expires_at: chrono::Utc::now().timestamp() + 300,
                },
            )
            .await
            .unwrap();
        let mut current = canonical_native_connections::party_with_scope(
            &f,
            &f.peer.identity_id,
            244,
            vec!["data".into()],
            scope,
        );
        let old = f.server.store.authenticate(&current.credential).unwrap();
        let key = f
            .server
            .store
            .resolve_peer_signing_key(
                &old.credential_id,
                "tnt_default",
                &current.signing_key_id,
                &current.signing_algorithm,
            )
            .unwrap();
        let admin = f.server.store.authenticate(&f.admin).unwrap();
        f.server
            .store
            .execute_provisioning(
                &admin.credential_id,
                ProvisioningCommand::SigningKey {
                    operation_id: "owned-consent-enrollment".into(),
                    identity_id: f.peer.identity_id.clone(),
                    credential_id: consent.credential_id,
                    device_id: current.device_id.clone(),
                    key_id: current.signing_key_id.clone(),
                    algorithm: if algorithm == "ES256" {
                        SigningAlgorithm::ES256
                    } else {
                        SigningAlgorithm::EdDSA
                    },
                    public_key: URL_SAFE_NO_PAD.encode(key.public_key),
                    not_before: key.not_before,
                    expires_at: key.expires_at,
                    expected_revision: Some(0),
                },
            )
            .unwrap();
        current.credential = consent.credential;
        let mut peer = CanonicalClient::open(current).await.unwrap();
        for (request, previous, reject) in original_invites {
            owner.session_command(request.clone()).await.unwrap();
            let invitation = peer.receive_session_invitation().await.unwrap();
            assert_ne!(invitation.id(), previous.id());
            assert_eq!(
                invitation.scope(rvoip_uctp::v2::Scope::Operation),
                previous.scope(rvoip_uctp::v2::Scope::Operation)
            );
            peer.answer_session_invitation(
                &previous,
                SessionInvitationAnswer::Accept {
                    capabilities: json!({"audio":["g.711-mu"]}),
                },
            )
            .await
            .unwrap();
            assert_eq!(peer.receive().await.unwrap().kind(), "error");
            let answer = if reject {
                SessionInvitationAnswer::Reject {
                    reason_code: 486,
                    reason: "busy".into(),
                }
            } else {
                SessionInvitationAnswer::Accept {
                    capabilities: json!({"audio":["g.711-mu"]}),
                }
            };
            peer.answer_session_invitation(&invitation, answer)
                .await
                .unwrap();
            let expected = if reject { "rejected" } else { "accepted" };
            tokio::time::timeout(Duration::from_secs(5),async {loop {
                let page=owner.conversation_snapshot(&room.id,serde_json::from_value(json!({})).unwrap()).await.unwrap();
                if page.items.into_iter().any(|item|matches!(serde_json::from_value::<SnapshotItem>(item).unwrap(),SnapshotItem::Session(s) if s.id==request.session_id && s.state==expected && s.topology_revision==1)){break;}
                tokio::time::sleep(Duration::from_millis(10)).await;
            }}).await.unwrap();
        }
        peer.close();
        owner.close();
        f.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn production_owned_decisions_relay_to_exact_caller_and_recover_after_disconnect() {
    use connect_client::canonical::SessionInvitationAnswer;
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_membership_scope(algorithm, 12, true).await;
        let mut owner = CanonicalClient::open(f.client.clone()).await.unwrap();
        let mut sibling = CanonicalClient::open(f.client.clone()).await.unwrap();
        let mut input = CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
        input.tenant_id = Some("tnt_default".into());
        let room = owner
            .create_conversation("relay-caller-room", input)
            .await
            .unwrap();
        let from = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.client.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let to = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.peer.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let config = canonical_native_connections::party_with_scope(
            &f,
            &f.peer.identity_id,
            245,
            vec!["data".into()],
            Scope::collaboration(vec!["*".into()]),
        );
        let mut peer = CanonicalClient::open(config).await.unwrap();
        for (index, reject, disconnect) in [(0, false, false), (1, true, false), (2, false, true)] {
            let sid = format!("relay-caller-{index}");
            let op = format!("{sid}-original");
            let body = json!({"from":from,"to":[to],"medium":"voice","intent":"synchronous-engagement","capabilities_offer":{"audio":["g.711-mu"]}});
            let scope = CommandScope {
                conversation_id: Some(room.id.clone()),
                session_id: Some(sid.clone()),
                operation_id: Some(op.clone()),
                ..Default::default()
            };
            let submitted = owner
                .request_value("session.invite", body.clone(), scope.clone())
                .await
                .unwrap();
            submitted.ack().unwrap();
            let invitation = peer.receive_session_invitation().await.unwrap();
            if disconnect {
                owner.close();
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
            let answer = if reject {
                SessionInvitationAnswer::Reject {
                    reason_code: 486,
                    reason: "busy".into(),
                }
            } else {
                SessionInvitationAnswer::Accept {
                    capabilities: json!({"audio":["g.711-mu"],"future":{"selected":"retained"}}),
                }
            };
            peer.answer_session_invitation(&invitation, answer)
                .await
                .unwrap();
            if disconnect {
                owner = CanonicalClient::open(f.client.clone()).await.unwrap();
                // Give the owned original response time to commit after its
                // caller's transport disappears, then recover by logical retry.
                tokio::time::sleep(Duration::from_millis(80)).await;
            }
            let first = if disconnect {
                let replay = owner
                    .request_value("session.invite", body.clone(), scope.clone())
                    .await
                    .unwrap();
                replay.ack().unwrap();
                replay
            } else {
                submitted
            };
            let relayed = owner.receive_control().await.unwrap();
            assert_eq!(
                relayed.kind(),
                if reject {
                    "session.reject"
                } else {
                    "session.accept"
                }
            );
            assert_eq!(
                relayed.scope(rvoip_uctp::v2::Scope::Reply),
                Some(first.request_id.as_str())
            );
            assert_eq!(
                relayed.scope(rvoip_uctp::v2::Scope::Conversation),
                Some(room.id.as_str())
            );
            assert_eq!(
                relayed.scope(rvoip_uctp::v2::Scope::Session),
                Some(sid.as_str())
            );
            assert_eq!(
                relayed.scope(rvoip_uctp::v2::Scope::Operation),
                Some(op.as_str())
            );
            assert!(relayed.scope(rvoip_uctp::v2::Scope::Connection).is_none());
            assert_eq!(relayed.original()["payload"]["by"], to);
            if reject {
                assert_eq!(relayed.original()["payload"]["reason_code"], 486);
                assert_eq!(relayed.original()["payload"]["reason"], "busy");
            } else {
                assert_eq!(
                    relayed.original()["payload"]["capabilities_answer"],
                    json!({"audio":["g.711-mu"],"future":{"selected":"retained"}})
                );
            }
            let replay = owner
                .request_value("session.invite", body, scope)
                .await
                .unwrap();
            replay.ack().unwrap();
            assert_ne!(replay.request_id, first.request_id);
            let recovered = owner.receive_control().await.unwrap();
            assert_eq!(recovered.kind(), relayed.kind());
            assert_eq!(
                recovered.original()["payload"],
                relayed.original()["payload"]
            );
            assert_eq!(
                recovered.scope(rvoip_uctp::v2::Scope::Reply),
                Some(replay.request_id.as_str())
            );
        }
        // Same identity, credential and Device never supply the original
        // caller's physical binding. This final timed read retires that sibling.
        assert!(
            tokio::time::timeout(Duration::from_millis(60), sibling.receive_control())
                .await
                .is_err()
        );
        sibling.close();
        peer.close();
        owner.close();
        f.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn production_cancellation_stops_at_first_acceptance_and_requires_end_through_native_and_http(
) {
    use connect_client::{
        canonical::CanonicalHttpClient,
        events::SnapshotItem,
        session_control::{Action, Request},
    };
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_membership_scope(algorithm, 12, true).await;
        let admin = f.server.store.authenticate(&f.admin).unwrap();
        let ProvisioningOutcome::Identity {
            identity_id: pending_identity,
            ..
        } = f
            .server
            .store
            .execute_provisioning(
                &admin.credential_id,
                ProvisioningCommand::Identity {
                    operation_id: "cancel-boundary-pending".into(),
                    alias: "cancel-boundary-pending".into(),
                    kind: IdentityKind::Human,
                    display_name: None,
                },
            )
            .unwrap()
        else {
            panic!("pending identity")
        };
        let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
        let mut create =
            CreateRequest::persistent(vec![f.peer.identity_id.clone(), pending_identity.clone()]);
        create.tenant_id = Some("tnt_default".into());
        let room = native
            .create_conversation("cancel-boundary-room", create)
            .await
            .unwrap();
        let peer_pid = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.peer.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let pending_pid = room
            .participants
            .iter()
            .find(|p| p.identity_id == pending_identity)
            .unwrap()
            .participant_id
            .clone();
        let mut caller_http = CanonicalHttpClient::open(canonical_http_config(&f))
            .await
            .unwrap();
        let mut peer_config = canonical_http_config(&f);
        peer_config.credential = f
            .server
            .store
            .issue(
                &f.peer.identity_id,
                Scope::collaboration(vec![f.client.identity_id.clone(), pending_identity.clone()]),
                chrono::Utc::now().timestamp() + 600,
            )
            .unwrap()
            .token;
        let mut peer_http = CanonicalHttpClient::open(peer_config).await.unwrap();
        for accepted in [false, true] {
            let sid = if accepted {
                "cancel-after-accept"
            } else {
                "cancel-before-accept"
            };
            let request = |operation: &str, expected_revision, action| Request {
                operation_id: format!("{sid}-{operation}"),
                conversation_id: room.id.clone(),
                session_id: sid.into(),
                by_participant: None,
                expected_revision,
                action,
            };
            let Outcome::SessionControl(invited) = native
                .session_command(request(
                    "invite",
                    None,
                    Action::Invite {
                        participants: vec![peer_pid.clone(), pending_pid.clone()],
                        medium: "voice".into(),
                        intent: "conference".into(),
                        capabilities: json!({"audio":["opus"]}),
                    },
                ))
                .await
                .unwrap()
            else {
                panic!("invitation receipt")
            };
            assert_eq!(invited.session.state, "invited");
            let cancel = request(
                "cancel",
                if accepted { Some(1) } else { None },
                Action::Cancel {
                    reason_code: 487,
                    reason: "request-cancelled".into(),
                },
            );
            if accepted {
                let Outcome::SessionControl(accepted) = peer_http
                    .session_command(request(
                        "accept",
                        Some(0),
                        Action::Accept {
                            capabilities: json!({"audio":["opus"]}),
                        },
                    ))
                    .await
                    .unwrap()
                else {
                    panic!("acceptance receipt")
                };
                assert_eq!(accepted.session.state, "accepted");
                assert_eq!(accepted.session.topology_revision, 1);
                // Use the current revision while another invitee remains pending;
                // this must fail for the acceptance boundary, not a stale CAS.
                assert!(matches!(native.session_command(cancel.clone()).await,
                    Err(ClientError::Application(ref e)) if e.code == "conflict"));
                let mut http_cancel = cancel.clone();
                http_cancel.operation_id.push_str("-fresh-http");
                assert!(matches!(caller_http.session_command(http_cancel).await,
                    Err(ClientError::Application(ref e)) if e.code == "conflict"));
            } else {
                let receipt = native.session_command(cancel.clone()).await.unwrap();
                let Outcome::SessionDecision(cancelled) = &receipt else {
                    panic!("cancellation receipt")
                };
                assert_eq!(cancelled.state, "cancelled");
                assert_eq!(Some(cancelled.reason.as_str()), Some("request-cancelled"));
                assert_eq!(cancelled.revision, 1);
                assert_eq!(
                    caller_http.session_command(cancel.clone()).await.unwrap(),
                    receipt
                );
                assert!(
                    matches!(peer_http.session_command(request("late-accept", Some(1), Action::Accept {
                    capabilities: json!({"audio":["opus"]}),
                })).await, Err(ClientError::Application(ref e)) if e.code == "conflict")
                );
            }
            let snapshot = native
                .conversation_snapshot(&room.id, serde_json::from_value(json!({})).unwrap())
                .await
                .unwrap();
            let items: Vec<SnapshotItem> = snapshot
                .items
                .into_iter()
                .map(|item| serde_json::from_value(item).unwrap())
                .collect();
            let expected_state = if accepted { "accepted" } else { "cancelled" };
            assert!(items
                .iter()
                .any(|item| matches!(item, SnapshotItem::Session(s)
                if s.id == sid && s.state == expected_state && s.topology_revision == 1)));
            assert!(items.iter().any(|item| matches!(item, SnapshotItem::SessionParticipant(p)
                if p.session_id == sid && p.participant_id == pending_pid && p.state == if accepted { "invited" } else { "left" })));
            if accepted {
                assert!(items.iter().any(|item| matches!(item, SnapshotItem::SessionParticipant(p)
                    if p.session_id == sid && p.participant_id == peer_pid && p.state == "present")));
                let Outcome::SessionControl(ended) = native
                    .session_command(request(
                        "end",
                        Some(1),
                        Action::End {
                            reason_code: 200,
                            reason: "normal-clear".into(),
                        },
                    ))
                    .await
                    .unwrap()
                else {
                    panic!("end receipt")
                };
                assert_eq!(ended.session.state, "ended");
                assert_eq!(ended.session.topology_revision, 2);
            }
        }
        native.close();
        f.server.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn production_cancellation_observations_correlate_each_physical_invitation_and_keep_the_durable_boundary(
) {
    use connect_client::{
        canonical::{CanonicalHttpClient, SessionInvitationAnswer},
        session_control::{Action, Request},
    };
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_membership_scope(algorithm, 12, true).await;
        let mut owner = CanonicalClient::open(f.client.clone()).await.unwrap();
        let mut create = CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
        create.tenant_id = Some("tnt_default".into());
        let room = owner
            .create_conversation("cancel-observation-room", create)
            .await
            .unwrap();
        let from = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.client.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let target = room
            .participants
            .iter()
            .find(|p| p.identity_id == f.peer.identity_id)
            .unwrap()
            .participant_id
            .clone();
        let config = canonical_native_connections::party_with_scope(
            &f,
            &f.peer.identity_id,
            245,
            vec!["data".into()],
            Scope::collaboration(vec![f.client.identity_id.clone()]),
        );
        let mut a = CanonicalClient::open(config.clone()).await.unwrap();
        let mut b = CanonicalClient::open(config).await.unwrap();
        let mut http = CanonicalHttpClient::open(canonical_http_config(&f))
            .await
            .unwrap();
        for mode in ["native", "http"] {
            let sid = format!("cancel-observation-{mode}");
            let make = |operation: &str, revision, action| Request {
                operation_id: format!("{sid}-{operation}"),
                conversation_id: room.id.clone(),
                session_id: sid.clone(),
                by_participant: None,
                expected_revision: revision,
                action,
            };
            owner
                .session_command(make(
                    "invite",
                    None,
                    Action::Invite {
                        participants: vec![target.clone()],
                        medium: "voice".into(),
                        intent: "synchronous-engagement".into(),
                        capabilities: json!({"audio":["opus"]}),
                    },
                ))
                .await
                .unwrap();
            let ia = a.receive_session_invitation().await.unwrap();
            let ib = b.receive_session_invitation().await.unwrap();
            assert_ne!(ia.id(), ib.id());
            let cancel = make(
                "cancel",
                Some(0),
                Action::Cancel {
                    reason_code: 487,
                    reason: "request-cancelled".into(),
                },
            );
            let receipt = if mode == "native" {
                owner.session_command(cancel.clone()).await.unwrap()
            } else {
                http.session_command(cancel.clone()).await.unwrap()
            };
            for (client, invitation) in [(&mut a, &ia), (&mut b, &ib)] {
                let hint = tokio::time::timeout(Duration::from_secs(5), client.receive_control())
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(hint.kind(), "session.cancel");
                assert_eq!(
                    hint.scope(rvoip_uctp::v2::Scope::Reply),
                    Some(invitation.id())
                );
                assert_eq!(
                    hint.scope(rvoip_uctp::v2::Scope::Conversation),
                    Some(room.id.as_str())
                );
                assert_eq!(
                    hint.scope(rvoip_uctp::v2::Scope::Session),
                    Some(sid.as_str())
                );
                assert_eq!(
                    hint.scope(rvoip_uctp::v2::Scope::Operation),
                    invitation.scope(rvoip_uctp::v2::Scope::Operation)
                );
                assert!(hint.scope(rvoip_uctp::v2::Scope::Connection).is_none());
                assert_eq!(
                    hint.original()["payload"],
                    json!({"by":from,"reason_code":487,"reason":"request-cancelled"})
                );
            }
            assert_eq!(
                owner.session_command(cancel.clone()).await.unwrap(),
                receipt
            );
            assert_eq!(http.session_command(cancel).await.unwrap(), receipt);
            if mode == "http" {
                b.answer_session_invitation(
                    &ib,
                    SessionInvitationAnswer::Accept {
                        capabilities: json!({"audio":["opus"]}),
                    },
                )
                .await
                .unwrap();
                let current = b
                    .conversation_snapshot(&room.id, serde_json::from_value(json!({})).unwrap())
                    .await
                    .unwrap();
                assert!(current.items.iter().any(|i| i["kind"] == "session"
                    && i["data"]["id"] == sid
                    && i["data"]["state"] == "cancelled"
                    && i["data"]["topology_revision"] == 1));
            }
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(80), a.receive_control())
                .await
                .is_err()
        );
        a.close();
        b.close();
        owner.close();
        f.server.shutdown().await.unwrap();
    }
}
