//! Retained artifact references cannot recover bytes after their real expiry.
#![allow(clippy::unwrap_used)]
#[path = "support/mcp_fixture.rs"]
mod fixture;
#[path = "support/mcp_client.rs"]
mod mcp;
use base64::{engine::general_purpose::STANDARD, Engine};
use mcp::{call, client, permissions, success};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expired_artifact_replay_and_reconnect_keep_references_without_returning_bytes() {
    let f = fixture::Fixture::start_permitted(true, permissions()).await;
    let client = client(&f).await;
    let bytes = b"Private artifact bytes expire independently of their conversation.";
    let encoded = STANDARD.encode(bytes);
    let expires_at = chrono::Utc::now().timestamp() + 10;
    let artifact = success(
        call(
            &client,
            "connect_artifact_begin",
            json!({"to":"all",
                "request_id":"expiring-begin", "step_id":"file", "name":"expires.txt",
                "media_type":"text/plain", "size":bytes.len(),
                "sha256":hex::encode(Sha256::digest(bytes)), "expires_at":expires_at
            }),
        )
        .await,
    );
    let id = artifact["data"]["outcome"]["data"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    success(
        call(
            &client,
            "connect_artifact_append",
            json!({
                "request_id":"expiring-append", "step_id":"file", "artifact_id":id,
                "offset":0, "data":encoded
            }),
        )
        .await,
    );
    success(
        call(
            &client,
            "connect_artifact_finish",
            json!({
                "request_id":"expiring-finish", "step_id":"file", "artifact_id":id
            }),
        )
        .await,
    );
    let read = json!({"request_id":"expiring-read", "step_id":"file-read",
        "artifact_id":id, "offset":0, "limit":32768});
    let before = success(call(&client, "connect_artifact_get", read.clone()).await);
    assert_eq!(before["data"]["outcome"]["data"]["data"], encoded);
    tokio::time::timeout(Duration::from_secs(15), async {
        while chrono::Utc::now().timestamp() <= expires_at {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let after = call(&client, "connect_artifact_get", read.clone()).await;
    assert_eq!(after.is_error, Some(true));
    assert_eq!(
        after.structured_content.as_ref().unwrap()["code"],
        "forbidden"
    );
    assert!(!serde_json::to_string(&after).unwrap().contains(&encoded));
    client.cancel().await.unwrap();
    let client = mcp::client(&f).await;
    let after = call(&client, "connect_artifact_get", read).await;
    assert_eq!(after.is_error, Some(true));
    assert!(!serde_json::to_string(&after).unwrap().contains(&encoded));
    let recall = success(call(&client, "connect_recall", json!({"limit":100})).await);
    let sources: Vec<_> = recall["data"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|source| source["source_id"] == id)
        .collect();
    assert!(
        !sources.is_empty(),
        "Retained artifact source references disappeared"
    );
    for source in sources {
        assert_eq!(source["state"], "expired");
        assert!(source["body"].is_null());
        assert!(source["provenance"]
            .as_str()
            .unwrap()
            .contains("bytes-require-artifact-get"));
    }
    assert!(!serde_json::to_string(&recall).unwrap().contains(&encoded));
    // Expiry does not close this conversation or terminalize unrelated task work.
    let status = success(call(&client, "connect_task_status", json!({})).await);
    assert_eq!(status["data"]["conversation_id"], f.task.conversation_id);
    assert_eq!(status["data"]["state"], "running");
    client.cancel().await.unwrap();
    f.close().await;
}
