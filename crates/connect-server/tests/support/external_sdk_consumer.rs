//! Bounded subprocess custody for the independent public-crate consumer.
use super::*;
use std::{
    path::PathBuf,
    process::Stdio,
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
pub(crate) fn enabled() -> bool {
    std::env::var_os("CONNECT_GATE6_EXTERNAL_SDK").is_some()
}
pub(crate) fn require() {
    assert!(
        enabled(),
        "build the independent external crate and explicitly set CONNECT_GATE6_EXTERNAL_SDK"
    );
}
pub(crate) struct Consumer {
    child: tokio::process::Child,
    lines: Lines<BufReader<tokio::process::ChildStdout>>,
    stderr: PathBuf,
    pub reference: Option<Value>,
    pub command: Value,
}
impl Consumer {
    pub(crate) async fn start(f: &Fixture, input: Value) -> Self {
        require();
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let index = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = f
            .temp
            .path()
            .join(format!("external-sdk-case-{index}.json"));
        let stderr = f
            .temp
            .path()
            .join(format!("external-sdk-case-{index}.stderr"));
        std::fs::write(&path, serde_json::to_vec(&input).unwrap()).unwrap();
        let mut child =
            tokio::process::Command::new(std::env::var_os("CONNECT_GATE6_EXTERNAL_SDK").unwrap())
                .arg(path)
                .current_dir(f.temp.path())
                .kill_on_drop(true)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::from(std::fs::File::create(&stderr).unwrap()))
                .spawn()
                .unwrap();
        let lines = BufReader::new(child.stdout.take().unwrap()).lines();
        Self {
            child,
            lines,
            stderr,
            reference: None,
            command: input.get("command").cloned().unwrap_or(Value::Null),
        }
    }
    pub(crate) async fn next(&mut self) -> Value {
        let line = tokio::time::timeout(Duration::from_secs(45), self.lines.next_line())
            .await
            .expect("external SDK output deadline")
            .unwrap()
            .unwrap_or_else(|| {
                panic!(
                    "external SDK ended without a result: {}",
                    std::fs::read_to_string(&self.stderr).unwrap()
                )
            });
        serde_json::from_str(&line).unwrap()
    }
    pub(crate) async fn registered(&mut self) {
        assert_eq!(self.next().await["phase"], "registered");
    }
    pub(crate) async fn begin(&mut self) {
        let mut input = self.child.stdin.take().unwrap();
        input.write_all(b"continue\n").await.unwrap();
        input.shutdown().await.unwrap();
        let accepted = self.next().await;
        assert_eq!(accepted["phase"], "accepted");
        self.reference = Some(accepted["reference"].clone());
    }
    pub(crate) async fn finish(mut self) -> Value {
        let value = self.next().await;
        let status = tokio::time::timeout(Duration::from_secs(5), self.child.wait())
            .await
            .expect("external SDK join deadline")
            .unwrap();
        assert!(
            status.success(),
            "external SDK failed: {}",
            std::fs::read_to_string(&self.stderr).unwrap()
        );
        assert!(matches!(
            value["phase"].as_str(),
            Some("completed" | "stopped" | "error")
        ));
        eprintln!("External public SDK result: {value}");
        value
    }
}
pub(crate) fn configured(control: CanonicalClientConfig) -> connect_client::sdk::ClientConfig {
    connect_client::sdk::ClientConfig {
        control,
        tenant_id: "tnt_default".parse().unwrap(),
        application: None,
    }
}

#[tokio::test]
#[ignore = "requires a built independent consumer via CONNECT_GATE6_EXTERNAL_SDK"]
async fn external_public_crate_message_session_invitation_and_independently_decoded_media() {
    require();
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 16).await;
        super::canonical_native_connections::ready_endpoint_with_capabilities(
            &f,
            vec!["audio".into(), "video".into()],
        )
        .await;
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope.operations.extend([
            "stream.subscribe".into(),
            "stream.unsubscribe".into(),
            "connection.end".into(),
        ]);
        let target = super::canonical_native_connections::party_with_scope(
            &f,
            &f.peer.identity_id,
            943,
            vec!["audio".into(), "video".into()],
            scope,
        );
        let consumer=Consumer::start(&f,json!({"mode":"media","publisher":configured(f.client.clone()),"subscriber":configured(target),"output":f.temp.path().join("external-decoded")})).await;
        let result = consumer.finish().await;
        assert_eq!(result["phase"], "completed");
        assert_eq!(result["owners_joined"], true);
        assert_eq!(result["summary"]["actual_invitation"], true);
        assert_eq!(result["summary"]["session_state"], "ended");
        assert!(
            result["summary"]["media"]["decoded_loud_samples"]
                .as_u64()
                .unwrap()
                > 500
        );
        assert_eq!(
            result["summary"]["media"]["decoded_rgb_bytes"],
            640 * 360 * 3
        );
        let db = rusqlite::Connection::open(&f.config.database).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM messages WHERE id='external-message'",
                [],
                |r| r.get::<_, u64>(0)
            )
            .unwrap(),
            1
        );
        let until = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let active = db
                .query_row(
                    "SELECT COUNT(*) FROM native_connection_lifetimes WHERE retired=0",
                    [],
                    |r| r.get::<_, u64>(0),
                )
                .unwrap();
            if active == 0 {
                break;
            }
            assert!(
                tokio::time::Instant::now() < until,
                "external process left native ownership"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        f.server.shutdown().await.unwrap();
        eprintln!("External crate {algorithm}: private Message once, actual incoming Session, public SDK both-end media, identified independent audio/video decode, ended Session and no live native owners");
    }
}
