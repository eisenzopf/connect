//! Real ordinary MCP mutations replay through native and HTTP SDKs on one ledger.
#![allow(clippy::unwrap_used)]
#[path = "support/agent_mcp_fixture.rs"]
mod agent;
#[path = "support/agent_mcp_client.rs"]
mod mcp;
use connect_client::{ClientConfig, ConnectClient};
use connect_protocol::{membership::*, Operation, Outcome, Response};
use connect_store::Scope;
use serde_json::{json, Value};

async fn http(
    f: &agent::Fixture,
    config: &ClientConfig,
    operation: &Operation,
    sample: &str,
) -> Outcome {
    let response = reqwest::Client::new()
        .post(format!(
            "http://{}/v1/operations",
            f.server.http_address.unwrap()
        ))
        .bearer_auth(&config.credential)
        .json(operation)
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    assert_eq!(status, 200);
    let body: Value = response.json().await.unwrap();
    if let Some(dir) = std::env::var_os("CONNECT_GATE6_HTTP_SAMPLES") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("membership-{sample}.json")),serde_json::to_vec_pretty(&json!({"api":"membership","route":"/v1/operations","status":status,"request":operation,"response":body})).unwrap()).unwrap();
    }
    serde_json::from_value::<Response>(body)
        .unwrap()
        .result
        .unwrap()
}
async fn change(
    f: &agent::Fixture,
    observer: &mcp::Client,
    config: &ClientConfig,
    client: &mcp::Client,
    cid: &str,
    id: &str,
    action: Value,
    member: bool,
) -> Value {
    let Outcome::MembershipSnapshot(snapshot) = mcp::outcome(
        mcp::call(
            observer,
            "connect_membership_snapshot",
            json!({"conversation_id":cid}),
        )
        .await,
    ) else {
        panic!("snapshot expected")
    };
    let revision = snapshot.revision;
    let request:MembershipRequest=serde_json::from_value(json!({"operation_id":id,"conversation_id":cid,"expected_revision":revision,"change":action})).unwrap();
    let mut arguments = action.as_object().unwrap().clone();
    let action = arguments.remove("action").unwrap();
    for (k, v) in [
        ("operation_id", json!(id)),
        ("conversation_id", json!(cid)),
        ("expected_revision", json!(revision)),
    ] {
        arguments.insert(k.into(), v);
    }
    let tool = match action.as_str().unwrap() {
        "share_history" => "connect_history_share".into(),
        "revoke_history_share" => "connect_history_share_revoke".into(),
        a => format!("connect_participant_{a}"),
    };
    let expected = mcp::outcome(mcp::call(client, &tool, Value::Object(arguments.clone())).await);
    assert_eq!(
        mcp::outcome(mcp::call(client, &tool, Value::Object(arguments.clone())).await),
        expected
    );
    let Outcome::Membership(value) = &expected else {
        panic!("membership expected")
    };
    assert_eq!(value.receipt().unwrap().operation_id, id);
    let mut native = ConnectClient::open(config.clone()).await.unwrap();
    assert_eq!(
        &native.change_membership(request.clone()).await.unwrap(),
        value
    );
    let operation = Operation::Membership {
        request: request.clone(),
    };
    assert_eq!(http(f, config, &operation, id).await, expected);
    let snapshot = if member {
        let snapshot = native.membership_snapshot(cid).await.unwrap();
        assert_eq!(
            mcp::outcome(
                mcp::call(
                    client,
                    "connect_membership_snapshot",
                    json!({"conversation_id":cid})
                )
                .await
            ),
            Outcome::MembershipSnapshot(snapshot.clone())
        );
        assert_eq!(
            http(
                f,
                config,
                &Operation::MembershipSnapshot {
                    conversation_id: cid.into()
                },
                "snapshot"
            )
            .await,
            Outcome::MembershipSnapshot(snapshot.clone())
        );
        Some(snapshot)
    } else {
        None
    };
    assert_eq!(
        mcp::outcome(mcp::call(client, "connect_membership_invitations", json!({"limit":1})).await),
        Outcome::MembershipInvitations(native.membership_invitations(None, 1).await.unwrap())
    );
    assert_eq!(
        http(
            f,
            config,
            &Operation::MembershipInvitations {
                after: None,
                limit: 1
            },
            "invitations"
        )
        .await,
        Outcome::MembershipInvitations(native.membership_invitations(None, 1).await.unwrap())
    );
    // Closed MCP arguments must refuse caller authority before executing another mutation.
    for field in ["actor", "credential_id", "action"] {
        let mut invalid = arguments.clone();
        invalid.insert(field.into(), json!("caller-injected"));
        mcp::invalid(client, &tool, Value::Object(invalid)).await;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let data = json!({"base":format!("http://{}",f.server.http_address.unwrap()),"token":config.credential,"request":request,"expected":value,"snapshot":snapshot});
    tokio::task::spawn_blocking(move || {
        use std::{
            io::Write,
            process::{Command, Stdio},
        };
        for (program, script) in [
            ("node", "sdk/tests/membership_probe.mjs"),
            ("python3", "sdk/tests/membership_probe.py"),
        ] {
            let mut child = Command::new(program)
                .current_dir(&root)
                .env("PYTHONPATH", root.join("sdk/python"))
                .arg(script)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(serde_json::to_string(&data).unwrap().as_bytes())
                .unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{program}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                serde_json::from_slice::<Value>(&output.stdout).unwrap(),
                json!({"passed":true})
            );
        }
    })
    .await
    .unwrap();
    serde_json::to_value(value).unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn all_membership_actions_share_receipts_across_mcp_native_http_and_public_sdks() {
    let f = agent::Fixture::start().await;
    let owner = f.issue_as_operator(&f.agent, Scope::collaboration(vec!["*".into()]));
    let peer = f.issue_as_operator(&f.peer, Scope::collaboration(vec!["*".into()]));
    let mut native = ConnectClient::open(owner.clone()).await.unwrap();
    let cid = native.create("membership-room", vec![]).await.unwrap().id;
    let message = native
        .send("old-message", &cid, "Explicit history consent")
        .await
        .unwrap()
        .message_id;
    let manager = mcp::client(f.server.mcp_address.unwrap(), &owner.credential).await;
    let target = mcp::client(f.server.mcp_address.unwrap(), &peer.credential).await;
    let catalog = manager.list_tools(None).await.unwrap();
    for name in [
        "connect_participant_add",
        "connect_participant_invite",
        "connect_participant_join",
        "connect_participant_decline",
        "connect_participant_cancel_invitation",
        "connect_participant_leave",
        "connect_participant_remove",
        "connect_participant_update_role",
        "connect_history_share",
        "connect_history_share_revoke",
        "connect_membership_snapshot",
        "connect_membership_invitations",
    ] {
        assert!(
            catalog.tools.iter().any(|t| t.name == name),
            "missing {name}"
        );
    }
    let expiry = chrono::Utc::now().timestamp() + 300;
    for (id, decision) in [
        ("declined", "decline"),
        ("cancelled", "cancel_invitation"),
        ("joined", "join"),
    ] {
        let v=change(&f,&manager,&owner,&manager,&cid,&format!("invite-{id}"),json!({"action":"invite","identity_id":peer.identity_id,"kind":"human","role":"guest","expires_at":expiry}),true).await;
        let invitation = &v["invitation"]["invitation_id"];
        let (config, client) = if decision == "cancel_invitation" {
            (&owner, &manager)
        } else {
            (&peer, &target)
        };
        change(
            &f,
            &manager,
            config,
            client,
            &cid,
            id,
            json!({"action":decision,"invitation_id":invitation}),
            decision != "decline",
        )
        .await;
    }
    let mut peer_native = ConnectClient::open(peer.clone()).await.unwrap();
    assert!(peer_native
        .history(&cid, None, 100)
        .await
        .unwrap()
        .messages
        .is_empty());
    let snapshot = native.membership_snapshot(&cid).await.unwrap();
    let pid = &snapshot
        .participants
        .iter()
        .find(|p| p.identity_id == peer.identity_id)
        .unwrap()
        .participant_id;
    let share=change(&f,&manager,&owner,&manager,&cid,"share",json!({"action":"share_history","resources":[{"kind":"message","id":message}],"participants":[pid],"provenance":"recorded consent"}),true).await;
    assert_eq!(
        peer_native
            .history(&cid, None, 100)
            .await
            .unwrap()
            .messages
            .len(),
        1
    );
    change(
        &f,
        &manager,
        &owner,
        &manager,
        &cid,
        "revoke-share",
        json!({"action":"revoke_history_share","sharing_id":share["sharing_id"]}),
        true,
    )
    .await;
    assert!(peer_native
        .history(&cid, None, 100)
        .await
        .unwrap()
        .messages
        .is_empty());
    change(
        &f,
        &manager,
        &owner,
        &manager,
        &cid,
        "role",
        json!({"action":"update_role","participant_id":pid,"role":"reviewer"}),
        true,
    )
    .await;
    let removed = change(
        &f,
        &manager,
        &owner,
        &manager,
        &cid,
        "remove",
        json!({"action":"remove","participant_id":pid,"reason":"completed review"}),
        true,
    )
    .await;
    assert_eq!(removed["participant"]["removal_reason"], "completed review");
    let added = change(
        &f,
        &manager,
        &owner,
        &manager,
        &cid,
        "add~again",
        json!({"action":"add","identity_id":peer.identity_id,"kind":"human","role":"guest"}),
        true,
    )
    .await;
    assert_eq!(added["participant"]["participant_id"], *pid);
    assert_eq!(added["participant"]["membership_epoch"], 2);
    assert_eq!(added["participant"]["removal_reason"], Value::Null);
    change(
        &f,
        &manager,
        &peer,
        &target,
        &cid,
        "leave",
        json!({"action":"leave"}),
        false,
    )
    .await;
    manager.cancel().await.unwrap();
    target.cancel().await.unwrap();
    f.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn membership_http_and_mcp_require_named_scopes_and_closed_authority_fields() {
    let f = agent::Fixture::start().await;
    let owner = f.issue_as_operator(&f.agent, Scope::collaboration(vec!["*".into()]));
    let mut native = ConnectClient::open(owner.clone()).await.unwrap();
    let room = native
        .create("membership-denial-room", vec![])
        .await
        .unwrap();
    let base = json!({"op":"membership","request":{"operation_id":"unauthorized-add","conversation_id":room.id,"expected_revision":room.revision,"change":{"action":"add","identity_id":f.peer.identity_id,"kind":"human","role":"admin"}}});
    let http = reqwest::Client::new();
    let url = format!("http://{}/v1/operations", f.server.http_address.unwrap());
    let response = http
        .post(&url)
        .bearer_auth(&f.agent.credential)
        .json(&base)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    let body: Response = response.json().await.unwrap();
    assert_eq!(body.result.unwrap_err().code, "forbidden");
    for field in ["actor", "credential_id", "approved"] {
        let mut injected = base.clone();
        injected["request"][field] = json!("override");
        let response = http
            .post(&url)
            .bearer_auth(&owner.credential)
            .json(&injected)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 422);
    }
    let mut injected = base.clone();
    injected["request"]["change"]["admin"] = json!(true);
    assert_eq!(
        http.post(&url)
            .bearer_auth(&owner.credential)
            .json(&injected)
            .send()
            .await
            .unwrap()
            .status(),
        422
    );
    let narrow = mcp::client(f.server.mcp_address.unwrap(), &f.agent.credential).await;
    let catalog = narrow.list_tools(None).await.unwrap();
    assert!(catalog
        .tools
        .iter()
        .all(|t| !t.name.starts_with("connect_participant_")
            && !t.name.starts_with("connect_membership_")
            && !t.name.starts_with("connect_history_share")));
    let request: MembershipRequest = serde_json::from_value(base["request"].clone()).unwrap();
    // All prior refusals leave the logical ID available for a properly authorized request.
    let value = native.change_membership(request).await.unwrap();
    assert_eq!(value.receipt().unwrap().operation_id, "unauthorized-add");
    narrow.cancel().await.unwrap();
    f.close().await;
}
