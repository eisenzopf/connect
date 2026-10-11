//! Derive continuation only from results in the actual resumed runtime request.
#![allow(clippy::unwrap_used)]
use serde_json::{json, Value};

const TOOLS: &str = include_str!("../fixtures/codex-workflow-tools.js");
pub fn question(body: &str) -> String {
    let arguments = json!({"request_id":"workflow-question","step_id":"review","body":body,"wait_for_reply":true});
    format!("{TOOLS}\nconst intent = {arguments};\nconst question = await invoke('connect_send', intent);\nconst task = await invoke('connect_task_status', {{}});\nconst journal = await invoke('connect_task_wait', {{limit:100,wait_ms:0}});\ntext(JSON.stringify({{question,task,journal}}));")
}
pub fn read(request: &Value) -> String {
    let prior = question_result(request);
    let arguments = json!({"after":prior["journal"]["data"]["next"],"limit":100,"wait_ms":0});
    format!("{TOOLS}\nconst journal = await invoke('connect_task_wait', {arguments});\nconst task = await invoke('connect_task_status', {{}});\nconst recall = await invoke('connect_recall', {{limit:100}});\ntext(JSON.stringify({{journal,task,recall}}));")
}
pub fn question_result(request: &Value) -> Value {
    let result = result(request, "workflow-question-call");
    assert_eq!(result["question"]["kind"], "tool");
    assert_eq!(result["task"]["data"]["state"], "waiting");
    assert!(result["journal"]["data"]["next"].is_string());
    result
}
pub fn follow_up(request: &Value) -> Option<String> {
    let prior = result(request, "workflow-read-call");
    let replies: Vec<_> = prior["journal"]["data"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["data"]["kind"] == "reply")
        .collect();
    if replies.is_empty() {
        return None;
    }
    assert_eq!(
        replies.len(),
        1,
        "Fixture expects one exactly attributed reply"
    );
    let reply = &replies[0]["data"]["data"];
    assert_eq!(reply["task_id"], prior["task"]["data"]["id"]);
    let provenance = format!(
        "conversation-journal:{}",
        reply["event_id"].as_str().unwrap()
    );
    let source = prior["recall"]["data"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|source| {
            source["provenance"].as_str().unwrap().split(';').next() == Some(provenance.as_str())
        })
        .unwrap();
    assert_eq!(source["state"], "available");
    assert!(source["provenance"]
        .as_str()
        .unwrap()
        .starts_with("conversation-journal:"));
    let arguments = json!({"request_id":"workflow-follow-up","step_id":"decision","body":format!("Confirmed from {}: {}",source["source_id"].as_str().unwrap(),source["body"].as_str().unwrap()),"reply_to":reply["event_id"],"wait_for_reply":false});
    Some(format!(
        "{TOOLS}\nconst intent = {arguments};\ntext(await invoke('connect_send', intent));"
    ))
}
pub fn saved_follow_up(request: &Value) -> Value {
    request["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item["type"] == "custom_tool_call" && item["call_id"] == "workflow-follow-up-call"
        })
        .unwrap()
        .clone()
}
fn result(request: &Value, id: &str) -> Value {
    let item = request["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "custom_tool_call_output" && item["call_id"] == id)
        .unwrap();
    // Responses allows either a string or typed input-text content blocks.
    let output = match &item["output"] {
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .map(|item| {
                assert_eq!(item["type"], "input_text");
                item["text"].as_str().unwrap()
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => panic!("Expected actual Responses text output"),
    };
    if let Ok(value) = serde_json::from_str::<Value>(&output) {
        if value.get("journal").is_some() {
            return value;
        }
    }
    let start = output
        .find("{\"question\"")
        .or_else(|| output.find("{\"journal\""))
        .expect("Actual code-mode output must retain the emitted JSON result");
    serde_json::Deserializer::from_str(&output[start..])
        .into_iter::<Value>()
        .next()
        .unwrap()
        .unwrap()
}
