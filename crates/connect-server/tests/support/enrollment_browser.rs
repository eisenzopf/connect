//! Explicit owned Chrome, actual HTTPS/IndexedDB/WebCrypto and signed WT/WSS.
use super::*;
use base64::engine::general_purpose::STANDARD;
use rcgen::PublicKeyData;
use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    process::{Child, Command, Stdio},
    sync::Arc,
};
use tokio_util::sync::CancellationToken;

pub(super) struct Page {
    pub(super) stop: CancellationToken,
    pub(super) task: tokio::task::JoinHandle<()>,
}
#[derive(Clone)]
struct Backend {
    origin: String,
    admin_origin: String,
    admin: String,
    identity: String,
    peer_identity: String,
    http: reqwest::Client,
    files: Arc<HashMap<String, Vec<u8>>>,
    typed_endpoints: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
}
pub(super) async fn page(listener: tokio::net::TcpListener, h: &Hosted, origin: String) -> Page {
    let mut files = HashMap::new();
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sdk/typescript/dist");
    for entry in std::fs::read_dir(sdk).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().to_str().unwrap().to_owned();
        if name.ends_with(".js") {
            let bytes = std::fs::read(entry.path()).unwrap();
            assert!(bytes.len() < 1_048_576);
            files.insert(format!("/sdk/{name}"), bytes);
        }
    }
    files.insert("/".into(),b"<!doctype html><meta charset=utf-8><title>Isolated enrollment</title><script type=module src=/browser-enrollment.mjs></script>".to_vec());
    files.insert(
        "/browser-enrollment.mjs".into(),
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/gate6-enrollment-browser/client.mjs"),
        )
        .unwrap(),
    );
    files.insert("/profile-fallback".into(), b"<!doctype html><meta charset=utf-8><title>Isolated profile fallback</title><script type=module src=/browser-profile-fallback.mjs></script>".to_vec());
    files.insert(
        "/browser-profile-fallback.mjs".into(),
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/gate6-enrollment-browser/profile-fallback.mjs"),
        )
        .unwrap(),
    );
    files.insert("/typed-browser".into(), b"<!doctype html><meta charset=utf-8><title>Owned typed browser workflow</title><script type=module src=/typed-browser.mjs></script>".to_vec());
    files.insert(
        "/typed-browser.mjs".into(),
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/gate6-enrollment-browser/typed-browser.mjs"),
        )
        .unwrap(),
    );
    let backend = Backend {
        origin,
        admin_origin: h.url(HttpSurface::Administration, ""),
        admin: h.f.admin.clone(),
        identity: h.f.client.identity_id.clone(),
        peer_identity: h.f.peer.identity_id.clone(),
        http: h.http.clone(),
        files: Arc::new(files),
        typed_endpoints: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
    };
    let cert = CertificateDer::from_pem_file(&h.f.config.certificate).unwrap();
    let key = PrivateKeyDer::from_pem_file(&h.f.config.private_key).unwrap();
    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .unwrap();
    tls.alpn_protocols = vec![b"http/1.1".to_vec()];
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
    let stop = CancellationToken::new();
    let cancelled = stop.clone();
    let task = tokio::spawn(async move {
        let mut children = tokio::task::JoinSet::new();
        loop {
            tokio::select! {biased;
                _=cancelled.cancelled()=>break,
                Some(_)=children.join_next()=>{},
                accepted=listener.accept()=>{
                    let (socket,_)=accepted.unwrap();let acceptor=acceptor.clone();let backend=backend.clone();
                    children.spawn(async move {let _=tokio::time::timeout(Duration::from_secs(10),async move {
                        let Ok(mut stream)=acceptor.accept(socket).await else{return;};
                        let mut input=Vec::new();let mut part=[0;4096];let (headers,body)=loop {
                            let Ok(n)=stream.read(&mut part).await else{return;};if n==0{return;}input.extend_from_slice(&part[..n]);
                            if input.len()>73728{return;}
                            if let Some(end)=input.windows(4).position(|v|v==b"\r\n\r\n") {
                                if end>8192{return;}let text=String::from_utf8(input[..end].to_vec()).unwrap();
                                let length=text.lines().find_map(|line|line.split_once(':').filter(|(k,_)|k.eq_ignore_ascii_case("content-length")).map(|(_,v)|v.trim().parse::<usize>().unwrap())).unwrap_or(0);
                                if length>65536{return;}if input.len()>=end+4+length {break (text,input[end+4..end+4+length].to_vec());}
                            }
                        };
                        let first=headers.lines().next().unwrap();let mut words=first.split_whitespace();let method=words.next().unwrap();let path=words.next().unwrap();
                        let (status,mime,body)=if method=="GET" {match backend.files.get(path){Some(bytes)=>(200,"text/javascript",bytes.clone()),None=>(404,"text/plain",b"missing".to_vec())}}
                        else if method=="POST" && headers.lines().any(|line|line.split_once(':').is_some_and(|(k,v)|k.eq_ignore_ascii_case("origin") && v.trim()==backend.origin)) {
                            let target=match path {
                                "/approve"=>{
                                    let mut request:EnrollmentRequest=serde_json::from_slice(&body).unwrap();request.normalize().unwrap();assert_eq!(request.identity_id,backend.identity);assert!(request.transport!=EnrollmentTransport::Quic);assert_eq!(request.operations,vec!["operation.get"]);
                                    Some((format!("{}/v2/enrollments",backend.admin_origin),body))
                                },
                                "/typed-approve"=>{
                                    let mut request:EnrollmentRequest=serde_json::from_slice(&body).unwrap();request.normalize().unwrap();
                                    assert!([&backend.identity,&backend.peer_identity].contains(&&request.identity_id));
                                    assert!(request.transport!=EnrollmentTransport::Quic);
                                    assert_eq!(request.operations,vec!["connection.end","connection.offer","connection.update","conversation.get","create","operation.get","session.accept","session.cancel","session.end","session.invite","session.reject","session_get","stream.subscribe","stream.unsubscribe"]);
                                    let mut recipients=vec![backend.identity.clone(),backend.peer_identity.clone()];recipients.sort();
                                    assert_eq!(request.recipients,recipients);
                                    Some((format!("{}/v2/enrollments",backend.admin_origin),body))
                                },
                                "/typed-verify"=>{
                                    let value:Value=serde_json::from_slice(&body).unwrap();let id=value["endpoint_id"].as_str().unwrap();
                                    connect_protocol::transitions::id(id).unwrap();
                                    assert!(backend.typed_endpoints.lock().unwrap().contains(id));
                                    let command=ProvisioningCommand::VerifyEndpoint{operation_id:value["operation_id"].as_str().unwrap().into(),endpoint_id:id.into(),expected_revision:0,evidence_ref:"owned-loopback-synthetic-device-proof".into()};
                                    Some((format!("{}/v2/endpoints/{id}/verify",backend.admin_origin),serde_json::to_vec(&command).unwrap()))
                                },
                                "/revoke"=>{
                                    let value:Value=serde_json::from_slice(&body).unwrap();let id=value["enrollment_id"].as_str().unwrap();connect_protocol::transitions::id(id).unwrap();
                                    Some((format!("{}/v2/enrollments/{id}/revoke",backend.admin_origin),serde_json::to_vec(&EnrollmentRevokeRequest{operation_id:value["operation_id"].as_str().unwrap().into()}).unwrap()))
                                }, _=>None,
                            };
                            if let Some((url,body))=target {
                                let response=backend.http.post(url).bearer_auth(&backend.admin).header("Content-Type","application/json").body(body).send().await.unwrap();let status=response.status().as_u16();let bytes=response.bytes().await.unwrap();assert!(bytes.len()<=65536);
                                if path=="/typed-approve" && status==200 {
                                    let value:Value=serde_json::from_slice(&bytes).unwrap();let id=value["result"]["Ok"]["endpoint_id"].as_str().unwrap();
                                    backend.typed_endpoints.lock().unwrap().insert(id.into());
                                }
                                (status,"application/json",bytes.to_vec())
                            }else{(404,"text/plain",b"missing".to_vec())}
                        }else{(403,"text/plain",b"refused".to_vec())};
                        // HTML MIME is needed for the isolated page; modules remain JS.
                        let mime=if path=="/" || path=="/profile-fallback" || path=="/typed-browser" {"text/html"}else{mime};
                        let header=format!("HTTP/1.1 {status} Fixture\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",body.len());
                        let _=stream.write_all(header.as_bytes()).await;let _=stream.write_all(&body).await;let _=stream.shutdown().await;
                    }).await;});
                }
            }
        }
        children.abort_all();
        while children.join_next().await.is_some() {}
    });
    Page { stop, task }
}
pub(super) struct Chrome(Child);
impl Drop for Chrome {
    fn drop(&mut self) {
        let id = self.0.id();
        let _ = self.0.kill();
        #[cfg(unix)]
        {
            let _ = Command::new("/bin/kill")
                .args(["-KILL", "--", &format!("-{id}")])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = self.0.wait();
    }
}
pub(super) fn launch(profile: &Path, log: &Path, spki: &str) -> Chrome {
    launch_capture(profile, log, spki, false)
}
pub(super) fn launch_capture(profile: &Path, log: &Path, spki: &str, synthetic: bool) -> Chrome {
    let binary = std::env::var_os("CONNECT_GATE6_CHROME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into());
    assert!(binary.is_file(), "explicit installed Chrome required");
    let mut command = Command::new(binary);
    command
        .args([
            "--headless=new",
            "--remote-debugging-address=127.0.0.1",
            "--remote-debugging-port=0",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-background-networking",
            "--disable-sync",
            "--disable-breakpad",
            "--disable-crash-reporter",
        ])
        .arg(format!("--ignore-certificate-errors-spki-list={spki}"))
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("about:blank")
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(log).unwrap());
    if synthetic {
        command.arg("--use-fake-device-for-media-stream");
        command.arg("--auto-select-tab-capture-source-by-title=Connect Owned Screen");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    Chrome(command.spawn().unwrap())
}
pub(super) async fn debug_port(chrome: &mut Chrome, profile: &Path) -> u16 {
    let until = tokio::time::Instant::now() + Duration::from_secs(12);
    loop {
        if let Ok(text) = std::fs::read_to_string(profile.join("DevToolsActivePort")) {
            if let Some(port) = text.lines().next().and_then(|v| v.parse().ok()) {
                return port;
            }
        }
        assert!(chrome.0.try_wait().unwrap().is_none());
        assert!(
            tokio::time::Instant::now() < until,
            "owned Chrome startup bound"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
pub(super) async fn run(h: &Hosted, port: u16, page_url: &str, run: Value, phase: &str) -> Value {
    let input = h.f.temp.path().join(format!("browser-{phase}.json"));
    std::fs::write(
        &input,
        serde_json::to_vec(&json!({"debug_port":port,"page_url":page_url,"run":run,"user_gesture":phase=="typed-browser","typed_media_ui":phase=="typed-browser"})).unwrap(),
    )
    .unwrap();
    let driver = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/gate6-webtransport-browser/run-browser.mjs");
    let output = tokio::time::timeout(
        Duration::from_secs(90),
        tokio::process::Command::new(
            std::env::var_os("CONNECT_GATE6_NODE").unwrap_or_else(|| "node".into()),
        )
        .arg(driver)
        .arg(input)
        .kill_on_drop(true)
        .output(),
    )
    .await
    .expect("owned browser script bound")
    .unwrap();
    if let Some(dir) = std::env::var_os("CONNECT_GATE6_ENROLLMENT_ARTIFACTS") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{phase}.json")), &output.stdout).unwrap();
        std::fs::write(dir.join(format!("{phase}.stderr")), &output.stderr).unwrap();
    }
    assert!(
        output.status.success(),
        "owned enrollment browser {phase} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Explicit isolated installed Chrome; production SDK must be built"]
async fn installed_chrome_public_browser_enrollment_restores_and_rotates_all_profiles() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let foreign = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("https://{}", listener.local_addr().unwrap());
    let other = format!("https://{}", foreign.local_addr().unwrap());
    let mut h = Hosted::with_origins(&[origin.clone(), other.clone()]).await;
    let before = counts(&h);
    let ui = page(listener, &h, origin.clone()).await;
    let other_ui = page(foreign, &h, other.clone()).await;
    let key = rcgen::KeyPair::from_pem(&std::fs::read_to_string(&h.f.config.private_key).unwrap())
        .unwrap();
    let spki = STANDARD.encode(Sha256::digest(key.subject_public_key_info()));
    let leaf = CertificateDer::from_pem_file(&h.f.config.certificate).unwrap();
    let hash = URL_SAFE_NO_PAD.encode(Sha256::digest(leaf.as_ref()));
    let profile = h.f.temp.path().join("enrollment-owned-chrome");
    std::fs::create_dir(&profile).unwrap();
    let mut chrome = launch(&profile, &h.f.temp.path().join("chrome.log"), &spki);
    let port = debug_port(&mut chrome, &profile).await;
    let c = &h.f.client;
    let mut config = json!({"actors":[],"database":"connect-enrollment-fixture","suffix":uuid::Uuid::new_v4().simple().to_string(),"phase":"prepare","identity_id":c.identity_id,"recipient_id":h.f.peer.identity_id,
        "api_origin":h.url(HttpSurface::Application,""),"wt_url":h.f.server.webtransport_url,"wss_url":h.f.server.websocket_url,"certificate_sha256":hash,
        "server_keyid":c.server_key_id,"server_algorithm":c.server_algorithm,"server_public_raw":URL_SAFE_NO_PAD.encode(std::fs::read(&c.server_public_key).unwrap()),"server_not_before":c.server_key_not_before.timestamp(),"server_expires_at":c.server_key_expires_at.timestamp()});
    let first = run(&h, port, &format!("{origin}/"), config.clone(), "prepare").await;
    assert_eq!(first["result"]["entries"].as_array().unwrap().len(), 4);
    assert_eq!(
        counts(&h),
        (before.0 + 4, before.1 + 4, before.2 + 4, before.3 + 4)
    );
    config["entries"] = first["result"]["entries"].clone();
    config["expired"] = first["result"]["expired"].clone();
    config["phase"] = json!("foreign");
    let refused = run(&h, port, &format!("{other}/"), config.clone(), "foreign").await;
    assert_eq!(refused["result"]["foreign_origin_refusals"], 4);
    assert_eq!(
        counts(&h),
        (before.0 + 4, before.1 + 4, before.2 + 4, before.3 + 4)
    );
    // Preserve actual contacts across a full server restart. The next Chrome
    // target has only IndexedDB original state, never an exported private key.
    h.f.config.bind = h.f.server.address;
    h.f.config.http_bind = h.f.server.http_address;
    h.f.config.websocket.as_mut().unwrap().bind = h.f.server.websocket_address.unwrap();
    for listener in &mut h.f.config.https {
        listener.bind =
            h.f.server
                .https_endpoints
                .iter()
                .find(|e| e.surface == listener.surface)
                .unwrap()
                .address;
    }
    let challenge_expiry = first["result"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["challengeExpiry"].as_i64().unwrap())
        .max()
        .unwrap();
    let remaining = (challenge_expiry - chrono::Utc::now().timestamp() + 1).max(0);
    assert!(remaining <= 12);
    tokio::time::sleep(Duration::from_secs(remaining as u64)).await;
    h.f.server.shutdown().await.unwrap();
    h.f.server = RunningCanonicalServer::start(h.f.config.clone())
        .await
        .unwrap();
    assert_eq!(config["api_origin"], h.url(HttpSurface::Application, ""));
    config["phase"] = json!("resume");
    let recovered = run(&h, port, &format!("{origin}/"), config, "resume").await;
    assert_eq!(recovered["result"]["recovered"], 4);
    assert_eq!(recovered["result"]["rotated"], 4);
    assert_eq!(recovered["result"]["revoked"], 8);
    assert_eq!(
        counts(&h),
        (before.0 + 8, before.1 + 4, before.2 + 4, before.3 + 8)
    );
    drop(chrome);
    ui.stop.cancel();
    other_ui.stop.cancel();
    ui.task.await.unwrap();
    other_ui.task.await.unwrap();
    h.f.server.shutdown().await.unwrap();
}
