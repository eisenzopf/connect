//! Public SDK publisher to an independently controlled/decoded native receiver.
use super::*;
pub(super) const VP8: &[u8] = include_bytes!("../../fixtures/gate6-vp8-black-640x360.vp8");
pub(super) fn rtp(
    pt: u8,
    sequence: u16,
    timestamp: u32,
    ssrc: u32,
    marker: bool,
    payload: &[u8],
) -> Vec<u8> {
    let mut bytes = vec![0x80, pt | if marker { 0x80 } else { 0 }];
    bytes.extend_from_slice(&sequence.to_be_bytes());
    bytes.extend_from_slice(&timestamp.to_be_bytes());
    bytes.extend_from_slice(&ssrc.to_be_bytes());
    bytes.extend_from_slice(payload);
    bytes
}
fn capabilities() -> Value {
    json!({"audio_codecs":[{"name":"pcmu","params":{"sample_rate":8000,"channels":1}}],"video_codecs":[{"name":"vp8","params":{"sample_rate":90000,"channels":0}}],"max_streams_per_connection":8,"identity_assurance_offered":"identified"})
}
async fn subscribe(target: &mut CanonicalClient, cid: &str) -> Value {
    let scope = CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        connection_id: Some("sdk-media-target".into()),
        operation_id: Some("sdk-media-selection".into()),
        expected_revision: Some(1),
    };
    target.send_value("stream.subscribe",json!({"subscriptions":[{"strm_id":"sdk-publish-audio"},{"strm_id":"sdk-publish-video"}]}),scope.clone()).await.unwrap();
    let mut setup = Value::Null;
    loop {
        let frame =
            super::super::canonical_native_connections::ControlReception::receive_control(target)
                .await
                .unwrap();
        match frame.kind() {
            "connection.offer" => {
                let p = &frame.original()["payload"];
                setup = p["substrate_setup"].clone();
                let answered=setup["streams"].as_array().unwrap().iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],"direction":match s["direction"].as_str().unwrap(){"sendonly"=>"recvonly","recvonly"=>"sendonly","sendrecv"=>"sendrecv",_=>panic!("inactive selected port")},"codec":s["codec"]})).collect::<Vec<_>>();
                target.answer_connection_offer(&frame,serde_json::from_value(json!({"by_participant":p["by_participant"],"substrate":"quic","capabilities":capabilities(),"streams_answered":answered,"substrate_setup":setup})).unwrap(),&scope).await.unwrap();
            }
            "operation.updated" => assert_eq!(frame.original()["payload"]["state"], "completed"),
            "ack" => return setup,
            _ => panic!("unexpected selection reply: {:?}", frame.original()),
        }
    }
}
pub(super) fn decoded_audio(directory: &std::path::Path, encoded: &[u8]) -> usize {
    let path = directory.join("sdk-received.mulaw");
    std::fs::write(&path, encoded).unwrap();
    let output = std::process::Command::new("ffmpeg")
        .args([
            "-v", "error", "-f", "mulaw", "-ar", "8000", "-ac", "1", "-i",
        ])
        .arg(path)
        .args(["-f", "s16le", "pipe:1"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout.len(), encoded.len() * 2);
    let loud = output
        .stdout
        .chunks_exact(2)
        .filter(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs() > 500)
        .count();
    assert!(loud > 500);
    loud
}
pub(super) fn decoded_video(directory: &std::path::Path, encoded: &[u8]) -> usize {
    let mut ivf = b"DKIF".to_vec();
    ivf.extend_from_slice(&0u16.to_le_bytes());
    ivf.extend_from_slice(&32u16.to_le_bytes());
    ivf.extend_from_slice(b"VP80");
    ivf.extend_from_slice(&640u16.to_le_bytes());
    ivf.extend_from_slice(&360u16.to_le_bytes());
    for value in [10u32, 1, 1, 0] {
        ivf.extend_from_slice(&value.to_le_bytes());
    }
    ivf.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
    ivf.extend_from_slice(&0u64.to_le_bytes());
    ivf.extend_from_slice(encoded);
    let path = directory.join("sdk-received.ivf");
    std::fs::write(&path, ivf).unwrap();
    let output = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-f", "ivf", "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "pipe:1",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout.len(), 640 * 360 * 3);
    assert!(output.stdout.iter().all(|p| *p < 8));
    output.stdout.len()
}
#[tokio::test]
async fn public_sdk_native_rtp_publisher_routes_independently_decoded_audio_video_and_keeps_control_live(
) {
    for algorithm in ["EdDSA", "ES256"] {
        let f = Fixture::with_native_end_scope(algorithm, 16).await;
        super::super::canonical_native_connections::ready_endpoint_with_capabilities(
            &f,
            vec!["audio".into(), "video".into()],
        )
        .await;
        let mut scope = Scope::collaboration(vec!["*".into()]);
        scope
            .operations
            .extend(["stream.subscribe".into(), "stream.unsubscribe".into()]);
        let target_config = super::super::canonical_native_connections::party_with_scope(
            &f,
            &f.peer.identity_id,
            173,
            vec!["audio".into(), "video".into()],
            scope,
        );
        let client = Client::connect(f.client.clone(), "tnt_default".parse().unwrap())
            .await
            .unwrap();
        let room = client
            .create_conversation(
                vec![f.peer.identity_id.parse().unwrap()],
                ConversationPolicy::Persistent,
                mutation("sdk-media-create"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        let view = room.refresh(options()).await.unwrap();
        let recipient = view
            .participants
            .iter()
            .find(|p| p.identity_id.as_str() == f.peer.identity_id)
            .unwrap()
            .id
            .clone();
        let caps = json!({"audio":["g.711-mu","g.711-a"],"video":["vp8"]});
        let session = room
            .invite(
                "native-session".parse().unwrap(),
                vec![recipient],
                sdk::SessionMedium::VoiceVideo,
                "conference".into(),
                caps.clone(),
                mutation("sdk-media-invite"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        let mut target = CanonicalClient::open(target_config).await.unwrap();
        target
            .session_command(connect_client::session_control::Request {
                operation_id: "sdk-media-accept".into(),
                conversation_id: room.id().as_str().into(),
                session_id: "native-session".into(),
                by_participant: None,
                expected_revision: Some(0),
                action: connect_client::session_control::Action::Accept { capabilities: caps },
            })
            .await
            .unwrap();
        let stream = |id: &str, kind, direction, codec: &str| sdk::OfferedStream {
            id: id.parse().unwrap(),
            kind,
            direction,
            codecs: NonEmpty {
                first: codec.into(),
                rest: vec![],
            },
        };
        let mut offered_audio = stream(
            "sdk-publish-audio",
            sdk::Medium::Audio,
            sdk::Direction::Sendrecv,
            "g.711-mu",
        );
        offered_audio.codecs.rest.push("pcma".into());
        let offer = session
            .offer_connection(
                "sdk-media-source".parse().unwrap(),
                NonEmpty {
                    first: offered_audio,
                    rest: vec![stream(
                        "sdk-publish-video",
                        sdk::Medium::Video,
                        sdk::Direction::Sendonly,
                        "vp8",
                    )],
                },
                mutation("sdk-media-offer"),
            )
            .await
            .unwrap();
        let connection = offer.wait(options()).await.unwrap().value;
        let audio = connection.stream_by_id("sdk-publish-audio".parse().unwrap());
        let video = connection.stream_by_id("sdk-publish-video".parse().unwrap());
        let initial=super::super::canonical_native_connections::negotiate_media_payload(&mut target,room.id().as_str(),"sdk-media-target-offer","sdk-media-target",json!({"substrate":"quic","capabilities":capabilities(),"streams_offered":[{"id":"sdk-monitor-audio","kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]},{"id":"sdk-monitor-video","kind":"video","direction":"recvonly","codec_preferences":["vp8"]}]})).await;
        let changed = subscribe(&mut target, room.id().as_str()).await;
        let setup = if changed.is_null() {
            initial.original()["payload"]["substrate_setup"].clone()
        } else {
            changed
        };
        let handles: std::collections::BTreeMap<u16, String> = setup["streams"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| matches!(s["direction"].as_str(), Some("sendonly" | "sendrecv")))
            .map(|s| {
                (
                    s["stream_local_id"].as_u64().unwrap() as u16,
                    s["kind"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        target.start_signaling_reader().unwrap();
        for stream in [&audio, &video] {
            let current = stream.refresh(options()).await.unwrap();
            assert_eq!(&current.id, stream.id());
            assert_eq!(&current.connection_id, connection.id());
            assert_eq!(current.ownership, sdk::MediaOwnership::Observation);
            assert_eq!(current.state, "negotiated");
            assert!(current.revision.get() > 0);
            assert!(
                current.format.is_none() && current.direction.is_none() && current.clock.is_none()
            );
        }
        let cancelled = options();
        cancelled.signal.cancel();
        assert!(
            matches!(audio.receive_packet(cancelled).await,Err(e) if e.kind==sdk::ErrorKind::Cancelled)
        );
        let idle = RequestOptions::new(Duration::from_millis(30)).unwrap();
        assert!(
            matches!(audio.receive_packet(idle).await,Err(e) if e.kind==sdk::ErrorKind::Deadline)
        );
        let a = audio.clone();
        let v = video.clone();
        let sending = async move {
            for index in 0..120u16 {
                let pcmu = (0..160)
                    .map(|i| if i % 16 < 8 { 0x80 } else { 0x00 })
                    .collect::<Vec<_>>();
                a.send_rtp(
                    sdk::RtpPacket::new(
                        u32::from(index),
                        rtp(0, index, u32::from(index) * 160, 0x735f6101, false, &pcmu),
                    )
                    .unwrap(),
                    options(),
                )
                .await
                .unwrap();
                if index % 5 == 0 {
                    let mut payload = vec![0x10];
                    payload.extend_from_slice(VP8);
                    v.send_rtp(
                        sdk::RtpPacket::new(
                            u32::from(index / 5),
                            rtp(
                                96,
                                index / 5,
                                u32::from(index) * 1800,
                                0x735f7601,
                                true,
                                &payload,
                            ),
                        )
                        .unwrap(),
                        options(),
                    )
                    .await
                    .unwrap();
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        let receiving = tokio::time::timeout(Duration::from_secs(5), async {
            let mut audio = Vec::new();
            let mut video = None;
            let mut assembler = connect_client::VideoFrameAssembler::default();
            while audio.len() < 1600 || video.is_none() {
                let bytes = target.receive_native_media_datagram().await.unwrap();
                let connect_client::canonical::ObservedNativeMediaDatagram::Rtp(packet) =
                    connect_client::canonical::observe_native_media_datagram(&bytes).unwrap()
                else {
                    continue;
                };
                if handles.get(&packet.stream_local_id).map(String::as_str) == Some("audio") {
                    assert_eq!(packet.rtp.packet().header.payload_type, 0);
                    audio.extend_from_slice(&packet.rtp.packet().payload);
                } else if handles.get(&packet.stream_local_id).map(String::as_str) == Some("video")
                {
                    let frame = connect_client::NativeVideoPacket::parse(
                        packet.rtp.packet().serialize().unwrap().to_vec(),
                    )
                    .unwrap();
                    assert!(frame.marker);
                    if let Some(frame) = assembler.push(&frame).unwrap() {
                        assert_eq!(frame.encoded, VP8);
                        assert!(frame.keyframe);
                        video = Some(frame.encoded);
                    }
                } else {
                    panic!(
                        "foreign target stream handle {} outside {:?}",
                        packet.stream_local_id, handles
                    );
                }
            }
            (audio, video.unwrap())
        });
        let (_, received) = tokio::join!(sending, receiving);
        let (received_audio, received_video) =
            received.expect("SDK-produced AV must reach the independent subscribed peer");
        let loud = decoded_audio(f.temp.path(), &received_audio);
        let pixels = decoded_video(f.temp.path(), &received_video);
        assert_eq!(
            offer.status(options()).await.unwrap().state,
            sdk::OperationState::Completed
        );
        let stats = client.media_stats().unwrap();
        assert!(stats.queued_bytes <= 4 * 1024 * 1024 && stats.queued_packets <= 256);
        println!("SDK RTP {algorithm}: decoded_audio_bytes={}, loud_samples={loud}, decoded_rgb_bytes={pixels}, source_media_queued={}",received_audio.len()*2,stats.queued_packets);
        let next_audio =
            super::runtime_controls::exercise(&f, &client, &connection, &audio, &video).await;
        super::codec_updates::replace(&f, &client, &room, &session, &connection, &audio, &video)
            .await;
        let spare = session
            .offer_connection(
                "sdk-spare-connection".parse().unwrap(),
                NonEmpty {
                    first: stream(
                        "sdk-spare-audio",
                        sdk::Medium::Audio,
                        sdk::Direction::Recvonly,
                        "g.711-mu",
                    ),
                    rest: vec![],
                },
                mutation("sdk-spare-offer"),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        let spare_stream = spare.stream_by_id("sdk-spare-audio".parse().unwrap());
        let pending_read = spare_stream.receive_packet(options());
        tokio::pin!(pending_read);
        assert!(
            tokio::time::timeout(Duration::from_millis(10), &mut pending_read)
                .await
                .is_err()
        );
        spare
            .end(200, "normal".into(), None, mutation("sdk-spare-end"))
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        assert!(
            matches!(tokio::time::timeout(Duration::from_millis(500),pending_read).await.expect("only ended Connection reader wakes"),Err(e) if e.kind==sdk::ErrorKind::Unsupported)
        );
        assert!(spare.native_negotiation().unwrap().is_none());
        assert!(connection.native_negotiation().unwrap().is_some());
        // Verify actual media on the other Connection after the typed end receipt.
        // The new 1kHz waveform excludes queued 500Hz packets from the AV scene.
        // Core packetization may assign a new RTP identity to routed audio.
        let sending = async {
            for index in next_audio..next_audio + 40 {
                let pcmu = (0..160)
                    .map(|i| if i % 8 < 4 { 0xaa } else { 0x2a })
                    .collect::<Vec<_>>();
                audio
                    .send_rtp(
                        sdk::RtpPacket::new(
                            u32::from(index),
                            rtp(8, index, u32::from(index) * 160, 0x735f6101, false, &pcmu),
                        )
                        .unwrap(),
                        options(),
                    )
                    .await
                    .unwrap();
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        let receiving = tokio::time::timeout(Duration::from_secs(5), async {
            let mut encoded = Vec::new();
            while encoded.len() < 1600 {
                let bytes = target.receive_native_media_datagram().await.unwrap();
                if let connect_client::canonical::ObservedNativeMediaDatagram::Rtp(packet) =
                    connect_client::canonical::observe_native_media_datagram(&bytes).unwrap()
                {
                    if handles.get(&packet.stream_local_id).map(String::as_str) == Some("audio") {
                        let payload = &packet.rtp.packet().payload;
                        let transitions = payload
                            .windows(2)
                            .filter(|pair| (pair[0] & 0x80) != (pair[1] & 0x80))
                            .count();
                        // 1kHz has about40 sign transitions per160samples; old500Hz has20.
                        if transitions >= 30 && transitions <= 50 {
                            encoded.extend_from_slice(payload);
                        }
                    }
                }
            }
            encoded
        });
        let (_, after_end) = tokio::join!(sending, receiving);
        let after_end =
            after_end.expect("ending spare Connection preserves independently decoded audio");
        let after_loud = decoded_audio(f.temp.path(), &after_end);
        println!(
            "SDK end {algorithm}: unrelated_audio_bytes={}, loud_samples={after_loud}",
            after_end.len() * 2
        );
        target.shutdown().await.unwrap();
        client.close(options()).await.unwrap();
        assert!(
            matches!(audio.send_rtp(sdk::RtpPacket::new(999,rtp(0,999,999*160,1,false,&[0x7f;160])).unwrap(),options()).await,Err(e) if e.kind==sdk::ErrorKind::Cancelled && e.certainty==sdk::Certainty::NotSubmitted)
        );
        f.server.shutdown().await.unwrap();
    }
}
