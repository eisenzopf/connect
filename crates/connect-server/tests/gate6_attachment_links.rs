//! Current attachment links across native, HTTP, MCP and independent SDKs.
#![allow(clippy::unwrap_used)]
#[path = "support/agent_mcp_fixture.rs"]
mod agent;
#[path = "support/agent_mcp_client.rs"]
mod mcp;
use connect_client::{ArtifactUpload, ConnectClient};
use connect_protocol::{messages::*, Operation, Outcome, Response};
use connect_store::Scope;
use serde_json::json;
use sha2::{Digest, Sha256};
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn issued_links_upload_once_and_download_only_for_current_message_and_file_recipient() {
    let f = agent::Fixture::start().await;
    let config = f.issue_as_operator(&f.agent, Scope::collaboration(vec!["*".into()]));
    let peer = f.issue_as_operator(&f.peer, Scope::collaboration(vec!["*".into()]));
    let foreign = f.issue_as_operator(&f.foreign, Scope::collaboration(vec!["*".into()]));
    let mut native = ConnectClient::open(config.clone()).await.unwrap();
    let cid = native
        .create(
            "link-room",
            vec![peer.identity_id.clone(), foreign.identity_id.clone()],
        )
        .await
        .unwrap()
        .id;
    let participant = native
        .capabilities("connect:agent@local", Some(&cid))
        .await
        .unwrap()
        .participant_id
        .unwrap();
    let audience = MessageAudience::Participants(vec![participant]);
    let artifact = native
        .artifact_begin_with_options(
            "begin",
            &cid,
            ArtifactUpload::new(
                "private.bin",
                "application/octet-stream",
                6,
                hex::encode(Sha256::digest(b"secret")),
            )
            .with_audience(audience.clone()),
        )
        .await
        .unwrap();
    let upload = native
        .artifact_upload_link(&cid, &artifact.id, 300)
        .await
        .unwrap();
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let base = format!("http://{}", f.server.http_address.unwrap());
    sample_http(
        &http,
        &base,
        &config.credential,
        "upload-link",
        Operation::ArtifactUploadLink {
            conversation_id: cid.clone(),
            artifact_id: artifact.id.clone(),
            expires_in: 60,
        },
    )
    .await;
    let response = http
        .put(format!("{base}{}?request_id=chunk&offset=0", upload.path))
        .bearer_auth(&config.credential)
        .body("secret")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let raw: serde_json::Value = response.json().await.unwrap();
    let decoded: Response = serde_json::from_value(raw.clone()).unwrap();
    assert!(matches!(decoded.result.unwrap(),Outcome::Artifact(a) if a.received==6));
    sample_http(
        &http,
        &base,
        &config.credential,
        "upload-chunk",
        Operation::ArtifactLinkAppend {
            request_id: "chunk".into(),
            conversation_id: cid.clone(),
            artifact_id: artifact.id.clone(),
            link: upload.path.clone(),
            offset: 0,
            data: "c2VjcmV0".into(),
        },
    )
    .await;
    sample_http(
        &http,
        &base,
        &config.credential,
        "inspect",
        Operation::ArtifactInspect {
            conversation_id: cid.clone(),
            artifact_id: artifact.id.clone(),
        },
    )
    .await;
    let refreshed = native
        .artifact_upload_link(&cid, &artifact.id, 60)
        .await
        .unwrap();
    assert_eq!(
        native
            .artifact_link_append("chunk", &refreshed, 0, "c2VjcmV0")
            .await
            .unwrap()
            .received,
        6
    );
    let client = mcp::client(f.server.mcp_address.unwrap(), &config.credential).await;
    assert!(
        matches!(mcp::outcome(mcp::operation(&client,&Operation::ArtifactInspect{conversation_id:cid.clone(),artifact_id:artifact.id.clone()}).await),Outcome::Artifact(a) if a.received==6)
    );
    assert!(matches!(
        mcp::outcome(
            mcp::operation(
                &client,
                &Operation::ArtifactUploadLink {
                    conversation_id: cid.clone(),
                    artifact_id: artifact.id.clone(),
                    expires_in: 30
                }
            )
            .await
        ),
        Outcome::ArtifactLink(_)
    ));
    client.cancel().await.unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf();
    let data=json!({"base":base,"token":config.credential,"cid":cid,"artifact":artifact.id,"peer_token":peer.credential,"foreign_token":foreign.credential}).to_string();
    tokio::task::spawn_blocking(move || {
        for (program, script) in [
            ("node", "sdk/tests/attachment_links_probe.mjs"),
            ("python3", "sdk/tests/attachment_links_probe.py"),
        ] {
            let result = std::process::Command::new(program)
                .current_dir(&root)
                .env("PYTHONPATH", root.join("sdk/python"))
                .arg(script)
                .arg(&data)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "SDK probe: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&result.stdout).unwrap(),
                json!({"passed":true})
            );
        }
    })
    .await
    .unwrap();
    native
        .artifact_finish("finish", &cid, &artifact.id)
        .await
        .unwrap();
    let mut draft = MessageDraft::text(audience, "private attachment");
    draft.attachments.push(AttachmentRef {
        id: artifact.id.clone(),
        content_type: artifact.media_type.clone(),
        size_bytes: artifact.size,
    });
    let message = native.send_message("send-file", &cid, draft).await.unwrap();
    let mut recipient = ConnectClient::open(peer.clone()).await.unwrap();
    let link = recipient
        .artifact_download_link(&cid, &artifact.id, &message.message_id, 60)
        .await
        .unwrap();
    sample_http(
        &http,
        &base,
        &peer.credential,
        "download-link",
        Operation::ArtifactDownloadLink {
            conversation_id: cid.clone(),
            artifact_id: artifact.id.clone(),
            message_id: message.message_id.clone(),
            expires_in: 60,
        },
    )
    .await;
    sample_http(
        &http,
        &base,
        &peer.credential,
        "download-chunk",
        Operation::ArtifactLinkGet {
            conversation_id: cid.clone(),
            artifact_id: artifact.id.clone(),
            link: link.path.clone(),
            offset: 0,
            limit: 32768,
        },
    )
    .await;
    let query = "?offset=0&limit=32768";
    let response = http
        .get(format!("{base}{}{query}", link.path))
        .bearer_auth(&peer.credential)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(response.bytes().await.unwrap().as_ref(), b"secret");
    assert_eq!(
        recipient
            .artifact_link_get(&link, 0, 32768)
            .await
            .unwrap()
            .data,
        "c2VjcmV0"
    );
    for token in [&config.credential, &foreign.credential] {
        assert_eq!(
            http.get(format!("{base}{}{query}", link.path))
                .bearer_auth(token)
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
    }
    assert_eq!(
        http.get(format!("{base}{}{query}", link.path))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let client = mcp::client(f.server.mcp_address.unwrap(), &peer.credential).await;
    let op = Operation::ArtifactDownloadLink {
        conversation_id: cid.clone(),
        artifact_id: artifact.id.clone(),
        message_id: message.message_id.clone(),
        expires_in: 60,
    };
    let Outcome::ArtifactLink(mcp_link) = mcp::outcome(mcp::operation(&client, &op).await) else {
        panic!()
    };
    assert!(
        matches!(mcp::outcome(mcp::operation(&client,&Operation::ArtifactLinkGet{conversation_id:cid.clone(),artifact_id:artifact.id.clone(),link:mcp_link.path,offset:0,limit:32768}).await),Outcome::ArtifactChunk(chunk) if chunk.data=="c2VjcmV0")
    );
    client.cancel().await.unwrap();
    native
        .artifact_delete("delete", &cid, &artifact.id)
        .await
        .unwrap();
    assert_eq!(
        http.get(format!("{base}{}{query}", link.path))
            .bearer_auth(&peer.credential)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    native.close().await;
    recipient.close().await;
    f.close().await;
}

async fn sample_http(
    http: &reqwest::Client,
    base: &str,
    token: &str,
    name: &str,
    operation: Operation,
) {
    let response = http
        .post(format!("{base}/v1/operations"))
        .bearer_auth(token)
        .json(&operation)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    let decoded: Response = serde_json::from_value(body.clone()).unwrap();
    assert!(decoded.result.is_ok());
    if let Some(directory) = std::env::var_os("CONNECT_GATE6_ATTACHMENT_SAMPLES") {
        let dir = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&json!({"request":operation,"response":body})).unwrap(),
        )
        .unwrap();
    }
}
