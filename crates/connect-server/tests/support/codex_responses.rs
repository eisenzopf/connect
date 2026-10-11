//! Scripted loopback Responses peer, not an inference/model implementation.
#![allow(clippy::unwrap_used)]
use super::barrier::Barrier;
use axum::{extract::State, response::IntoResponse, routing::post, Json, Router};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc, Mutex,
};
use tokio::sync::{Notify, Semaphore};

pub struct Script {
    pub requests: Mutex<Vec<Value>>,
    pub barrier: Arc<Barrier>,
    pub returned: Notify,
    pub finish: Semaphore,
    pub arguments: Value,
    stage: AtomicU8,
}
pub struct Responses {
    pub url: String,
    pub script: Arc<Script>,
    handle: tokio::task::JoinHandle<()>,
}
impl Responses {
    pub async fn start(barrier: Arc<Barrier>, arguments: Value) -> Self {
        let script = Arc::new(Script {
            requests: Mutex::new(vec![]),
            barrier,
            returned: Notify::new(),
            finish: Semaphore::new(0),
            arguments,
            stage: AtomicU8::new(0),
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
    pub fn requests(&self) -> Vec<Value> {
        self.script.requests.lock().unwrap().clone()
    }
    pub fn replay(&self) {
        self.script.stage.store(2, Ordering::SeqCst);
    }
}
impl Drop for Responses {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

fn find_exec(tools: &Value, namespace: Option<&str>) -> Option<(String, Option<String>)> {
    for tool in tools.as_array()? {
        let Some(name) = tool["name"].as_str() else {
            continue;
        };
        if name == "exec" && tool["type"] == "custom" {
            return Some((name.into(), namespace.map(str::to_owned)));
        }
        if tool["type"] == "namespace" {
            if let Some(found) = find_exec(&tool["tools"], Some(name)) {
                return Some(found);
            }
        }
    }
    None
}
fn original_call(request: &Value) -> Option<Value> {
    request["input"]
        .as_array()?
        .iter()
        .find(|item| item["type"] == "custom_tool_call" && item["call_id"] == "intent-call")
        .cloned()
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
    let item = match script.stage.load(Ordering::SeqCst) {
        0 => {
            script.stage.store(1, Ordering::SeqCst);
            script.barrier.pause();
            let (name, namespace) = request["input"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| item["type"] == "additional_tools")
                .find_map(|item| find_exec(&item["tools"], None))
                .expect("Actual runtime must advertise code-mode exec");
            let code=format!("const intent = {};\nconst candidates = ALL_TOOLS.filter(t => t.name.endsWith('connect_send') && t.name.includes('connect_task'));\nif (candidates.length !== 1) throw new Error('Expected one installed Connect send tool');\ntext(await tools[candidates[0].name](intent));",script.arguments);
            let mut call =
                json!({"type":"custom_tool_call","call_id":"intent-call","name":name,"input":code});
            if let Some(namespace) = namespace {
                call["namespace"] = json!(namespace);
            }
            call
        }
        1 => {
            script.returned.notify_one();
            script.finish.acquire().await.unwrap().forget();
            json!({"type":"message","role":"assistant","id":"fixture-final","content":[{"type":"output_text","text":"Fixture complete."}]})
        }
        2 => {
            script.stage.store(3, Ordering::SeqCst);
            // Recover the intended arguments from actual resumed input, never a
            // replacement prompt or this fixture's original in-memory args.
            let mut call =
                original_call(&request).expect("Resume must replay original persisted intent");
            call["call_id"] = json!("intent-replay");
            call["id"] = json!("intent-replay-item");
            call
        }
        3 => {
            json!({"type":"message","role":"assistant","id":"fixture-recovered","content":[{"type":"output_text","text":"Fixture reconciled original intent."}]})
        }
        _ => panic!("Unexpected model fixture retry"),
    };
    let id = format!("response-{index}");
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
