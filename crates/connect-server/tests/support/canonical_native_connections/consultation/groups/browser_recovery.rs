//! Actual browser owner observing live native transitions. Operator and TLS
//! private authority stay in this fixture; the page gets its scoped enrollment.
use super::*;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};

pub(super) struct Setup {
    asset: std::net::TcpListener,
    browser: &'static str,
}
pub(super) async fn configure(f: Fixture, browser: &'static str) -> (Fixture, Setup) {
    assert!(["chrome", "firefox"].contains(&browser));
    let asset = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let Fixture {
        temp,
        mut config,
        server,
        mut client,
        peer,
        admin,
        key_revision,
    } = f;
    server.shutdown().await.unwrap();
    let mut parameters =
        rcgen::CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
    let now = std::time::SystemTime::now();
    parameters.not_before = (now - Duration::from_secs(60)).into();
    parameters.not_after = (now + Duration::from_secs(3600)).into();
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let certificate = parameters.self_signed(&key).unwrap();
    std::fs::write(&config.certificate, certificate.der()).unwrap();
    std::fs::write(&config.private_key, key.serialize_der()).unwrap();
    config.webtransport = Some(
        connect_server::canonical_webtransport::CanonicalWebTransportConfig {
            mount_path: "/connect".into(),
            allowed_origins: vec![format!(
                "http://127.0.0.1:{}",
                asset.local_addr().unwrap().port()
            )],
            advertised_url: None,
        },
    );
    let server = RunningCanonicalServer::start(config.clone()).await.unwrap();
    client.endpoint = server.address;
    (
        Fixture {
            temp,
            config,
            server,
            client,
            peer,
            admin,
            key_revision,
        },
        Setup { asset, browser },
    )
}
struct OwnedBrowser(Child);
impl Drop for OwnedBrowser {
    fn drop(&mut self) {
        let id = self.0.id();
        let _ = self.0.kill();
        #[cfg(unix)]
        let _ = Command::new("/bin/kill")
            .args(["-KILL", "--", &format!("-{id}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = self.0.wait();
    }
}
pub(super) struct Worker {
    child: tokio::process::Child,
    input: tokio::process::ChildStdin,
    output: BufReader<tokio::process::ChildStdout>,
    browser: Option<OwnedBrowser>,
    public_key: Vec<u8>,
    key_revision: Option<u64>,
    key_not_before: i64,
    key_expires: i64,
    phase: usize,
    reports: Vec<Value>,
    name: &'static str,
}
impl Worker {
    pub(super) async fn start(f: &Fixture, setup: Setup) -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let profile = f.temp.path().join("live-control-profile");
        init::private_directory(&profile).unwrap();
        let browser_log = f.temp.path().join("live-control-browser.log");
        let binary = std::env::var_os(if setup.browser == "chrome" {
            "CONNECT_GATE6_CHROME"
        } else {
            "CONNECT_GATE6_FIREFOX"
        })
        .unwrap();
        if setup.browser == "firefox" {
            let version = Command::new(&binary).arg("--version").output().unwrap();
            assert_eq!(
                String::from_utf8(version.stdout).unwrap().trim(),
                "Mozilla Firefox 158.0.1"
            );
        }
        let mut command = Command::new(binary);
        if setup.browser == "chrome" {
            command.args([
                "--headless=new",
                "--remote-debugging-address=127.0.0.1",
                "--remote-debugging-port=0",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-background-networking",
                "--disable-sync",
                "--disable-breakpad",
                "--disable-crash-reporter",
            ]);
            command.arg(format!("--user-data-dir={}", profile.display()));
        } else {
            command
                .args([
                    "--headless",
                    "--no-remote",
                    "--remote-debugging-port",
                    "0",
                    "--profile",
                ])
                .arg(&profile);
        }
        command
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&browser_log).unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut browser = OwnedBrowser(command.spawn().unwrap());
        let until = tokio::time::Instant::now() + Duration::from_secs(15);
        let debug_port = loop {
            if setup.browser == "firefox" {
                if let Ok(text) = std::fs::read_to_string(&browser_log) {
                    if let Some(port) = text
                        .lines()
                        .filter_map(|l| l.split("WebDriver BiDi listening on ").nth(1))
                        .filter_map(|u| url::Url::parse(u).ok())
                        .filter(|u| u.host_str() == Some("127.0.0.1"))
                        .find_map(|u| u.port())
                    {
                        break port;
                    }
                }
            }
            if let Ok(text) = std::fs::read_to_string(profile.join("DevToolsActivePort")) {
                if let Some(port) = text.lines().next().and_then(|s| s.parse::<u16>().ok()) {
                    break port;
                }
            }
            assert!(
                browser.0.try_wait().unwrap().is_none(),
                "owned browser exited before debugging ready"
            );
            assert!(
                tokio::time::Instant::now() < until,
                "owned browser debugging deadline"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        let private = f.temp.path().join("live-browser-control-key.der");
        let public_key = signing(&private, &f.client.signing_algorithm);
        let admin = f.server.store.authenticate(&f.admin).unwrap();
        f.server
            .store
            .execute_provisioning(
                &admin.credential_id,
                ProvisioningCommand::Endpoint {
                    operation_id: "live-control-device".into(),
                    identity_id: f.client.identity_id.clone(),
                    device: DeviceSpec {
                        id: "live-control-device".into(),
                        kind: "browser".into(),
                        platform: "fixture".into(),
                    },
                    transport: EndpointTransport::Quic,
                    address: format!("quic://localhost:{}/live-control", f.server.address.port()),
                    capabilities: vec!["data".into()],
                    secret_ref: None,
                },
            )
            .unwrap();
        let initial = json!({"root":root,"browser":setup.browser,"assetPort":setup.asset.local_addr().unwrap().port(),"debugPort":debug_port,
            "url":f.server.webtransport_url,"identityId":f.client.identity_id,"device":{"id":"live-control-device","kind":"browser","platform":"fixture"},
            "signing":{"algorithm":f.client.signing_algorithm,"keyId":"live-control-key","privatePkcs8":URL_SAFE_NO_PAD.encode(std::fs::read(private).unwrap())},
            "server":{"algorithm":f.client.server_algorithm,"keyId":f.client.server_key_id,"publicRaw":URL_SAFE_NO_PAD.encode(std::fs::read(&f.client.server_public_key).unwrap()),"notBefore":f.client.server_key_not_before.timestamp(),"expiresAt":f.client.server_key_expires_at.timestamp()},
            "certificateHash":Sha256::digest(std::fs::read(&f.config.certificate).unwrap()).to_vec()});
        drop(setup.asset);
        let mut child =
            tokio::process::Command::new(std::env::var_os("CONNECT_GATE6_NODE").unwrap())
                .arg(root.join("scripts/video/browser-live-recovery.mjs"))
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(
                    std::fs::File::create(f.temp.path().join("live-control-worker.log")).unwrap(),
                )
                .kill_on_drop(true)
                .spawn()
                .unwrap();
        let mut worker = Self {
            input: child.stdin.take().unwrap(),
            output: BufReader::new(child.stdout.take().unwrap()),
            child,
            browser: Some(browser),
            public_key,
            key_revision: None,
            key_not_before: chrono::Utc::now().timestamp() - 1,
            key_expires: chrono::Utc::now().timestamp() + 3600,
            phase: 0,
            reports: Vec::new(),
            name: setup.browser,
        };
        assert_eq!(worker.exchange(initial).await["ready"], true);
        worker
    }
    async fn exchange(&mut self, request: Value) -> Value {
        let mut bytes = serde_json::to_vec(&request).unwrap();
        assert!(bytes.len() < 65536);
        bytes.push(b'\n');
        tokio::time::timeout(Duration::from_secs(20), async {
            self.input.write_all(&bytes).await.unwrap();
            self.input.flush().await.unwrap();
            let mut response = Vec::new();
            loop {
                let b = self
                    .output
                    .read_u8()
                    .await
                    .expect("owned browser worker reply");
                if b == b'\n' {
                    break;
                }
                assert!(response.len() < 65536);
                response.push(b);
            }
            serde_json::from_slice(&response).unwrap()
        })
        .await
        .expect("bounded browser controller exchange")
    }
    pub(super) async fn recover(
        &mut self,
        f: &Fixture,
        command: &TransitionCommand,
        token: &str,
        expected: &str,
    ) -> Value {
        self.phase += 1;
        let store = f.server.store.clone();
        let admin = f.admin.clone();
        let token = token.to_owned();
        let identity = f.client.identity_id.clone();
        let binding = ProvisioningCommand::SigningKey {
            operation_id: format!("live-control-bind-{}", self.phase),
            identity_id: identity.clone(),
            credential_id: store.authenticate(&token).unwrap().credential_id,
            device_id: "live-control-device".into(),
            key_id: "live-control-key".into(),
            algorithm: if f.client.signing_algorithm == "ES256" {
                SigningAlgorithm::ES256
            } else {
                SigningAlgorithm::EdDSA
            },
            public_key: URL_SAFE_NO_PAD.encode(&self.public_key),
            not_before: self.key_not_before,
            expires_at: self.key_expires,
            expected_revision: self.key_revision,
        };
        let result = tokio::task::spawn_blocking(move || {
            let admin = store.authenticate(&admin).unwrap();
            store
                .execute_provisioning(&admin.credential_id, binding)
                .unwrap()
        })
        .await
        .unwrap();
        let ProvisioningOutcome::SigningKey { revision, .. } = result else {
            panic!("control signing enrollment")
        };
        self.key_revision = Some(revision);
        let reference = json!({"operationId":command.operation_id,"command":command.intent.required_scope(),"scope":{"conversationId":command.conversation_id,"sessionId":command.session_id,"connectionId":command.connection_id}});
        let result=self.exchange(json!({"mode":"recover","credential":token,"reference":reference,"expectedState":expected})).await;
        assert_eq!(result["passed"], true);
        assert_eq!(result["state"], expected);
        assert_eq!(result["operation_id"], command.operation_id);
        assert_eq!(result["joined_reopens"], 3);
        eprintln!(
            "browser live recovery {} {}: {}",
            self.name, f.client.signing_algorithm, result
        );
        self.reports.push(result.clone());
        result
    }
    pub(super) async fn completed(&mut self, command: &TransitionCommand, record: &str) {
        let reference = json!({"operationId":command.operation_id,"command":command.intent.required_scope(),"scope":{"conversationId":command.conversation_id,"sessionId":command.session_id,"connectionId":command.connection_id}});
        let reply = self
            .exchange(json!({"mode":"observe","reference":reference}))
            .await;
        assert_eq!(reply["passed"], true);
        assert_eq!(reply["record_id"], record);
        assert_eq!(reply["completion_observed"], true);
        self.reports.push(reply);
    }
    pub(super) async fn close(mut self, f: &Fixture) {
        let reply = self.exchange(json!({"mode":"close"})).await;
        assert_eq!(reply["closed"], true);
        self.input.shutdown().await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(10), self.child.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
        drop(self.browser.take());
        let directory =
            PathBuf::from(std::env::var_os("CONNECT_GATE6_CANONICAL_BROWSER_ARTIFACTS").unwrap());
        std::fs::write(directory.join(format!("browser-live-{}-{}.json",self.name,f.client.signing_algorithm)),serde_json::to_vec(&json!({"passed":true,"browser":self.name,"algorithm":f.client.signing_algorithm,"phases":self.reports,"node_joined":true,"browser_joined":true,"control_device":"live-control-device","media_device_unchanged":f.client.device_id})).unwrap()).unwrap();
    }
}
