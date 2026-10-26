//! Real CLI and public HTTP/native backend, including committed lost responses.
use super::*;
use connect_client::application::{
    credential_issuance::CredentialIssueRequest, ApplicationClient, ApplicationClientConfig,
};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
};
use tokio_util::sync::CancellationToken;

fn write(path: &Path, bytes: &[u8]) {
    connect_server::cli::init::private_write(path, bytes).unwrap();
}
fn app(f: &Fixture, endpoint: SocketAddr) -> ApplicationClientConfig {
    ApplicationClientConfig {
        endpoint,
        credential: f.admin.clone(),
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        timeout_ms: 2000,
    }
}
struct Input {
    admin: std::path::PathBuf,
    template: std::path::PathBuf,
    request: std::path::PathBuf,
    output: std::path::PathBuf,
}
impl Input {
    fn new(f: &Fixture, endpoint: SocketAddr, native_endpoint: SocketAddr) -> Self {
        let admin = f.temp.path().join("issuer.application.toml");
        write(
            &admin,
            toml::to_string_pretty(&app(f, endpoint))
                .unwrap()
                .as_bytes(),
        );
        let mut source = f.client.clone();
        if source.server_algorithm == "ES256" {
            // Exercise the SDK's explicit PEM trust bundle profile, including
            // a second independent configured root without system trust.
            let der = std::fs::read(&source.trust_certificate).unwrap();
            let encoded = base64::engine::general_purpose::STANDARD.encode(der);
            let lines = encoded
                .as_bytes()
                .chunks(64)
                .map(|v| std::str::from_utf8(v).unwrap())
                .collect::<Vec<_>>()
                .join("\n");
            let other =
                rcgen::generate_simple_self_signed(vec!["unused-root.invalid".into()]).unwrap();
            let pem = format!(
                "-----BEGIN CERTIFICATE-----\n{lines}\n-----END CERTIFICATE-----\n{}",
                other.cert.pem()
            );
            let path = f.temp.path().join("contact-roots.pem");
            write(&path, pem.as_bytes());
            source.trust_certificate = path;
        }
        source.endpoint = native_endpoint;
        source.timeout_ms = 400;
        // A contact template does not need its old signer to be present/readable.
        source.signing_private_key = f.temp.path().join("absent-parent-signer.der");
        let template = f.temp.path().join("contact.toml");
        write(
            &template,
            toml::to_string_pretty(&source).unwrap().as_bytes(),
        );
        let request = f.temp.path().join("credential-request.json");
        write(
            &request,
            &serde_json::to_vec(&CredentialIssueRequest {
                operation_id: "cli-credential-original".into(),
                identity_id: f.client.identity_id.clone(),
                operations: vec!["operation.get".into()],
                recipients: vec![f.peer.identity_id.clone()],
                expires_at: chrono::Utc::now().timestamp() + 240,
            })
            .unwrap(),
        );
        Self {
            admin,
            template,
            request,
            output: f.temp.path().join("issued"),
        }
    }
    async fn issue(&self, f: &Fixture, algorithm: &str) -> std::process::Output {
        let output = tokio::time::timeout(
            Duration::from_secs(15),
            tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"))
                .env_clear()
                .env("HOME", f.temp.path())
                .env("PATH", "/usr/bin:/bin")
                .kill_on_drop(true)
                .args(["issue", "--config"])
                .arg(&self.admin)
                .arg("--native-template")
                .arg(&self.template)
                .arg("--request")
                .arg(&self.request)
                .arg("--output")
                .arg(&self.output)
                .args(["--signing-algorithm", algorithm])
                .output(),
        )
        .await
        .unwrap()
        .unwrap();
        self.redacted(f, &output);
        output
    }
    async fn resume(&self, f: &Fixture) -> std::process::Output {
        let output = tokio::time::timeout(
            Duration::from_secs(15),
            tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"))
                .env_clear()
                .env("HOME", f.temp.path())
                .env("PATH", "/usr/bin:/bin")
                .kill_on_drop(true)
                .args(["issue-resume", "--config"])
                .arg(&self.admin)
                .arg("--installation")
                .arg(&self.output)
                .output(),
        )
        .await
        .unwrap()
        .unwrap();
        self.redacted(f, &output);
        output
    }
    fn redacted(&self, f: &Fixture, output: &std::process::Output) {
        assert!(output.stdout.len() + output.stderr.len() < 65_536);
        let mut secrets = vec![f.admin.clone(), f.client.credential.clone()];
        if let Ok(bytes) = std::fs::read(self.output.join("issued.json")) {
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            secrets.push(value["credential"].as_str().unwrap().into());
        }
        for secret in secrets {
            assert!(!output
                .stdout
                .windows(secret.len())
                .any(|v| v == secret.as_bytes()));
            assert!(!output
                .stderr
                .windows(secret.len())
                .any(|v| v == secret.as_bytes()));
        }
    }
}
fn snapshot(path: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(path)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().to_str().unwrap().to_owned(),
                std::fs::read(entry.path()).unwrap(),
            )
        })
        .collect()
}
fn counts(f: &Fixture) -> (u64, u64, u64, u64) {
    let c = rusqlite::Connection::open_with_flags(
        f.server.store.path(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    c.query_row("SELECT (SELECT COUNT(*) FROM credentials),(SELECT COUNT(*) FROM operations),(SELECT COUNT(*) FROM devices),(SELECT COUNT(*) FROM peer_signing_keys)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap()
}
fn exactly_one(f: &Fixture, before: (u64, u64, u64, u64)) {
    assert_eq!(
        counts(f),
        (before.0 + 1, before.1 + 3, before.2 + 1, before.3 + 1)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_cli_issues_independent_native_assets_rechecks_completion_and_exports_scoped_agent()
{
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_configured_mcp(algorithm, &[]).await;
        let input = Input::new(&f, f.server.http_address.unwrap(), f.server.address);
        let before = counts(&f);
        let result = input.issue(&f, algorithm).await;
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(report["status"], "credential-installed-or-reconciled");
        let saved = snapshot(&input.output);
        assert_eq!(saved.len(), 10);
        exactly_one(&f, before);
        use std::os::unix::fs::PermissionsExt;
        for path in std::fs::read_dir(&input.output).unwrap() {
            assert_eq!(
                path.unwrap().metadata().unwrap().permissions().mode() & 0o077,
                0
            );
        }
        let intent: Value = serde_json::from_slice(&saved["intent.json"]).unwrap();
        assert_ne!(intent["native"]["device_id"], f.client.device_id);
        assert_ne!(intent["native"]["signing_key_id"], f.client.signing_key_id);
        assert_eq!(
            intent["native"]["credential"],
            "pending-administrative-issuance"
        );
        for bytes in saved.values() {
            for secret in [&f.admin, &f.client.credential] {
                assert!(!bytes.windows(secret.len()).any(|v| v == secret.as_bytes()));
            }
        }
        let native = CanonicalClientConfig::load(&input.output.join("client.toml")).unwrap();
        assert_ne!(native.credential, f.admin);
        assert_ne!(native.credential, f.client.credential);
        CanonicalClient::open(native.clone())
            .await
            .unwrap()
            .shutdown()
            .await
            .unwrap();
        write(&input.output.join("notes.txt"), b"unrelated saved work");
        let saved = snapshot(&input.output);
        assert!(input.resume(&f).await.status.success());
        assert!(!input.issue(&f, algorithm).await.status.success());
        assert_eq!(snapshot(&input.output), saved);
        exactly_one(&f, before);
        // A different valid credential cannot silently become the original issuer.
        let mut changed = app(&f, f.server.http_address.unwrap());
        changed.credential = f.client.credential.clone();
        std::fs::write(&input.admin, toml::to_string_pretty(&changed).unwrap()).unwrap();
        assert!(!input.resume(&f).await.status.success());
        changed = app(&f, f.server.http_address.unwrap());
        changed.endpoint.set_port(1);
        std::fs::write(&input.admin, toml::to_string_pretty(&changed).unwrap()).unwrap();
        assert!(!input.resume(&f).await.status.success());
        std::fs::write(
            &input.admin,
            toml::to_string_pretty(&app(&f, f.server.http_address.unwrap())).unwrap(),
        )
        .unwrap();
        // A completed config may not be silently edited or reconstituted.
        std::fs::write(
            input.output.join("client.toml"),
            [saved["client.toml"].as_slice(), b"\n# changed"].concat(),
        )
        .unwrap();
        assert!(!input.resume(&f).await.status.success());
        std::fs::write(input.output.join("client.toml"), &saved["client.toml"]).unwrap();
        assert_eq!(snapshot(&input.output), saved);
        exactly_one(&f, before);
        let server_config = f.temp.path().join("canonical-server.toml");
        write(
            &server_config,
            toml::to_string_pretty(&f.config).unwrap().as_bytes(),
        );
        let export = f.temp.path().join("agent-export");
        let exported = tokio::time::timeout(
            Duration::from_secs(10),
            tokio::process::Command::new(env!("CARGO_BIN_EXE_connect"))
                .env_clear()
                .env("HOME", f.temp.path())
                .env("PATH", "/usr/bin:/bin")
                .kill_on_drop(true)
                .args(["agent-access", "export-codex", "--config"])
                .arg(input.output.join("client.toml"))
                .arg("--server-config")
                .arg(server_config)
                .arg("--mcp-address")
                .arg(f.server.mcp_address.unwrap().to_string())
                .arg("--output")
                .arg(&export)
                .output(),
        )
        .await
        .unwrap()
        .unwrap();
        input.redacted(&f, &exported);
        assert!(
            exported.status.success(),
            "{}",
            String::from_utf8_lossy(&exported.stderr)
        );
        let exported: toml::Value =
            toml::from_str(&std::fs::read_to_string(export.join("agent.toml")).unwrap()).unwrap();
        assert_eq!(exported["credential"].as_str().unwrap(), native.credential);
        assert_eq!(exported.as_table().unwrap().len(), 4);
        let mut admin = ApplicationClient::open(app(&f, f.server.http_address.unwrap()))
            .await
            .unwrap();
        admin
            .provision(ProvisioningCommand::RevokeSigningKey {
                operation_id: "revoke-cli-issued-key".into(),
                key_id: native.signing_key_id.clone(),
                expected_revision: 0,
            })
            .await
            .unwrap();
        assert!(!input.resume(&f).await.status.success());
        assert_eq!(snapshot(&input.output), saved);
        assert_eq!(
            counts(&f),
            (before.0 + 1, before.1 + 4, before.2 + 1, before.3 + 1)
        );
        admin
            .provision(ProvisioningCommand::RevokeCredential {
                operation_id: "revoke-cli-issued".into(),
                credential_id: report["credential_id"].as_str().unwrap().into(),
            })
            .await
            .unwrap();
        assert!(!input.resume(&f).await.status.success());
        assert_eq!(snapshot(&input.output), saved);
        assert!(CanonicalClient::open(native).await.is_err());
        f.server.shutdown().await.unwrap();
    }
}

struct Proxy {
    address: SocketAddr,
    stop: CancellationToken,
    task: tokio::task::JoinHandle<()>,
    cut: Arc<AtomicBool>,
}
impl Proxy {
    async fn new(backend: SocketAddr, phase: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let stop = CancellationToken::new();
        let cancelled = stop.clone();
        let cut = Arc::new(AtomicBool::new(false));
        let observed = cut.clone();
        let task = tokio::spawn(async move {
            let mut workers = tokio::task::JoinSet::new();
            loop {
                tokio::select! {biased;
                    _=cancelled.cancelled()=>break,
                    Some(result)=workers.join_next(),if !workers.is_empty()=>{result.unwrap();},
                    accepted=listener.accept(),if workers.len()<8=>{
                        let (mut incoming,_)=accepted.unwrap();let cut=observed.clone();
                        workers.spawn(async move {tokio::time::timeout(Duration::from_secs(5),async {
                            let mut header=Vec::new();let mut byte=[0u8;1];
                            while !header.ends_with(b"\r\n\r\n") {incoming.read_exact(&mut byte).await.unwrap();header.push(byte[0]);assert!(header.len()<16_384);}
                            let header=String::from_utf8(header).unwrap();let path=header.lines().next().unwrap().split_whitespace().nth(1).unwrap().to_owned();
                            let length:usize=header.lines().find_map(|v|v.to_ascii_lowercase().strip_prefix("content-length: ").map(str::to_owned)).unwrap().parse().unwrap();assert!(length<=65_536);
                            let mut body=vec![0;length];incoming.read_exact(&mut body).await.unwrap();
                            let mut forwarded=header.lines().filter(|v|!v.is_empty()&&!v.to_ascii_lowercase().starts_with("host:")&&!v.to_ascii_lowercase().starts_with("connection:")).map(str::to_owned).collect::<Vec<_>>();
                            forwarded.push(format!("Host: {backend}"));forwarded.push("Connection: close".into());
                            let mut upstream=TcpStream::connect(backend).await.unwrap();upstream.write_all((forwarded.join("\r\n")+"\r\n\r\n").as_bytes()).await.unwrap();upstream.write_all(&body).await.unwrap();
                            let mut response=Vec::new();(&mut upstream).take(2*1024*1024+1).read_to_end(&mut response).await.unwrap();assert!(response.len()<=2*1024*1024);
                            let matches=match phase {"issue"=>path=="/v2/credentials","device"=>path.ends_with("/endpoints"),"key"=>path.ends_with("/signing-keys"),_=>false};
                            if matches&&!cut.swap(true,Ordering::SeqCst) {assert!(response.starts_with(b"HTTP/1.1 200"));return;}
                            incoming.write_all(&response).await.unwrap();
                        }).await.unwrap();});
                    }
                }
            }
            workers.abort_all();
            while let Some(result) = workers.join_next().await {
                if let Err(e) = result {
                    assert!(e.is_cancelled());
                }
            }
        });
        Self {
            address,
            stop,
            task,
            cut,
        }
    }
    async fn close(self) {
        self.stop.cancel();
        self.task.await.unwrap();
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn actual_cli_resumes_committed_lost_issuance_device_key_and_failed_native_admission() {
    for algorithm in ["EdDSA", "ES256"] {
        for phase in ["issue", "device", "key", "native"] {
            let f = Fixture::new(algorithm, 8).await;
            let proxy = Proxy::new(f.server.http_address.unwrap(), phase).await;
            let gate = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let native_endpoint = if phase == "native" {
                gate.local_addr().unwrap()
            } else {
                f.server.address
            };
            let input = Input::new(&f, proxy.address, native_endpoint);
            let before = counts(&f);
            let failed = input.issue(&f, algorithm).await;
            assert!(!failed.status.success(), "{phase}");
            assert!(!input.output.join("install.json").exists());
            assert_eq!(proxy.cut.load(Ordering::SeqCst), phase != "native");
            let partial = snapshot(&input.output);
            if phase == "issue" {
                let before_edit = counts(&f);
                let mut edited: Value = serde_json::from_slice(&partial["intent.json"]).unwrap();
                edited["request"]["operation_id"] = json!("replacement-logical-id");
                std::fs::write(
                    input.output.join("intent.json"),
                    serde_json::to_vec(&edited).unwrap(),
                )
                .unwrap();
                assert!(!input.resume(&f).await.status.success());
                assert_eq!(counts(&f), before_edit);
                std::fs::write(input.output.join("intent.json"), &partial["intent.json"]).unwrap();
            }
            assert!(
                partial.contains_key("intent.json") && partial.contains_key("device-signing.der")
            );
            assert_eq!(partial.contains_key("issued.json"), phase != "issue");
            assert_eq!(partial.contains_key("client.toml"), phase == "native");
            // Open the same native UDP endpoint only after the failed attempt. No
            // saved contact, Device, key, credential or operation ID changes.
            let stop = CancellationToken::new();
            let cancelled = stop.clone();
            let backend = f.server.address;
            let forwarding = tokio::spawn(async move {
                let mut client = None;
                let mut bytes = vec![0; 65_536];
                loop {
                    tokio::select! {
                        _=cancelled.cancelled()=>break,
                        received=gate.recv_from(&mut bytes)=>{let (size,source)=received.unwrap();if source==backend {if let Some(peer)=client {gate.send_to(&bytes[..size],peer).await.unwrap();}}else{client=Some(source);gate.send_to(&bytes[..size],backend).await.unwrap();}}
                    }
                }
            });
            let recovered = input.resume(&f).await;
            assert!(
                recovered.status.success(),
                "{phase}: {}",
                String::from_utf8_lossy(&recovered.stderr)
            );
            exactly_one(&f, before);
            let complete = snapshot(&input.output);
            for (name, bytes) in partial {
                assert_eq!(complete[&name], bytes, "{phase} changed {name}");
            }
            assert!(input.resume(&f).await.status.success());
            assert_eq!(snapshot(&input.output), complete);
            exactly_one(&f, before);
            stop.cancel();
            forwarding.await.unwrap();
            proxy.close().await;
            f.server.shutdown().await.unwrap();
        }
    }
}
