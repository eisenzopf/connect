//! Draft bytes and public recipient verification for the installed-host journey.
#![allow(clippy::unwrap_used)]
use base64::{engine::general_purpose::STANDARD, Engine};
use connect_client::{Artifact, ConnectClient, Message};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub fn program(name: &str, bytes: &[u8]) -> String {
    let tools = include_str!("../fixtures/codex-workflow-tools.js");
    let steps = include_str!("../fixtures/codex-draft.js");
    let draft = json!({"name":name,"size":bytes.len(),"sha256":hex::encode(Sha256::digest(bytes)),"data":STANDARD.encode(bytes)});
    format!("{tools}\nconst draft = {draft};\n{steps}")
}

/// The native participant verifies the actual stored bytes, not a model claim.
pub async fn verify(
    human: &mut ConnectClient,
    messages: &[Message],
    name: &str,
    bytes: &[u8],
) -> String {
    let (message, body) = messages
        .iter()
        .find_map(|message| {
            let body: Value = serde_json::from_str(&message.body).ok()?;
            (body["title"] == name).then_some((message, body))
        })
        .unwrap();
    let artifact: Artifact = serde_json::from_value(body["artifact"].clone()).unwrap();
    assert_eq!(artifact.conversation_id, message.conversation_id);
    assert_eq!(artifact.owner_id, message.sender);
    assert_eq!(artifact.name, name);
    assert_eq!(artifact.size, bytes.len() as u64);
    assert_eq!(artifact.sha256, hex::encode(Sha256::digest(bytes)));
    let chunk = human
        .artifact_get(&message.conversation_id, &artifact.id, 0, 32768)
        .await
        .unwrap();
    assert_eq!(chunk.artifact, artifact);
    assert_eq!(chunk.offset, 0);
    assert!(chunk.eof);
    assert_eq!(STANDARD.decode(chunk.data).unwrap(), bytes);
    message.id.clone()
}
