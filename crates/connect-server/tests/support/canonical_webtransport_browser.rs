//! Explicit isolated installed-Chrome qualification; no personal browser state.
use super::*;
use sha2::{Digest, Sha256};
use std::{
    net::SocketAddr,
    process::{Child, Command, Stdio},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

struct Page {
    address: SocketAddr,
    stop: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Page {
    fn drop(&mut self) {
        self.stop.cancel();
        self.task.abort();
    }
}
async fn page(sdk: bool, media: bool) -> Page {
    let mut sdk_files = if sdk {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sdk/typescript/dist");
        [
            "browser_client.js",
            "browser_media.js",
            "browser_codecs.js",
            "browser_rtp.js",
            "uctp_json.js",
            "uctp_signing.js",
            "uctp_wire.js",
            "errors.js",
        ]
        .into_iter()
        .map(|name| {
            let body = std::fs::read_to_string(root.join(name))
                .expect("Build the production TypeScript SDK before its explicit browser lane");
            assert!(body.len() <= 1048576);
            (format!("/sdk/{name}"), body)
        })
        .collect::<std::collections::HashMap<_, _>>()
    } else {
        std::collections::HashMap::new()
    };
    if media {
        let body = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/gate6-webtransport-browser/sdk-media.mjs"),
        )
        .unwrap();
        assert!(body.len() <= 1048576);
        sdk_files.insert("/sdk-media.mjs".into(), body);
    }
    let sdk_files = Arc::new(sdk_files);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let stop = CancellationToken::new();
    let cancelled = stop.clone();
    let task = tokio::spawn(async move {
        let mut children = tokio::task::JoinSet::new();
        loop {
            let result =
                tokio::select! {biased;_=cancelled.cancelled()=>break,peer=listener.accept()=>peer};
            let Ok((mut socket, _)) = result else { break };
            while children.try_join_next().is_some() {}
            if children.len() >= 8 {
                continue;
            }
            let sdk_files = sdk_files.clone();
            children.spawn(async move {
                let mut bytes=[0u8;8192];let Ok(Ok(size))=tokio::time::timeout(Duration::from_secs(2),socket.read(&mut bytes)).await else {return};
                let request=String::from_utf8_lossy(&bytes[..size]);let path=request.split_whitespace().nth(1).unwrap_or("");
                let (status,mime,body)=match path {
                    "/" if media =>("200 OK","text/html","<!doctype html><meta charset=utf-8><script type=module>import {run} from '/sdk-media.mjs';globalThis.runBrowserFixture=run;</script>"),
                    "/" if sdk =>("200 OK","text/html","<!doctype html><meta charset=utf-8><script type=module>import {run} from '/sdk-peer.mjs';globalThis.runBrowserFixture=run;</script>"),
                    "/"=>("200 OK","text/html","<!doctype html><meta charset=utf-8><script type=module>import {run} from '/peer.mjs';globalThis.runBrowserFixture=run;</script>"),
                    "/peer.mjs"=>("200 OK","text/javascript",include_str!("../fixtures/gate6-webtransport-browser/peer.mjs")),
                    "/sdk-peer.mjs" if sdk =>("200 OK","text/javascript",include_str!("../fixtures/gate6-webtransport-browser/sdk-peer.mjs")),
                    path if sdk_files.contains_key(path)=>("200 OK","text/javascript",sdk_files[path].as_str()),
                    _=>("404 Not Found","text/plain","fixture route unavailable"),
                };
                let header=format!("HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",body.len());
                let _=tokio::time::timeout(Duration::from_secs(2),async {socket.write_all(header.as_bytes()).await?;socket.write_all(body.as_bytes()).await?;socket.shutdown().await}).await;
            });
        }
        children.abort_all();
        while children.join_next().await.is_some() {}
    });
    Page {
        address,
        stop,
        task,
    }
}
struct OwnedChrome(Child);
impl Drop for OwnedChrome {
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
fn launch(profile: &Path, log: &Path) -> OwnedChrome {
    let binary = std::env::var_os("CONNECT_GATE6_CHROME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
        });
    assert!(
        binary.is_file(),
        "Explicit browser lane requires an installed Chrome executable"
    );
    let mut command = Command::new(binary);
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
    command
        .arg(format!(
            "--log-net-log={}",
            log.with_extension("netlog.json").display()
        ))
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("about:blank")
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(log).unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    OwnedChrome(command.spawn().unwrap())
}
async fn debug_port(chrome: &mut OwnedChrome, profile: &Path) -> u16 {
    let until = tokio::time::Instant::now() + Duration::from_secs(12);
    loop {
        if let Ok(text) = std::fs::read_to_string(profile.join("DevToolsActivePort")) {
            if let Some(port) = text
                .lines()
                .next()
                .and_then(|s| s.parse::<u16>().ok())
                .filter(|p| *p > 0)
            {
                return port;
            }
        }
        assert!(
            chrome.0.try_wait().unwrap().is_none(),
            "Owned Chrome exited before CDP became ready"
        );
        assert!(
            tokio::time::Instant::now() < until,
            "Owned Chrome debugger startup deadline"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
fn actor(f: &Fixture, c: &CanonicalClientConfig, hash: &[u8]) -> Value {
    json!({"url":f.server.webtransport_url,"certificate_sha256":URL_SAFE_NO_PAD.encode(hash),"private_pkcs8":URL_SAFE_NO_PAD.encode(std::fs::read(&c.signing_private_key).unwrap()),"keyid":c.signing_key_id,"algorithm":c.signing_algorithm,"identity_id":c.identity_id,"credential":c.credential,"device":{"id":c.device_id,"kind":c.device_kind,"platform":c.device_platform},"server_keyid":c.server_key_id,"server_algorithm":c.server_algorithm,"server_public_raw":URL_SAFE_NO_PAD.encode(std::fs::read(&c.server_public_key).unwrap()),"server_not_before":c.server_key_not_before.timestamp(),"server_expires_at":c.server_key_expires_at.timestamp()})
}
fn capture(f: &Fixture, algorithm: &str) {
    if let Some(destination) = std::env::var_os("CONNECT_GATE6_BROWSER_ARTIFACTS") {
        let directory = std::path::PathBuf::from(destination).join(algorithm);
        std::fs::create_dir_all(&directory).unwrap();
        for name in ["chrome-stderr.log", "chrome-stderr.netlog.json"] {
            let source = f.temp.path().join(name);
            if source.is_file() {
                std::fs::copy(source, directory.join(name)).unwrap();
            }
        }
    }
}
async fn browser(algorithm: &str, sdk: bool, media: bool) {
    if media && std::env::var_os("CONNECT_GATE6_PACKET_TRACE").is_some() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("connect_service::canonical_packet=trace,connect_service::canonical_native_streams=debug")
            .without_time()
            .with_ansi(false)
            .with_test_writer()
            .try_init();
    }
    let page = page(sdk, media).await;
    let Fixture {
        temp,
        mut config,
        server,
        mut client,
        peer,
        admin,
        key_revision,
    } = fixture(algorithm).await;
    // Exercise the derived contact for the actual bound address family.
    assert!(config
        .webtransport
        .as_ref()
        .unwrap()
        .advertised_url
        .is_none());
    server.shutdown().await.unwrap();
    // WebTransport's explicit hash trust requires a short-lived ECDSA leaf.
    // The separate enrolled server signing key/pin is unchanged.
    use chrono::Datelike;
    let today = chrono::Utc::now().date_naive();
    let before = today - chrono::Duration::days(1);
    let after = today + chrono::Duration::days(7);
    let mut params =
        rcgen::CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
    params.not_before =
        rcgen::date_time_ymd(before.year(), before.month() as u8, before.day() as u8);
    params.not_after = rcgen::date_time_ymd(after.year(), after.month() as u8, after.day() as u8);
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let cert = params.self_signed(&key).unwrap();
    let hash = Sha256::digest(cert.der());
    std::fs::write(&config.certificate, cert.der()).unwrap();
    std::fs::write(&config.private_key, key.serialize_der()).unwrap();
    config.webtransport.as_mut().unwrap().allowed_origins =
        vec![format!("http://{}", page.address)];
    let server = RunningCanonicalServer::start(config.clone()).await.unwrap();
    client.endpoint = server.address;
    let f = Fixture {
        temp,
        config,
        server,
        client,
        peer,
        admin,
        key_revision,
    };
    let discovery = connect_client::canonical::CanonicalHttpClient::open(canonical_http_config(&f))
        .await
        .unwrap()
        .transport_discovery()
        .await
        .unwrap();
    assert_eq!(
        Some(discovery.webtransport.unwrap().url),
        f.server.webtransport_url
    );
    assert!(f
        .server
        .webtransport_url
        .as_ref()
        .unwrap()
        .starts_with("https://127.0.0.1:"));
    let capabilities = if media {
        vec!["audio".into(), "video".into(), "data".into()]
    } else {
        vec!["audio".into()]
    };
    super::canonical_webtransport_media::endpoint_caps(&f, &f.client, 0, capabilities.clone());
    let other = super::canonical_webtransport_media::other_caps(&f, capabilities);
    let (cid, initiator, recipient) = super::canonical_webtransport_media::accepted_caps(
        &f,
        &other,
        if media { "mixed" } else { "voice" },
        if media {
            json!({"audio":["opus"],"video":["vp8"],"data":["reliable-ordered"]})
        } else {
            json!({"audio":["g.711-mu"]})
        },
    )
    .await;
    let profile = f.temp.path().join("owned-chrome-profile");
    std::fs::create_dir(&profile).unwrap();
    let mut chrome = launch(&profile, &f.temp.path().join("chrome-stderr.log"));
    let port = debug_port(&mut chrome, &profile).await;
    let config = json!({"debug_port":port,"page_url":format!("http://{}/",page.address),"run":{"media_cid":cid,"participants":[initiator,recipient],"actors":[actor(&f,&f.client,&hash),actor(&f,&other,&hash)]}});
    let path = f.temp.path().join("browser-private-config.json");
    std::fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
    let driver = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/gate6-webtransport-browser/run-browser.mjs");
    let node = std::env::var_os("CONNECT_GATE6_NODE").unwrap_or_else(|| "node".into());
    let output = tokio::time::timeout(
        Duration::from_secs(90),
        tokio::process::Command::new(node)
            .arg(driver)
            .arg(path)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("Owned browser script deadline")
    .unwrap();
    if !output.status.success() {
        drop(chrome);
        capture(&f, algorithm);
        if media {
            let snapshot = f.server.service.canonical_runtime_snapshot().unwrap();
            eprintln!(
                "SDK media server ingress accepted={} rejected={}",
                snapshot.media_datagrams_accepted, snapshot.media_datagrams_rejected
            );
        }
        panic!(
            "Browser fixture failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["result"]["passed"], true);
    if media {
        assert_eq!(result["result"]["production_sdk_media"], true);
        for field in [
            "decoded_forward",
            "decoded_reverse",
            "reliable_data_duplex",
            "owned_cleanup",
        ] {
            assert_eq!(result["result"][field], true);
        }
    } else if sdk {
        assert_eq!(result["result"]["production_sdk"], true);
        assert_eq!(result["result"]["operation_state"], "completed");
        assert_eq!(result["result"]["credential_refused"], true);
        assert_eq!(result["result"]["trust_refused"], true);
    } else {
        assert_eq!(result["result"]["decoded_forward"], true);
        assert_eq!(result["result"]["decoded_reverse"], true);
        assert!(result["result"]["verified_server_frames"].as_u64().unwrap() >= 8);
    }
    assert_eq!(result["result"]["algorithm"], algorithm);
    let created = result["result"]["created_cid"].as_str().unwrap();
    let mut native = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut input =
        connect_client::conversations::CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
    input.tenant_id = Some("tnt_default".into());
    assert_eq!(
        native
            .create_conversation("browser-create", input)
            .await
            .unwrap()
            .id,
        created
    );
    native.shutdown().await.unwrap();
    if !sdk {
        let (started, _) = f
            .server
            .store
            .session_started_template(&cid, "http3-media-session")
            .unwrap();
        assert_eq!(started.active_connections.len(), 2);
        assert!(started
            .active_connections
            .iter()
            .all(|c| c.transport == "webtransport"));
    }
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    if sdk && !media {
        assert_eq!(
            db.query_row("SELECT count(*) FROM connections", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM operations WHERE owner_id=?1 AND operation_id='browser-create'",
            [&f.client.identity_id],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(db.query_row("SELECT count(*) FROM operations o JOIN connections c ON c.id=o.connection_id JOIN endpoints e ON e.id=c.endpoint_id WHERE o.command_type='connection.offer' AND o.operation_id IN ('browser-offer-0','browser-offer-1') AND o.state='completed' AND e.transport='webtransport'",[],|r|r.get::<_,u64>(0)).unwrap(),if sdk && !media { 0 } else { 2 });
    if media {
        assert_eq!(db.query_row("SELECT count(*) FROM operations WHERE command_type='message.send' AND operation_id LIKE 'browser-data-%' AND state='completed'", [], |r|r.get::<_,u64>(0)).unwrap(),6);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let retired = db.query_row("SELECT count(*) FROM connections WHERE id IN ('browser-connection-0','browser-connection-1') AND state='retired' AND observed_runtime_id IS NULL", [], |r|r.get::<_,u64>(0)).unwrap();
                let lifetimes = db.query_row("SELECT count(*) FROM native_connection_lifetimes WHERE connection_id IN ('browser-connection-0','browser-connection-1') AND retired=1", [], |r|r.get::<_,u64>(0)).unwrap();
                if retired == 2 && lifetimes == 2 { break; }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }).await.expect("Owned media must durably retire both original Connections");
    }
    drop(db);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let s = f.server.service.canonical_runtime_snapshot().unwrap();
            if s.physical_peers == 0 && s.connections == 0 && s.media_readers == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    println!("browser_evidence {}", result);
    drop(chrome);
    capture(&f, algorithm);
    drop(page);
    f.server.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Explicit isolated installed-Chrome profile; run the bounded WebTransport browser rehearsal"]
async fn installed_chrome_signed_control_reconnect_and_independently_decoded_duplex_eddsa() {
    browser("EdDSA", false, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Explicit isolated installed-Chrome profile; run the bounded WebTransport browser rehearsal"]
async fn installed_chrome_signed_control_reconnect_and_independently_decoded_duplex_es256() {
    browser("ES256", false, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Explicit production TypeScript SDK against isolated installed Chrome"]
async fn installed_chrome_production_sdk_signed_control_receipts_and_refusals_eddsa() {
    browser("EdDSA", true, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Explicit production TypeScript SDK against isolated installed Chrome"]
async fn installed_chrome_production_sdk_signed_control_receipts_and_refusals_es256() {
    browser("ES256", true, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Explicit production SDK Opus/VP8/data on actual WebTransport in owned Chrome"]
async fn installed_chrome_production_sdk_negotiated_duplex_opus_vp8_and_reliable_data_eddsa() {
    browser("EdDSA", true, true).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Explicit production SDK Opus/VP8/data on actual WebTransport in owned Chrome"]
async fn installed_chrome_production_sdk_negotiated_duplex_opus_vp8_and_reliable_data_es256() {
    browser("ES256", true, true).await;
}
