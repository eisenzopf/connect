//! Actual installed JSON-lines process against the production canonical server.
#[cfg(unix)]
#[path = "canonical_stdio/node.rs"]
mod node;
use super::*;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines},
    process::{Child, ChildStdin, ChildStdout},
};
pub(crate) struct Process {
    child: Child,
    input: Option<ChildStdin>,
    lines: Lines<BufReader<ChildStdout>>,
}
impl Process {
    pub(crate) fn start(config: &Path, application: bool) -> Self {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"));
        command.args(["stdio", "--config"]).arg(config);
        if application {
            command.arg("--application");
        }
        let mut child = command
            .kill_on_drop(true)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let lines = BufReader::new(child.stdout.take().unwrap()).lines();
        Self {
            child,
            input,
            lines,
        }
    }
    pub(crate) async fn exchange(&mut self, value: Value) -> Value {
        let id = value["id"].clone();
        self.raw(&serde_json::to_vec(&value).unwrap()).await;
        let line = tokio::time::timeout(Duration::from_secs(15), self.lines.next_line())
            .await
            .unwrap()
            .unwrap()
            .expect("correlated helper response");
        assert!(line.len() <= 2 * 1024 * 1024);
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], id);
        assert_eq!(response["response"]["profile"], "connect-control/1");
        response["response"]["result"].clone()
    }
    async fn raw(&mut self, bytes: &[u8]) {
        let input = self.input.as_mut().unwrap();
        input.write_all(bytes).await.unwrap();
        input.write_all(b"\n").await.unwrap();
        input.flush().await.unwrap();
    }
    pub(crate) async fn finish(mut self, success: bool, secrets: &[&str]) {
        drop(self.input.take());
        assert!(
            tokio::time::timeout(Duration::from_secs(15), self.lines.next_line())
                .await
                .unwrap()
                .unwrap()
                .is_none(),
            "unexpected extra helper stdout"
        );
        let output = tokio::time::timeout(Duration::from_secs(15), self.child.wait_with_output())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(output.status.success(), success);
        for secret in secrets {
            assert!(!String::from_utf8_lossy(&output.stderr).contains(secret));
        }
    }
}
fn command(
    id: &str,
    kind: &str,
    value: Value,
    cid: Option<&str>,
    operation: Option<&str>,
) -> Value {
    json!({"id":id,"command":{"type":kind,"payload":value},"scope":{"conversation_id":cid,"operation_id":operation}})
}
fn frames(result: &Value) -> &Vec<Value> {
    assert!(result.get("Err").is_none(), "{result}");
    assert_eq!(result["Ok"]["kind"], "canonical_reply");
    result["Ok"]["data"]["frames"].as_array().unwrap()
}
fn record(result: &Value) -> Value {
    frames(result)
        .iter()
        .find(|f| f["type"] == "operation.updated")
        .unwrap()["payload"]
        .clone()
}

async fn one_shot(path: &Path, mut input: Value) -> (bool, Value) {
    input.as_object_mut().unwrap().remove("id");
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"))
        .args(["request", "--envelope", "--config"])
        .arg(path)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut writer = child.stdin.take().unwrap();
    writer
        .write_all(&serde_json::to_vec(&input).unwrap())
        .await
        .unwrap();
    drop(writer);
    let output = tokio::time::timeout(Duration::from_secs(15), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["profile"], "connect-control/1");
    (output.status.success(), value["result"].clone())
}
#[tokio::test]
async fn canonical_stdio_preserves_original_recovery_finite_events_current_authority_and_fixed_configuration(
) {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_membership_scope(algorithm, 8, true).await;
        let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
        let cid = create(&f, &mut native, "stdio-room").await;
        let path = f.temp.path().join("stdio-native.toml");
        init::private_write(&path, toml::to_string(&f.client).unwrap().as_bytes()).unwrap();
        let mut helper = Process::start(&path, false);
        let sent = command(
            "send",
            "message.send",
            json!({"msg_id":"stdio-message","to":"all","content_type":"text/plain","body":"stdio saved original","body_encoding":"utf8","label":"connect-messages","attachments":[],"in_reply_to_msg":null}),
            Some(&cid),
            Some("stdio-send"),
        );
        frames(&helper.exchange(sent.clone()).await);
        // A running helper never rereads a replacement file or broadens authority.
        std::fs::write(&path, "not a client TOML").unwrap();
        let get = command(
            "get",
            "operation.get",
            json!({"operation_id":"stdio-send"}),
            Some(&cid),
            None,
        );
        let original = record(&helper.exchange(get.clone()).await);
        assert_eq!(original["state"], "completed");
        frames(&helper.exchange(sent.clone()).await);
        for live in [Value::Null, json!(true)] {
            let mut input = command(
                "live",
                "conversation.subscribe",
                json!({}),
                Some(&cid),
                None,
            );
            if !live.is_null() {
                input["command"]["payload"]["live"] = live;
            }
            assert_eq!(
                helper.exchange(input).await["Err"]["code"],
                "invalid-request"
            );
        }
        let finite = helper
            .exchange(command(
                "events",
                "conversation.subscribe",
                json!({"live":false,"limit":100}),
                Some(&cid),
                None,
            ))
            .await;
        assert!(frames(&finite)
            .iter()
            .any(|f| f["type"] == "conversation.snapshot"));
        helper.finish(true, &[&f.client.credential]).await;
        std::fs::write(&path, toml::to_string(&f.client).unwrap()).unwrap();
        let mut recovered = Process::start(&path, false);
        let current = record(&recovered.exchange(get.clone()).await);
        assert_eq!(current["record_id"], original["record_id"]);
        assert_eq!(current["result"], original["result"]);
        let (success, read) = one_shot(&path, get).await;
        assert!(success);
        assert_eq!(record(&read)["record_id"], original["record_id"]);
        let (success, rejected) = one_shot(
            &path,
            command(
                "live-once",
                "conversation.subscribe",
                json!({"live":true}),
                Some(&cid),
                None,
            ),
        )
        .await;
        assert!(!success);
        assert_eq!(rejected["Err"]["code"], "invalid-request");
        frames(&recovered.exchange(sent.clone()).await);
        let history = recovered
            .exchange(command(
                "history",
                "message.history",
                json!({"limit":100}),
                Some(&cid),
                None,
            ))
            .await;
        assert_eq!(frames(&history).iter().filter(|f|f["type"]=="message.send"&&f["payload"]["msg_id"]=="stdio-message").count(),1);
        let principal = f.server.store.authenticate(&f.client.credential).unwrap();
        let admin = f.server.store.authenticate(&f.admin).unwrap();
        let store = f.server.store.clone();
        tokio::task::spawn_blocking(move || store.revoke(&admin, &principal.credential_id))
            .await
            .unwrap()
            .unwrap();
        assert!(
            recovered.exchange(sent).await.get("Err").is_some(),
            "replay must enforce current authority"
        );
        recovered.finish(true, &[&f.client.credential]).await;
        let _ = native.shutdown().await;
        f.server.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn canonical_stdio_caches_startup_refusal_and_rejects_wrong_mode_unknown_and_oversized_input()
{
    let f = Fixture::new("ES256", 8).await;
    let config = f.temp.path().join("absent.toml");
    let mut helper = Process::start(&config, false);
    let list = command(
        "missing",
        "conversation.list",
        json!({"limit":1}),
        None,
        None,
    );
    assert_eq!(
        helper.exchange(list.clone()).await["Err"]["code"],
        "configuration"
    );
    init::private_write(&config, toml::to_string(&f.client).unwrap().as_bytes()).unwrap();
    assert_eq!(helper.exchange(list).await["Err"]["code"], "configuration");
    helper.finish(true, &[]).await;
    let mut repaired = Process::start(&config, false);
    frames(
        &repaired
            .exchange(command(
                "repaired",
                "conversation.list",
                json!({"limit":1}),
                None,
                None,
            ))
            .await,
    );
    repaired.finish(true, &[&f.client.credential]).await;
    for input in [
        json!({"id":"old","operation":{"op":"health"}}),
        json!({"id":"unknown","command":{"type":"conversation.list","payload":{}},"credential":"must-not-be-echoed"}),
    ] {
        let mut helper = Process::start(&config, false);
        helper.raw(&serde_json::to_vec(&input).unwrap()).await;
        helper.finish(false, &["must-not-be-echoed"]).await;
    }
    let mut helper = Process::start(&config, false);
    let _ = helper
        .input
        .as_mut()
        .unwrap()
        .write_all(&vec![b'x'; 65_537])
        .await;
    helper.finish(false, &[]).await;
    f.server.shutdown().await.unwrap();
}
