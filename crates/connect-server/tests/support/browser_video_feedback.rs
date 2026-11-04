use super::*;
use std::{collections::HashMap, sync::Arc};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;
struct Page {
    address: std::net::SocketAddr,
    stop: CancellationToken,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl Drop for Page {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
impl Page {
    async fn open() -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut files = HashMap::new();
        let sdk = std::env::var_os("CONNECT_GATE6_TS_DIST")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| root.join("sdk/typescript/dist"));
        for item in std::fs::read_dir(sdk).unwrap() {
            let item = item.unwrap();
            if item.path().extension().and_then(|e| e.to_str()) != Some("js") {
                continue;
            }
            assert!(item.file_type().unwrap().is_file());
            let name = item.file_name().into_string().unwrap();
            let bytes = std::fs::read(item.path()).unwrap();
            assert!(bytes.len() <= 1048576 && files.len() < 256);
            files.insert(format!("/sdk/{name}"), ("text/javascript", bytes));
        }
        files.insert("/recovery.mjs".into(), ("text/javascript",std::fs::read(root.join("crates/connect-server/tests/fixtures/gate6-webtransport-browser/video-feedback.mjs")).unwrap()));
        files.insert("/".into(), ("text/html",b"<!doctype html><meta charset=utf-8><title>Owned video feedback</title><script type=module>import {runRecovery} from '/recovery.mjs';globalThis.runRecovery=runRecovery;</script>".to_vec()));
        let files = Arc::new(files);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let stop = CancellationToken::new();
        let stopped = stop.clone();
        let task = tokio::spawn(async move {
            let mut children = tokio::task::JoinSet::new();
            loop {
                let accepted = tokio::select! {biased;_=stopped.cancelled()=>break,result=listener.accept()=>result};
                let Ok((mut socket, _)) = accepted else { break };
                while children.try_join_next().is_some() {}
                if children.len() >= 32 {
                    continue;
                }
                let files = files.clone();
                children.spawn(async move {
                    let _=tokio::time::timeout(Duration::from_secs(5),async {
                        let mut request=[0;4096];let n=socket.read(&mut request).await?;
                        let path=std::str::from_utf8(&request[..n]).ok().and_then(|r|r.lines().next()).and_then(|r|r.strip_prefix("GET ")).and_then(|r|r.split_whitespace().next());
                        let (status,mime,body)=path.and_then(|p|files.get(p)).map(|(mime,body)|("200 OK",*mime,body.as_slice())).unwrap_or(("404 Not Found","text/plain",b"not found"));
                        socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",body.len()).as_bytes()).await?;
                        socket.write_all(body).await?;socket.shutdown().await
                    }).await;
                });
            }
            children.abort_all();
            while children.join_next().await.is_some() {}
        });
        Self {
            address,
            stop,
            task: Some(task),
        }
    }
    async fn close(mut self) {
        self.stop.cancel();
        self.task.take().unwrap().await.unwrap();
    }
}
fn actor(f: &Fixture, c: &CanonicalClientConfig, hash: &[u8]) -> Value {
    json!({"url":f.server.webtransport_url,"certificate_sha256":URL_SAFE_NO_PAD.encode(hash),"private_pkcs8":URL_SAFE_NO_PAD.encode(std::fs::read(&c.signing_private_key).unwrap()),"keyid":c.signing_key_id,"algorithm":c.signing_algorithm,"identity_id":c.identity_id,"credential":c.credential,"device":{"id":c.device_id,"kind":c.device_kind,"platform":c.device_platform},"server_keyid":c.server_key_id,"server_algorithm":c.server_algorithm,"server_public_raw":URL_SAFE_NO_PAD.encode(std::fs::read(&c.server_public_key).unwrap()),"server_not_before":c.server_key_not_before.timestamp(),"server_expires_at":c.server_key_expires_at.timestamp()})
}
#[tokio::test]
async fn actual_chromium_signed_webtransport_reselection_feedback_fresh_vp8() {
    tokio::time::timeout(Duration::from_secs(90), exercise(false))
        .await
        .unwrap();
}
#[tokio::test]
async fn actual_chromium_signed_webtransport_feedback_writer_uncertainty() {
    tokio::time::timeout(Duration::from_secs(90), exercise(true))
        .await
        .unwrap();
}
async fn exercise(delayed_feedback: bool) {
    let page = Page::open().await;
    let Fixture {
        temp,
        mut config,
        server,
        mut client,
        peer,
        admin,
        key_revision,
    } = Fixture::with_native_end_scope("EdDSA", 12).await;
    server.shutdown().await.unwrap();
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
    config.webtransport = Some(
        connect_server::canonical_webtransport::CanonicalWebTransportConfig {
            mount_path: "/connect".into(),
            allowed_origins: vec![format!("http://{}", page.address)],
            advertised_url: None,
        },
    );
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
    media_fixture::endpoint_caps(&f, &f.client, 0, vec!["video".into()]);
    let other = media_fixture::other_caps_with_end(&f, vec!["video".into()], true);
    let (cid, source_participant, _) =
        media_fixture::accepted_caps(&f, &other, "video", json!({"video":["vp8"]})).await;
    let output = std::env::var_os("CONNECT_GATE6_BROWSER_FEEDBACK_ARTIFACTS")
        .map(std::path::PathBuf::from)
        .expect("explicit private output required");
    assert!(output.components().any(|c| c.as_os_str() == ".local"));
    std::fs::create_dir(&output).unwrap();
    let port = std::env::var("CONNECT_GATE6_BROWSER_DEBUG_PORT")
        .unwrap()
        .parse::<u16>()
        .unwrap();
    assert!(port > 1024);
    let input = json!({"debug_port":port,"page_url":format!("http://{}/",page.address),"run":{"delayed_feedback":delayed_feedback,"cid":cid,"sid":"http3-media-session","source_participant":source_participant,"actors":[actor(&f,&f.client,&hash),actor(&f,&other,&hash)]}});
    let path = output.join("private-input.json");
    init::private_write(&path, &serde_json::to_vec(&input).unwrap()).unwrap();
    let driver = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/gate6-webtransport-browser/run-video-feedback.mjs");
    let process = tokio::process::Command::new("node")
        .arg(driver)
        .arg(&path)
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    init::private_write(&output.join("driver.stdout"), &process.stdout).unwrap();
    init::private_write(&output.join("driver.stderr"), &process.stderr).unwrap();
    assert!(
        process.status.success(),
        "browser recovery driver failed; inspect private driver.stderr"
    );
    let result: Value = serde_json::from_slice(&process.stdout).unwrap();
    assert_eq!(result["passed"], true);
    assert_eq!(result["same_ssrc"], true);
    assert_eq!(result["new_epoch"], true);
    if delayed_feedback {
        assert_eq!(result["delayed_feedback"], true);
        assert_eq!(result["write_attempts"], 1);
        assert_eq!(result["forwarded_feedback"], 1);
        assert_eq!(result["shutdown_waited"], true);
        assert_eq!(
            result["timeout_message"],
            "Video feedback writer exceeded original age"
        );
    } else {
        assert_eq!(result["fresh_frame"], true);
    }
    assert_eq!(result["shutdown_joined"], true);
    assert!(result["feedback_packets"].as_u64().unwrap() > 0);
    f.server.shutdown().await.unwrap();
    page.close().await;
    if delayed_feedback {
        eprintln!("actual local Chromium: signed production WebTransport, one native feedback submission, original writer timeout, no retry, shutdown waits for delayed completion and joins both peers; not a full matrix cell");
    } else {
        eprintln!("actual local Chromium: signed production WebTransport, same-SSRC epoch change, receiver PLI, fresh original VP8 frame and joined browser peers; not a full matrix cell");
    }
}
