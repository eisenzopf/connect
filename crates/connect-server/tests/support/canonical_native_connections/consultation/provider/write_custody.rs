//! Actual unfinished provider socket writes. The test constrains only its exact
//! designated loopback connection before the WebSocket upgrade completes. No
//! production send-buffer policy, context-size bound or authority hook changes.
use super::*;
use futures_util::{SinkExt, StreamExt};
use nix::sys::socket::{getpeername, getsockname, getsockopt, setsockopt, sockopt, SockaddrIn};
use std::net::SocketAddr;
use std::os::fd::AsRawFd;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;
use tokio_tungstenite::{
    tungstenite::{handshake::derive_accept_key, protocol::Role},
    WebSocketStream,
};

// The provider is the receiving TCP server. On Darwin, disabling options only
// on its listener is too late for the initiating peer's scaling negotiation.
// Run just this fixture in a child with a narrowly matched pre-connect option
// shim. It changes neither addresses nor application bytes, touches no host
// network settings and is absent from production binaries.
pub(super) async fn isolated_profile(test: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        if std::env::var_os("CONNECT_PROVIDER_SOCKET_PROFILE").is_some() {
            return false;
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".local/gate-6/provider-socket-profiles");
        std::fs::create_dir_all(&root).unwrap();
        let scratch = tempfile::tempdir_in(root).unwrap();
        let library = scratch.path().join("socket-profile.dylib");
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/support/provider_socket_profile.c");
        let status = tokio::process::Command::new("cc")
            .args([
                "-std=c11",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-dynamiclib",
                "-O2",
            ])
            .arg(source)
            .arg("-o")
            .arg(&library)
            .status()
            .await
            .unwrap();
        assert!(status.success(), "compile designated socket profile");
        let port = scratch.path().join("port");
        std::fs::write(&port, b"0\n").unwrap();
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
            .args([test, "--test-threads=1", "--nocapture"])
            .env("CONNECT_PROVIDER_SOCKET_PROFILE", &port)
            .env("DYLD_INSERT_LIBRARIES", &library)
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let status = tokio::time::timeout(Duration::from_secs(90), child.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(status.success(), "isolated physical socket case failed");
        return true;
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = test;
        false
    }
}

struct PendingFrame {
    length: usize,
    sender_buffer: usize,
    receiver_buffer: usize,
    queued_bytes: usize,
    in_flight_bound: usize,
    sender_fd: i32,
    receiver_fd: i32,
    client: SocketAddr,
    server: SocketAddr,
}
struct ReceivedFrame {
    length: usize,
    received: usize,
    complete: bool,
}
pub(super) struct Peer {
    pub(super) base: String,
    pending: Option<oneshot::Receiver<PendingFrame>>,
    received: Option<oneshot::Receiver<ReceivedFrame>>,
    read: Arc<tokio::sync::Notify>,
    stop: tokio_util::sync::CancellationToken,
    task: Option<tokio::task::JoinHandle<()>>,
}

// fcntl duplicates an open descriptor atomically; invalid or concurrently closed
// descriptors simply fail. Each duplicate is owned and immediately closed. The
// exact local AND peer tuple must match before any socket option is changed.
// No duplicate survives the handshake, so fixture instrumentation cannot keep
// a cancelled production socket alive or flush its buffered private bytes.
fn constrain_sender(local: SocketAddr, remote: SocketAddr) -> (usize, i32, usize) {
    struct Duplicate(i32);
    impl Drop for Duplicate {
        fn drop(&mut self) {
            let _ = nix::unistd::close(self.0);
        }
    }
    for fd in 0..4096 {
        let Ok(duplicate) = nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_DUPFD_CLOEXEC(0)) else {
            continue;
        };
        let socket = Duplicate(duplicate);
        let address = |value: nix::Result<SockaddrIn>| {
            value
                .ok()
                .map(|v| SocketAddr::V4(std::net::SocketAddrV4::from(v)))
        };
        if address(getsockname(socket.0)) == Some(local)
            && address(getpeername(socket.0)) == Some(remote)
        {
            setsockopt(socket.0, sockopt::SndBuf, &1024).unwrap();
            let actual = getsockopt(socket.0, sockopt::SndBuf).unwrap();
            assert!(
                actual <= 8192,
                "fixture sender buffer was not constrained: {actual}"
            );
            let mss = getsockopt(socket.0, sockopt::TcpMaxSeg).unwrap() as usize;
            return (actual, fd, mss);
        }
    }
    panic!("designated live provider socket was absent from the bounded descriptor inventory");
}

// Socket high-water marks are not credits on Darwin: loopback can report a
// large allocation while a small advertised window blocks the sender. Inspect
// actual queue occupancy instead, without reading or owning the producer FD.
// Count BOTH send/receive queues (overlapping unacknowledged data is counted
// twice), then reserve one additional full negotiated segment plus headers.
async fn queued_bytes(sender: i32, receiver: i32, client: SocketAddr, server: SocketAddr) -> usize {
    #[cfg(target_os = "macos")]
    {
        let mut total = 0;
        // Read sender first, receiver second. Data transferred between these
        // observations is counted twice, never lost between two snapshots.
        for fd in [sender, receiver] {
            let output = tokio::process::Command::new("/usr/sbin/lsof")
                .args([
                    "-nP",
                    "-a",
                    "-p",
                    &std::process::id().to_string(),
                    "-d",
                    &fd.to_string(),
                    "-FfT",
                    "-Tqs",
                ])
                .output()
                .await
                .unwrap();
            assert!(
                output.status.success(),
                "fixture socket queue inspection failed"
            );
            let text = std::str::from_utf8(&output.stdout).unwrap();
            let mut fields = 0;
            for line in text.lines() {
                if let Some(value) = line
                    .strip_prefix("TQR=")
                    .or_else(|| line.strip_prefix("TQS="))
                {
                    total += value.parse::<usize>().unwrap();
                    fields += 1;
                }
            }
            assert_eq!(
                fields, 2,
                "designated live socket queue pair must be observed"
            );
        }
        let _ = (client, server);
        total
    }
    #[cfg(target_os = "linux")]
    {
        let encoded = |address: SocketAddr| {
            let SocketAddr::V4(a) = address else {
                panic!("IPv4 fixture")
            };
            format!(
                "{:08X}:{:04X}",
                u32::from_le_bytes(a.ip().octets()),
                a.port()
            )
        };
        let client = encoded(client);
        let server = encoded(server);
        let mut total = 0;
        for (local, remote) in [(&client, &server), (&server, &client)] {
            let data = std::fs::read_to_string("/proc/self/net/tcp").unwrap();
            let mut matches = 0;
            for line in data.lines().skip(1) {
                let fields = line.split_whitespace().collect::<Vec<_>>();
                if fields.len() > 4 && fields[1] == local && fields[2] == remote {
                    let (send, receive) = fields[4].split_once(':').unwrap();
                    total += usize::from_str_radix(send, 16).unwrap()
                        + usize::from_str_radix(receive, 16).unwrap();
                    matches += 1;
                }
            }
            assert_eq!(matches, 1);
        }
        let _ = (sender, receiver);
        total
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (sender, receiver, client, server);
        panic!("physical provider queue profile requires macOS or Linux");
    }
}
impl Peer {
    pub(super) async fn start(provider: Provider) -> Self {
        let socket = socket2::Socket::new(
            socket2::Domain::IPV4,
            socket2::Type::STREAM,
            Some(socket2::Protocol::TCP),
        )
        .unwrap();
        socket.set_recv_buffer_size(1024).unwrap();
        #[cfg(target_os = "linux")]
        socket.set_tcp_mss(1024).unwrap();
        socket.set_nonblocking(true).unwrap();
        socket
            .bind(&"127.0.0.1:0".parse::<SocketAddr>().unwrap().into())
            .unwrap();
        #[cfg(target_os = "macos")]
        {
            // Use safe ownership transfer through stdin, rather than unsafe
            // Rust or process-wide descriptor inheritance. The duplicate is
            // closed by the helper before listening or starting provider IO.
            let duplicate: std::os::fd::OwnedFd = socket.try_clone().unwrap().into();
            let child = tokio::process::Command::new("python3")
                .arg(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/support/provider_socket_options.py"),
                )
                .stdin(std::process::Stdio::from(duplicate))
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .unwrap();
            let output = tokio::time::timeout(Duration::from_secs(2), child.wait_with_output())
                .await
                .unwrap()
                .unwrap();
            assert!(
                output.status.success(),
                "designated listener options were not installed"
            );
            assert_eq!(
                serde_json::from_slice::<Value>(&output.stdout).unwrap(),
                json!({"no_options":true})
            );
        }
        socket.listen(8).unwrap();
        let listener = tokio::net::TcpListener::from_std(socket.into()).unwrap();
        let address = listener.local_addr().unwrap();
        #[cfg(target_os = "macos")]
        {
            let path = std::env::var_os("CONNECT_PROVIDER_SOCKET_PROFILE")
                .expect("isolated socket profile");
            std::fs::write(path, format!("{}\n", address.port())).unwrap();
        }
        let (pending_tx, pending_rx) = oneshot::channel();
        let (received_tx, received_rx) = oneshot::channel();
        let read = Arc::new(tokio::sync::Notify::new());
        let ready_read = read.clone();
        let stop = tokio_util::sync::CancellationToken::new();
        let stopped = stop.clone();
        let task = tokio::spawn(async move {
            let mut jobs = tokio::task::JoinSet::new();
            let mut first = Some((pending_tx, received_tx, ready_read));
            loop {
                tokio::select! {
                    _=stopped.cancelled()=>break,
                    done=jobs.join_next(),if !jobs.is_empty()=>{done.unwrap().unwrap();},
                    accepted=listener.accept()=>{
                        let (stream, client_address)=accepted.unwrap();
                        let provider=provider.clone();
                        if let Some((pending,received,read))=first.take() {
                            jobs.spawn(initial(stream,client_address,address,provider,pending,received,read));
                        } else {
                            jobs.spawn(async move { control(tokio_tungstenite::accept_async(stream).await.unwrap(),provider).await; });
                        }
                    }
                }
            }
            jobs.abort_all();
            while let Some(done) = jobs.join_next().await {
                if let Err(error) = done {
                    assert!(error.is_cancelled());
                }
            }
        });
        Self {
            base: format!("ws://{address}"),
            pending: Some(pending_rx),
            received: Some(received_rx),
            read,
            stop,
            task: Some(task),
        }
    }
    async fn pending(&mut self) -> PendingFrame {
        tokio::time::timeout(Duration::from_secs(5), self.pending.take().unwrap())
            .await
            .unwrap()
            .unwrap()
    }
    async fn receive(&mut self) -> ReceivedFrame {
        self.read.notify_one();
        tokio::time::timeout(Duration::from_secs(5), self.received.take().unwrap())
            .await
            .unwrap()
            .unwrap()
    }
    pub(super) async fn shutdown(mut self) {
        self.stop.cancel();
        self.task.take().unwrap().await.unwrap();
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

async fn control(mut ws: WebSocketStream<tokio::net::TcpStream>, provider: Provider) {
    while let Some(frame) = ws.next().await {
        match frame {
            Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                let value: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(
                    value["type"], "end-call",
                    "cleanup must not repeat context or media"
                );
                provider.ended.fetch_add(1, Ordering::SeqCst);
                ws.send(tokio_tungstenite::tungstenite::Message::Text(
                    json!({"type":"status-update","status":"ended"})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
                return;
            }
            Ok(tokio_tungstenite::tungstenite::Message::Close(_)) | Err(_) => return,
            _ => panic!("unexpected cleanup media or control"),
        }
    }
}
async fn initial(
    mut stream: tokio::net::TcpStream,
    client: SocketAddr,
    server: SocketAddr,
    provider: Provider,
    pending: oneshot::Sender<PendingFrame>,
    received: oneshot::Sender<ReceivedFrame>,
    read: Arc<tokio::sync::Notify>,
) {
    let accepted_receiver_buffer = socket2::SockRef::from(&stream).recv_buffer_size().unwrap();
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        assert!(header.len() < 8192);
        header.push(stream.read_u8().await.unwrap());
    }
    let header = std::str::from_utf8(&header).unwrap();
    #[cfg(target_os = "macos")]
    {
        let path = std::env::var("CONNECT_PROVIDER_SOCKET_PROFILE").unwrap();
        let value = std::fs::read_to_string(format!("{path}.applied"))
            .expect("initiating socket option must be observed before handshake");
        let values = value.split_whitespace().collect::<Vec<_>>();
        assert_eq!(values.len(), 2);
        assert_eq!(values[0].parse::<u16>().unwrap(), server.port());
        assert_ne!(values[1].parse::<i32>().unwrap(), 0);
    }
    assert!(header.starts_with("GET /private-fixture-call/transport HTTP/1.1\r\n"));
    assert!(header.lines().any(|line| line
        .split_once(':')
        .is_some_and(|(name, value)| name.eq_ignore_ascii_case("authorization")
            && value.trim() == "Bearer private-fixture-token")));
    let key = header
        .lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(name, _)| name.eq_ignore_ascii_case("sec-websocket-key"))
                .map(|(_, v)| v.trim())
        })
        .unwrap();
    let (sender_buffer, sender_fd, mss) = constrain_sender(client, server);
    let negotiated_receiver_buffer = socket2::SockRef::from(&stream).recv_buffer_size().unwrap();
    socket2::SockRef::from(&stream)
        .set_recv_buffer_size(1024)
        .unwrap();
    stream.write_all(format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",derive_accept_key(key.as_bytes())).as_bytes()).await.unwrap();
    let mut header = [0u8; 2];
    stream.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 0x81);
    assert_ne!(header[1] & 128, 0);
    let length = match header[1] & 127 {
        126 => stream.read_u16().await.unwrap() as usize,
        127 => stream.read_u64().await.unwrap() as usize,
        n => n as usize,
    };
    assert!(length > 64 * 1024 && length < 256 * 1024);
    let mut mask = [0u8; 4];
    stream.read_exact(&mut mask).await.unwrap();
    let mut bytes = vec![0u8; 16];
    stream.read_exact(&mut bytes).await.unwrap();
    let receiver = socket2::SockRef::from(&stream);
    let initial_receiver_buffer = receiver.recv_buffer_size().unwrap();
    receiver.set_recv_buffer_size(1024).unwrap();
    let receiver_buffer = receiver
        .recv_buffer_size()
        .unwrap()
        .max(initial_receiver_buffer)
        .max(accepted_receiver_buffer)
        .max(negotiated_receiver_buffer);
    let sender_buffer = sender_buffer.max(constrain_sender(client, server).0);
    let first = queued_bytes(sender_fd, stream.as_raw_fd(), client, server).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let queued_bytes = queued_bytes(sender_fd, stream.as_raw_fd(), client, server)
        .await
        .max(first);
    let in_flight_bound = mss + 128;
    assert!(length>bytes.len()+queued_bytes+in_flight_bound,"frame already admitted to kernel queues: length={length}, queued={queued_bytes}, segment_slack={in_flight_bound}, sender_buffer={sender_buffer}, receiver_buffer={receiver_buffer}");
    pending
        .send(PendingFrame {
            length,
            sender_buffer,
            receiver_buffer,
            queued_bytes,
            in_flight_bound,
            sender_fd,
            receiver_fd: stream.as_raw_fd(),
            client,
            server,
        })
        .unwrap_or_else(|_| panic!("pending receiver stopped"));
    read.notified().await;
    socket2::SockRef::from(&stream)
        .set_recv_buffer_size(256 * 1024)
        .unwrap();
    while bytes.len() < length {
        let mut chunk = [0u8; 4096];
        let bound = chunk.len().min(length - bytes.len());
        match stream.read(&mut chunk[..bound]).await {
            Ok(0) | Err(_) => break,
            Ok(count) => bytes.extend_from_slice(&chunk[..count]),
        }
    }
    let complete = bytes.len() == length;
    received
        .send(ReceivedFrame {
            length,
            received: bytes.len(),
            complete,
        })
        .unwrap_or_else(|_| panic!("frame receiver stopped"));
    if !complete {
        return;
    }
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte ^= mask[index % 4];
    }
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["type"], "add-message");
    provider.exports.fetch_add(1, Ordering::SeqCst);
    provider.body.send(body.clone()).unwrap();
    provider.release_ack.notified().await;
    let mut ws = WebSocketStream::from_raw_socket(stream, Role::Server, None).await;
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"type":"conversation-update","messagesOpenAIFormatted":[body["message"].clone()]})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    control(ws, provider).await;
}

fn scope(cid: &str) -> connect_store::records::RecordScope {
    connect_store::records::RecordScope {
        tenant_id: "tnt_default".into(),
        conversation_id: cid.into(),
        session_id: "supervised-record-source".into(),
    }
}
pub(super) async fn pad_record(f: &Fixture, cid: &str) {
    let original = f
        .server
        .store
        .list_records("tnt_default", cid, Some("supervised-record-source"))
        .unwrap()
        .pop()
        .unwrap();
    let padded = f
        .server
        .store
        .mutate_record(
            &scope(cid),
            "provider-write-size-fixture",
            connect_store::records::RecordMutationKind::Redact,
            "provider-write-size-v1",
            &original,
            |bytes| {
                let mut body: Value = serde_json::from_slice(&bytes.body).unwrap();
                body["analysis"][0]["body"]["transport_fixture_padding"] =
                // JSON quoting expands this bounded metadata through the vCon,
                // ContextView and provider envelopes; the selected body still
                // stays below the unchanged32KiB ContextView budget.
                json!("\\".repeat(12 * 1024));
                Ok(Some(connect_store::records::RecordBytes {
                    uuid: bytes.uuid.clone(),
                    body: serde_json::to_vec(&body).unwrap(),
                    manifest: bytes.manifest.clone(),
                }))
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(padded.result.revision, 2);
}
fn source_until(f: &Fixture, uuid: &str) -> chrono::DateTime<chrono::Utc> {
    let manifest: Vec<u8> = rusqlite::Connection::open(&f.config.database)
        .unwrap()
        .query_row(
            "SELECT manifest FROM session_records WHERE uuid=?1",
            [uuid],
            |r| r.get(0),
        )
        .unwrap();
    let manifest: Value = serde_json::from_slice(&manifest).unwrap();
    let document = manifest["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["target"]["kind"] == "document")
        .unwrap();
    chrono::DateTime::parse_from_rfc3339(document["metadata_retention"]["until"].as_str().unwrap())
        .unwrap()
        .with_timezone(&chrono::Utc)
}
fn cutoff(f: &Fixture, uuid: &str) -> i64 {
    source_until(f, uuid).timestamp()
}
pub(super) fn creation_release(f: &Fixture, uuid: &str) -> i64 {
    cutoff(f, uuid) - 4
}
pub(super) async fn wait_for_source_cutoff(
    f: &Fixture,
    cid: &str,
    uuid: &str,
    clients: &mut [CanonicalClient],
    public: &mut CanonicalClient,
) {
    // Complete native preflight before the critical export interval. The real
    // HTTP creation response is held until four seconds before this unchanged
    // frozen cutoff; creation/export each retain their original five-second cap.
    let until = cutoff(f, uuid) - 6;
    while chrono::Utc::now().timestamp() < until {
        for peer in clients.iter_mut().chain(std::iter::once(&mut *public)) {
            let id = peer.keepalive().await.unwrap();
            reply(peer, &id, "ack").await;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(chrono::Utc::now().timestamp() < cutoff(f, uuid));
    assert_eq!(
        f.server
            .store
            .list_records("tnt_default", cid, Some("supervised-record-source"))
            .unwrap()[0]
            .revision,
        2
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn case(
    f: &Fixture,
    clients: &mut [CanonicalClient],
    cid: &str,
    sh: u16,
    th: u16,
    credential: &connect_store::IssuedCredential,
    command: &TransitionCommand,
    bridge: &mut Bridge,
    record: &str,
    provider: &Provider,
    bodies: &mut mpsc::UnboundedReceiver<Value>,
    remote_clock: tokio::time::Instant,
    primary_clock: tokio::time::Instant,
    mode: Case,
    uuid: &str,
    peer: &mut Peer,
) {
    let creation = tokio::time::timeout(Duration::from_secs(6), bodies.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(creation["assistantId"], "private-fixture-assistant");
    assert!(!creation.to_string().contains(uuid));
    let pending = peer.pending().await;
    let source_until = if mode == Case::AutomaticRecordWriteExpiry {
        Some(source_until(f, uuid))
    } else {
        None
    };
    assert!(pending.length > 16 + pending.queued_bytes + pending.in_flight_bound);
    assert!(pending.sender_buffer <= 8192 && pending.receiver_buffer > 0);
    assert_eq!(
        provider.exports.load(Ordering::SeqCst),
        0,
        "no complete frame reached the provider"
    );
    let mut cfg = canonical_http_config(f);
    cfg.credential = credential.token.clone();
    let mut client = CanonicalHttpClient::open(cfg).await.unwrap();
    let complete = mode == Case::AutomaticRecordWriteCancel;
    let cancel_scope = CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some(command.session_id.clone()),
        connection_id: command.connection_id.clone(),
        operation_id: Some("cancel-physical-provider-write".into()),
        ..Default::default()
    };
    let cancel_body = json!({"operation_id":command.operation_id,"reason":"cancel uncompleted provider initialization"});
    // The completing case records intent before queuing a writer, avoiding a
    // self-blocked cancellation request. The expiry case must stop its actual
    // write independently of cancellation and retain its definitive failure.
    if complete {
        client
            .request_value(
                "operation.cancel",
                cancel_body.clone(),
                cancel_scope.clone(),
            )
            .await
            .unwrap()
            .ack()
            .unwrap();
    }
    let writing = if complete {
        let mut config = canonical_http_config(f);
        config.credential = f.admin.clone();
        let mut api = connect_client::application::ApplicationClient::open(config)
            .await
            .unwrap();
        let request = connect_protocol::record_mutations::Request {
            operation_id: "erase-record-during-actual-provider-write".into(),
            conversation_id: cid.into(),
            session_id: "supervised-record-source".into(),
            uuid: uuid.into(),
            expected_revision: 2,
            action: connect_protocol::record_mutations::Action::Delete,
        };
        tokio::spawn(async move {
            let receipt = api.mutate_record(request.clone()).await.unwrap();
            assert_eq!(receipt.revision, 3);
            assert!(api.mutate_record(request).await.unwrap() == receipt);
            Ok::<(), connect_protocol::Error>(())
        })
    } else {
        let store = f.server.store.clone();
        let target = f.client.identity_id.clone();
        tokio::task::spawn_blocking(move || {
            store
                .acquire_authority(
                    &[connect_store::AuthorityScope::identity(
                        &target,
                        connect_store::AuthorityMode::Write,
                    )
                    .unwrap()],
                    &connect_store::AuthorityWait::new(Duration::from_secs(8)),
                )
                .map(drop)
        })
    };
    let readers = {
        let c = rusqlite::Connection::open(&f.config.database).unwrap();
        let mut q=c.prepare("SELECT identity_id FROM participants WHERE conversation_id=?1 AND active=1 ORDER BY identity_id").unwrap();
        q.query_map([cid], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|id| {
                connect_store::AuthorityScope::identity(
                    &id.unwrap(),
                    connect_store::AuthorityMode::Read,
                )
                .unwrap()
            })
            .collect::<Vec<_>>()
    };
    let until = tokio::time::Instant::now() + Duration::from_secs(1);
    loop {
        match f.server.store.try_acquire_authority(&readers) {
            Err(e) if e.code == "authority-busy" => break,
            Ok(held) => drop(held),
            Err(e) => panic!("writer probe: {}", e.code),
        }
        assert!(tokio::time::Instant::now() < until);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    if writing.is_finished() {
        let code = writing
            .await
            .unwrap()
            .err()
            .map(|e| e.code)
            .unwrap_or_else(|| "acquired-and-released".into());
        panic!("authority writer finished before the pending-write assertion: {code}; now={}; source_until={:?}",chrono::Utc::now().timestamp(),source_until.map(|t|t.timestamp()));
    }
    let current_queued = queued_bytes(
        pending.sender_fd,
        pending.receiver_fd,
        pending.client,
        pending.server,
    )
    .await;
    assert!(
        pending.length > 16 + current_queued + pending.in_flight_bound,
        "whole frame reached kernel queues before the authority writer queued"
    );
    println!("provider write custody: frame_bytes={}, initial_queued_bytes={}, writer_queued_bytes={}, in_flight_slack={}",pending.length,pending.queued_bytes,current_queued,pending.in_flight_bound);
    if complete {
        let received = peer.receive().await;
        assert!(received.complete);
        assert_eq!(received.received, received.length);
        assert!(
            !writing.is_finished(),
            "reading the full frame cannot replace the exact provider acknowledgement"
        );
        let export = tokio::time::timeout(Duration::from_secs(2), bodies.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(export["type"], "add-message");
        let data = export["message"]["content"]
            .as_str()
            .unwrap()
            .split_once("permission. ")
            .unwrap()
            .1;
        let view: connect_protocol::ContextView = serde_json::from_str(data).unwrap();
        let item = view.items.iter().find(|i| i.source_id == uuid).unwrap();
        let reference = item.record.as_ref().unwrap();
        assert_eq!(reference.uuid, uuid);
        assert_eq!(reference.session_id, "supervised-record-source");
        assert_eq!(reference.revision, 2);
        assert_eq!(
            reference.state,
            connect_protocol::ContextRecordState::Available
        );
        assert!(item.body.contains("transport_fixture_padding"));
        assert!(
            item.body.len() < 32 * 1024,
            "the canonical source body must remain inside the original context budget"
        );
        assert_eq!(
            reference.content_hash.as_deref(),
            Some(
                format!(
                    "sha512-{}",
                    base64::engine::general_purpose::URL_SAFE_NO_PAD
                        .encode(sha2::Sha512::digest(item.body.as_bytes()))
                )
                .as_str()
            )
        );
        provider.release_ack.notify_one();
    } else {
        let (expires,created):(i64,i64)=rusqlite::Connection::open(&f.config.database).unwrap().query_row("SELECT export_expires_at,created_at FROM private_provider_contexts WHERE operation_record_id=?1",[record],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(expires, source_until.as_ref().unwrap().timestamp());
        assert!(
            expires <= created + 4,
            "source retention must expire before the five-second IO cap"
        );
        tokio::time::timeout(Duration::from_secs(5), bridge.context_ready())
            .await
            .unwrap()
            .unwrap_err();
        let received = peer.receive().await;
        assert!(!received.complete);
        assert!(received.received < received.length);
        assert!(chrono::Utc::now().timestamp() >= expires);
        assert_eq!(provider.exports.load(Ordering::SeqCst), 0);
        // Expiry must fail the original operation without cancellation causing
        // either the partial write to retire or its authority guard to release.
    }
    tokio::time::timeout(Duration::from_secs(5), bridge.context_ready())
        .await
        .unwrap()
        .unwrap_err();
    let expected_state = if complete { "cancelled" } else { "failed" };
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let facts:(String,bool,bool,bool,u64)=rusqlite::Connection::open(&f.config.database).unwrap().query_row("SELECT o.state,o.cancel_requested,p.delivered_at IS NOT NULL,p.closed_at IS NOT NULL,(SELECT COUNT(*) FROM native_consultation_attempts n WHERE n.operation_record_id=o.record_id) FROM operations o JOIN private_provider_contexts p ON p.operation_record_id=o.record_id WHERE o.record_id=?1",[record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
        if facts == (expected_state.into(), complete, complete, true, 0) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < until,
            "unfinished write cleanup did not settle: {facts:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::timeout(Duration::from_secs(2), writing)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(f.server.store.authenticate(&credential.token).is_ok());
    let replay = client.transition(command.clone()).await.unwrap();
    assert_eq!(
        serde_json::to_value(replay.state).unwrap(),
        json!(expected_state)
    );
    assert_eq!(provider.creations.load(Ordering::SeqCst), 1);
    assert_eq!(
        provider.exports.load(Ordering::SeqCst),
        usize::from(complete)
    );
    assert_eq!(provider.ended.load(Ordering::SeqCst), 1);
    assert_eq!(provider.private_audio.load(Ordering::SeqCst), 0);
    if let Some(until) = source_until {
        if let Ok(wait) = (until - chrono::Utc::now()).to_std() {
            tokio::time::sleep(wait + Duration::from_millis(20)).await;
        }
        // Physical IO has already retired at the frozen source deadline. The
        // independently scheduled retention worker commits durable unavailability
        // afterward; do not confuse that sweep interval with missing-record state.
        let sweep = tokio::time::Instant::now() + Duration::from_secs(3);
        while !f
            .server
            .store
            .list_records("tnt_default", cid, Some("supervised-record-source"))
            .unwrap()
            .is_empty()
        {
            assert!(
                tokio::time::Instant::now() < sweep,
                "owned retention did not commit expiry"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    let expired = client
        .fetch_record(
            cid,
            "supervised-record-source",
            connect_client::canonical::CanonicalRecordSelector {
                uuid: Some(uuid.into()),
                ..Default::default()
            },
        )
        .await;
    assert!(
        matches!(&expired, Err(ClientError::Application(e)) if e.code == "not-found"),
        "fresh expired record read: {expired:?}"
    );
    clocked_audio_reaches(clients, sh, th, 123100, remote_clock).await;
    clients.swap(0, 1);
    clocked_audio_reaches(clients, th, sh, 124100, primary_clock).await;
    clients.swap(0, 1);
}
