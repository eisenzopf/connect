//! Deterministic Responses transport fixture; no model reasoning or inference.
#![allow(clippy::unwrap_used)]
use super::program;
use axum::{extract::State, response::IntoResponse, routing::post, Json, Router};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc, Mutex,
};

pub struct Script {
    pub requests: Mutex<Vec<Value>>,
    pub stage: AtomicU8,
    question: String,
}
pub struct Peer {
    pub url: String,
    pub script: Arc<Script>,
    handle: tokio::task::JoinHandle<()>,
}
impl Peer {
    pub async fn start(body: &str) -> Self {
        let script = Arc::new(Script {
            requests: Mutex::new(vec![]),
            stage: AtomicU8::new(0),
            question: program::question(body),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .route("/responses", post(response))
            .with_state(script.clone());
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            url,
            script,
            handle,
        }
    }
    pub fn resume(&self) {
        self.script.stage.store(2, Ordering::SeqCst);
    }
    pub fn replay(&self) {
        self.script.stage.store(5, Ordering::SeqCst);
    }
    pub fn requests(&self) -> Vec<Value> {
        self.script.requests.lock().unwrap().clone()
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

fn exec(tools: &Value, namespace: Option<&str>) -> Option<(String, Option<String>)> {
    for tool in tools.as_array()? {
        let Some(name) = tool["name"].as_str() else {
            continue;
        };
        if name == "exec" && tool["type"] == "custom" {
            return Some((name.into(), namespace.map(str::to_owned)));
        }
        if tool["type"] == "namespace" {
            if let Some(found) = exec(&tool["tools"], Some(name)) {
                return Some(found);
            }
        }
    }
    None
}
fn call(request: &Value, id: &str, input: String) -> Value {
    let (name, namespace) = request["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "additional_tools")
        .find_map(|item| exec(&item["tools"], None))
        .unwrap();
    let mut item = json!({"type":"custom_tool_call","call_id":id,"name":name,"input":input});
    if let Some(namespace) = namespace {
        item["namespace"] = json!(namespace);
    }
    item
}
async fn response(
    State(script): State<Arc<Script>>,
    Json(request): Json<Value>,
) -> impl IntoResponse {
    let index = {
        let mut requests = script.requests.lock().unwrap();
        let index = requests.len();
        requests.push(request.clone());
        index
    };
    let stage = script.stage.fetch_add(1, Ordering::SeqCst);
    let item = match stage {
        0 => call(&request, "workflow-question-call", script.question.clone()),
        2 => call(&request, "workflow-read-call", program::read(&request)),
        3 => program::follow_up(&request)
            .map(|code| call(&request, "workflow-follow-up-call", code))
            .unwrap_or_else(final_message),
        5 => {
            let mut item = program::saved_follow_up(&request);
            item["call_id"] = json!("workflow-follow-up-replay");
            item["id"] = json!("workflow-follow-up-replay-item");
            item
        }
        1 => {
            program::question_result(&request);
            final_message()
        }
        4 | 6 => final_message(),
        _ => panic!("Unexpected scripted Responses retry"),
    };
    let id = format!("workflow-response-{index}");
    let events = [
        json!({"type":"response.created","response":{"id":id}}),
        json!({"type":"response.output_item.done","item":item}),
        json!({"type":"response.completed","response":{"id":id,"usage":{"input_tokens":0,"output_tokens":0,"total_tokens":0}}}),
    ];
    let body = events
        .iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap()
            )
        })
        .collect::<String>();
    ([("content-type", "text/event-stream")], body)
}
fn final_message() -> Value {
    json!({"type":"message","role":"assistant","id":"workflow-final","content":[{"type":"output_text","text":"Scripted runtime checkpoint; task state remains authoritative."}]})
}
