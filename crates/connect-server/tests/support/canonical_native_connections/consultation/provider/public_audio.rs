//! Actual fresh SDK peer: no pre-ACK audio, public duplex, revocation and drain.
use super::*;
use connect_server::assistant_vapi::*;
#[derive(Clone)]
struct AudioProvider {
    base: String,
    reference: String,
    timing: AssistantProviderTiming,
    creations: Arc<AtomicUsize>,
    contexts: Arc<std::sync::Mutex<Vec<Value>>>,
    initial: Arc<std::sync::Mutex<Option<Value>>>,
    audio_enabled: Arc<AtomicBool>,
    acknowledge: Arc<AtomicBool>,
    exported: Arc<AtomicBool>,
    received: Arc<AtomicUsize>,
    ended: Arc<AtomicUsize>,
}
async fn create_audio(
    State(s): State<AudioProvider>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    assert_eq!(
        headers.get("authorization").unwrap(),
        "Bearer public-audio-token"
    );
    assert_eq!(body["assistantOverrides"]["model"]["tools"], json!([]));
    assert_eq!(s.creations.fetch_add(1, Ordering::SeqCst), 0);
    let messages = body["assistantOverrides"]["model"]["messages"]
        .as_array()
        .unwrap();
    if s.timing == AssistantProviderTiming::Initialization {
        assert_eq!(
            messages.len(),
            2,
            "required voice history must be in creation"
        );
        *s.initial.lock().unwrap() = Some(capture(&s, &messages[1]));
    } else {
        assert_eq!(messages.len(), 1);
    }
    Json(
        json!({"id":s.reference,"transport":{"websocketCallUrl":format!("{}/{}/transport",s.base.replace("http:","ws:"),s.reference)}}),
    )
}
async fn upgrade_audio(
    State(s): State<AudioProvider>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> impl IntoResponse {
    assert_eq!(
        headers.get("authorization").unwrap(),
        "Bearer public-audio-token"
    );
    upgrade.on_upgrade(move |ws| audio_socket(ws, s))
}
fn capture(s: &AudioProvider, message: &Value) -> Value {
    assert_eq!(message["role"], "system");
    let body = message["content"].as_str().unwrap();
    assert!(body.contains("authorized provider history marker"));
    assert!(!body.contains("assistant-context-pair-only marker"));
    assert!(!body.contains("assistant-context-ai-only marker"));
    let mut contexts = s.contexts.lock().unwrap();
    if contexts.len() == 1 {
        assert!(
            body.contains("UPDATED-PUBLIC-VOICE-HISTORY"),
            "initial history was submitted again instead of an update"
        );
    }
    contexts.push(message.clone());
    s.exported.store(true, Ordering::SeqCst);
    message.clone()
}
async fn audio_socket(mut ws: WebSocket, s: AudioProvider) {
    let mut message = s.initial.lock().unwrap().clone();
    let mut acked = false;
    let mut ticker = tokio::time::interval(Duration::from_millis(20));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _=ticker.tick(),if message.is_some()=>{
                if !acked && s.acknowledge.load(Ordering::SeqCst) {
                    if ws.send(Message::Text(json!({"type":"conversation-update","messagesOpenAIFormatted":[message.clone().unwrap()]}).to_string().into())).await.is_err(){return;}acked=true;
                }
                // Deliberately send speech before context acknowledgment too.
                if !s.audio_enabled.load(Ordering::SeqCst){continue;}
                let audio=(0..160).map(|n|if n%16<8{0x90}else{0x10}).collect::<Vec<_>>();
                if ws.send(Message::Binary(audio.into())).await.is_err(){return;}
            },
            frame=ws.recv()=>match frame {
                Some(Ok(Message::Text(text)))=>{
                    let value:Value=serde_json::from_str(&text).unwrap();
                    if value["type"]=="add-message" {
                        message=Some(capture(&s,&value["message"]));acked=false;

                    }
                    if value["type"]=="end-call" {
                        s.ended.fetch_add(1,Ordering::SeqCst);
                        let _=ws.send(Message::Text(json!({"type":"status-update","status":"ended"}).to_string().into())).await;
                    }
                },
                Some(Ok(Message::Binary(bytes)))=>{assert!(acked,"native audio exported before exact context ACK");if bytes.iter().any(|b|!matches!(*b,0xff|0x7f)){s.received.fetch_add(1,Ordering::SeqCst);}},
                Some(Ok(Message::Close(frame)))=>{let _=ws.send(Message::Close(frame)).await;return;},
                None|Some(Err(_))=>return,_=>{}
            }
        }
    }
}
pub(super) async fn verify(
    f: &Fixture,
    key: &str,
    endpoint: &str,
    cid: &str,
    config: &CanonicalClientConfig,
    public: &mut CanonicalClient,
    handle: u16,
    clock: tokio::time::Instant,
    sequence: &mut u32,
) {
    for (index, timing) in [
        AssistantProviderTiming::AfterCreation,
        AssistantProviderTiming::Initialization,
    ]
    .into_iter()
    .enumerate()
    {
        verify_one(
            f, key, endpoint, cid, config, public, handle, clock, sequence, index, timing,
        )
        .await;
    }
}
async fn verify_one(
    f: &Fixture,
    key: &str,
    endpoint: &str,
    cid: &str,
    config: &CanonicalClientConfig,
    public: &mut CanonicalClient,
    handle: u16,
    clock: tokio::time::Instant,
    sequence: &mut u32,
    index: usize,
    timing: AssistantProviderTiming,
) {
    quiet(public).await;
    let conn = format!("owned-public-provider-audio-{index}");
    let scope = CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        connection_id: Some(conn.clone()),
        operation_id: Some(format!("owned-public-provider-audio-offer-{index}")),
        expected_revision: Some(0),
    };
    let offer = json!({"substrate":"quic","capabilities":{"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"},"streams_offered":[{"id":format!("public-provider-voice-{index}"),"kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}]});
    let mut native = CanonicalClient::open(config.clone())
        .await
        .unwrap()
        .prepare_public_native_audio(scope, offer)
        .await
        .unwrap();
    assert_eq!(native.connection_id(), conn);
    assert_eq!(native.generation(), 1);
    let primary = f
        .server
        .store
        .membership_snapshot(key, cid)
        .unwrap()
        .participants
        .into_iter()
        .find(|p| p.identity_id == f.client.identity_id)
        .unwrap()
        .participant_id;
    let initial_revision = native.stream_revision();
    let subscriptions =
        vec![
            serde_json::from_value(json!({"from_participant":primary,"kinds":["audio"]})).unwrap(),
        ];
    for _ in 0..2 {
        let revision = native
            .subscribe_audio(
                &format!("owned-public-provider-hears-primary-{index}"),
                initial_revision,
                subscriptions.clone(),
            )
            .await
            .unwrap();
        assert_eq!(revision, initial_revision + 1);
        assert_eq!(native.stream_revision(), revision);
    }
    let actor = f.server.store.revalidate(key).unwrap();
    let context_key = f
        .server
        .store
        .issue(
            &actor.identity_id,
            actor.scope,
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap()
        .id;
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    let selected = loop {
        match f
            .server
            .service
            .try_assistant_context_read(&context_key, &conn, 1, endpoint)
        {
            Ok(read) => break read,
            Err(e)
                if matches!(
                    e.code.as_str(),
                    "not-ready" | "authority-busy" | "storage-unavailable"
                ) && tokio::time::Instant::now() < until =>
            {
                tokio::time::sleep(Duration::from_millis(20)).await
            }
            Err(e) => panic!("owned audio context: {}", e.code),
        }
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let provider = AudioProvider {
        base: base.clone(),
        reference: format!("public-audio-call-{index}"),
        timing,
        creations: Arc::new(AtomicUsize::new(0)),
        contexts: Arc::new(std::sync::Mutex::new(Vec::new())),
        initial: Arc::new(std::sync::Mutex::new(None)),
        audio_enabled: Arc::new(AtomicBool::new(true)),
        acknowledge: Arc::new(AtomicBool::new(false)),
        exported: Arc::new(AtomicBool::new(false)),
        received: Arc::new(AtomicUsize::new(0)),
        ended: Arc::new(AtomicUsize::new(0)),
    };
    let app = Router::new()
        .route("/call", post(create_audio))
        .route(
            &format!("/{}/transport", provider.reference),
            get(upgrade_audio),
        )
        .with_state(provider.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let profile = AssistantVapiProfile {
        observations: None,
        context_timing: timing,
        api_base: format!("{base}/").parse().unwrap(),
        websocket_origin: format!("{}/", base.replace("http:", "ws:"))
            .parse()
            .unwrap(),
        loopback_fixture: true,
        model: "designated-public-audio-model".into(),
        application_prompt: "Use only the supplied attributed public history.".into(),
    };
    let secrets: ProviderSecrets = Arc::new(|reference| {
        assert_eq!(reference, "private_fixture_vapi_secret");
        connect_server::secrets::SecretValue::new("public-audio-token".into())
            .map_err(|_| connect_protocol::Error::forbidden())
    });
    let mut job = AssistantVapiContextSession::start_with_audio(
        f.server.service.clone(),
        format!("owned-public-provider-context-{index}"),
        selected,
        profile,
        secrets,
        native,
    )
    .await
    .unwrap();
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    while !provider.exported.load(Ordering::SeqCst) {
        assert!(tokio::time::Instant::now() < until, "context not exported");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let deadline = tokio::time::sleep(Duration::from_millis(400));
    tokio::pin!(deadline);
    let mut pre_ack_packets = 0;
    loop {
        tokio::select! {
            _=&mut deadline=>break,
        packet=public.receive_native_media_datagram()=>{let packet=rvoip_uctp::substrate::unpack_rtp_datagram(&packet.unwrap()).unwrap();pre_ack_packets+=1;assert!(packet.rtp.payload.iter().all(|b|matches!(*b,0xff|0x7f)),"provider speech reached public audience before context ACK");}
        }
    }
    assert_eq!(provider.received.load(Ordering::SeqCst), 0);
    provider.acknowledge.store(true, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(6), job.context_ready())
        .await
        .unwrap()
        .unwrap();
    let received = duplex(public, handle, clock, sequence, &provider, 16, &job).await;
    provider.audio_enabled.store(false, Ordering::SeqCst);
    quiet(public).await;
    let marker = format!("UPDATED-PUBLIC-VOICE-HISTORY-{index}");
    let mut client = connect_client::canonical::CanonicalHttpClient::open(canonical_http_config(f))
        .await
        .unwrap();
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match client
            .send_message(
                &format!("public-voice-update-source-{index}"),
                cid,
                &format!("public-voice-update-message-{index}"),
                connect_client::messages::MessageDraft::text_all(&marker),
            )
            .await
        {
            Ok(_) => break,
            Err(connect_client::ClientError::Application(e))
                if matches!(e.code.as_str(), "authority-busy" | "storage-unavailable")
                    && tokio::time::Instant::now() < until =>
            {
                tokio::time::sleep(Duration::from_millis(20)).await
            }
            Err(e) => panic!("public voice source message failed: {e:?}"),
        }
    }
    provider.acknowledge.store(false, Ordering::SeqCst);
    let operation = format!("public-voice-context-update-{index}");
    let update_before = provider.received.load(Ordering::SeqCst);
    let update_id = {
        let update = job.update_context(&operation);
        tokio::pin!(update);
        let until = tokio::time::sleep(Duration::from_secs(6));
        tokio::pin!(until);
        loop {
            if provider.contexts.lock().unwrap().len() == 2 {
                break;
            }
            tokio::select! {
                _=&mut until=>panic!("context update was not exported"),
                result=&mut update=>panic!("update completed without its context echo: {result:?}"),
                _=tokio::time::sleep(Duration::from_millis(10))=>{},
            }
        }
        let before = provider.received.load(Ordering::SeqCst);
        provider.audio_enabled.store(true, Ordering::SeqCst);
        let window = tokio::time::sleep(Duration::from_millis(400));
        tokio::pin!(window);
        loop {
            tokio::select! {
                _=&mut window=>break,
                result=&mut update=>panic!("update completed before exact ACK: {result:?}"),
                packet=public.receive_native_media_datagram()=>{let p=rvoip_uctp::substrate::unpack_rtp_datagram(&packet.unwrap()).unwrap();assert!(p.rtp.payload.iter().all(|b|matches!(*b,0xff|0x7f)),"provider speech escaped during an unacknowledged context update");}
            }
        }
        assert_eq!(
            provider.received.load(Ordering::SeqCst),
            before,
            "native audio continued during context update"
        );
        provider.acknowledge.store(true, Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(5), &mut update)
            .await
            .unwrap()
            .unwrap()
    };
    assert_eq!(
        job.update_context(&operation).await.unwrap(),
        update_id,
        "logical replay changed receipt"
    );
    assert_eq!(
        provider.contexts.lock().unwrap().len(),
        2,
        "logical replay repeated context export"
    );
    assert_eq!(provider.creations.load(Ordering::SeqCst), 1);
    let resumed = duplex(
        public,
        handle,
        clock,
        sequence,
        &provider,
        update_before + 16,
        &job,
    )
    .await;
    let mut api = connect_client::application::ApplicationClient::open(canonical_http_config(f))
        .await
        .unwrap();
    let op = connect_client::application::ApplicationOperation::try_from(
        connect_protocol::Operation::ContextDeliveries {
            conversation_id: cid.into(),
            before: None,
            limit: 100,
        },
    )
    .unwrap();
    let connect_protocol::Outcome::ContextDeliveries(page) = api.execute(op).await.unwrap() else {
        panic!()
    };
    let original = page
        .deliveries
        .iter()
        .find(|d| d.id == job.delivery_id())
        .unwrap();
    assert_eq!(
        original.lifecycle.as_ref().unwrap().phase,
        if timing == AssistantProviderTiming::Initialization {
            connect_protocol::ContextDeliveryPhase::Initialization
        } else {
            connect_protocol::ContextDeliveryPhase::PostCreation
        }
    );
    let updated = page.deliveries.iter().find(|d| d.id == update_id).unwrap();
    let lifecycle = updated.lifecycle.as_ref().unwrap();
    assert_eq!(
        lifecycle.phase,
        connect_protocol::ContextDeliveryPhase::Update
    );
    assert_eq!(
        lifecycle.parent_delivery_id.as_deref(),
        Some(job.delivery_id())
    );
    assert_eq!(updated.state, "delivered");
    assert_eq!(
        updated.provider_ref.as_deref(),
        Some(provider.reference.as_str())
    );
    assert!(
        !lifecycle.creation_admitted
            && lifecycle.creation_confirmed
            && lifecycle.delivery_confirmed
    );
    assert_eq!(updated.destination.as_ref().unwrap().connection_id, conn);
    {
        let contexts = provider.contexts.lock().unwrap();
        assert!(!contexts[0]["content"].as_str().unwrap().contains(&marker));
        let view: connect_protocol::ContextView =
            serde_json::from_str(contexts[1]["content"].as_str().unwrap()).unwrap();
        let item = view.items.iter().find(|i| i.body == marker).unwrap();
        assert_eq!(item.actor_id, f.client.identity_id);
        assert!(!item.source_id.is_empty());
    }
    let admin = f.server.store.authenticate(&f.admin).unwrap();
    let store = f.server.store.clone();
    tokio::task::spawn_blocking(move || {
        let until = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            match store.revoke(&admin, &context_key) {
                Ok(()) => break,
                Err(e) if e.code == "authority-busy" && std::time::Instant::now() < until => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                Err(e) => panic!("public audio context revocation failed: {}", e.code),
            }
        }
    })
    .await
    .unwrap();
    let until = tokio::time::Instant::now() + Duration::from_secs(8);
    while !matches!(job.status(), AssistantVapiStatus::Stopped { .. }) {
        assert!(
            tokio::time::Instant::now() < until,
            "revoked audio job did not stop/drain"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(job.shutdown().await.is_err());
    assert_eq!(provider.ended.load(Ordering::SeqCst), 1);
    let before = provider.received.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(provider.received.load(Ordering::SeqCst), before);
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let (state,closed):(String,bool)=c.query_row("SELECT v.state,p.closed_at IS NOT NULL FROM native_assistant_providers p JOIN connections v ON v.id=p.connection_id WHERE p.connection_id=?1",[&conn],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(state, "retired");
    assert!(closed);
    println!("actual public provider audio: {timing:?}; owned signed peer; pre-ACK packets={pre_ack_packets}; initial native frames={received}; update resumed native frames={resumed}; provider frames={before}; exact update/replay receipts; context revocation closed provider and drained owned peer");
    server.abort();
    let _ = server.await;
}

async fn quiet(public: &mut CanonicalClient) {
    // This retained peer has not been read during the nine control-only cases.
    // Drain their older native datagrams and establish silence before attributing
    // a newly observed packet to this provider's deliberately premature speech.
    let until = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut quiet_since = tokio::time::Instant::now();
    let mut settle = tokio::time::interval(Duration::from_millis(20));
    loop {
        tokio::select! {
            _=settle.tick()=>{
                assert!(tokio::time::Instant::now()<until,"old public audio queue did not settle");
                if quiet_since.elapsed()>=Duration::from_millis(250){break;}
            },
            packet=public.receive_native_media_datagram()=>{
                let packet=rvoip_uctp::substrate::unpack_rtp_datagram(&packet.unwrap()).unwrap();
                if packet.rtp.payload.iter().any(|b|!matches!(*b,0xff|0x7f)){quiet_since=tokio::time::Instant::now();}
            }
        }
    }
}

async fn duplex(
    public: &mut CanonicalClient,
    handle: u16,
    clock: tokio::time::Instant,
    sequence: &mut u32,
    provider: &AudioProvider,
    minimum_provider: usize,
    job: &AssistantVapiContextSession,
) -> usize {
    let mut seq = *sequence;
    let mut received = 0;
    let mut ticker = tokio::time::interval(Duration::from_millis(20));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let deadline = tokio::time::sleep(Duration::from_secs(10));
    tokio::pin!(deadline);
    loop {
        if received >= 16 && provider.received.load(Ordering::SeqCst) >= minimum_provider {
            break;
        }
        tokio::select! {
            _=&mut deadline=>panic!("public provider duplex incomplete: timing={:?},native={received},provider={},end_commands={},status={:?}",provider.timing,provider.received.load(Ordering::SeqCst),provider.ended.load(Ordering::SeqCst),job.status()),
            _=ticker.tick()=>{
                use rvoip_uctp::substrate::{pack_rtp_datagram,RtpDatagram,RtpMediaPayload};
                let audio=(0..160).map(|n|if n%16<8{0x80}else{0x00}).collect::<Vec<_>>();
                public.send_native_media_datagram(&pack_rtp_datagram(&RtpDatagram{flags:0,stream_local_id:handle,seq,rtp:RtpMediaPayload{payload:audio.into(),payload_type:0,sequence_number:seq as u16,timestamp:(clock.elapsed().as_micros()*8/1000)as u32,ssrc:0x4736_0001}}).unwrap()).unwrap();seq+=1;
            },
            packet=public.receive_native_media_datagram()=>{let packet=rvoip_uctp::substrate::unpack_rtp_datagram(&packet.unwrap()).unwrap();if packet.rtp.payload.iter().any(|b|!matches!(*b,0xff|0x7f)){received+=1;}}
        }
    }
    *sequence = seq;
    received
}
