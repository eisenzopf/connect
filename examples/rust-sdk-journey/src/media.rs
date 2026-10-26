use super::*;
use sdk::{Direction, Medium, NonEmpty};
const VP8: &[u8] = include_bytes!("black-640x360.vp8");
fn stream(
    id: &str,
    kind: Medium,
    direction: Direction,
    codec: &str,
) -> sdk::Result<sdk::OfferedStream> {
    Ok(sdk::OfferedStream {
        id: id.parse()?,
        kind,
        direction,
        codecs: NonEmpty {
            first: codec.into(),
            rest: vec![],
        },
    })
}
fn rtp(pt: u8, seq: u16, timestamp: u32, ssrc: u32, marker: bool, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x80, pt | if marker { 0x80 } else { 0 }];
    bytes.extend(seq.to_be_bytes());
    bytes.extend(timestamp.to_be_bytes());
    bytes.extend(ssrc.to_be_bytes());
    bytes.extend(payload);
    bytes
}
async fn send(audio: sdk::Stream, video: sdk::Stream) -> Result<()> {
    let clock = tokio::time::Instant::now();
    for i in 0..100u16 {
        let payload = (0..160)
            .map(|n| if n % 16 < 8 { 0x80 } else { 0 })
            .collect::<Vec<_>>();
        audio
            .send_rtp(
                sdk::RtpPacket::new(
                    u32::from(i),
                    rtp(
                        0,
                        i,
                        (clock.elapsed().as_micros() * 8 / 1000) as u32,
                        0x45585401,
                        false,
                        &payload,
                    ),
                )?,
                options(),
            )
            .await?;
        if i % 5 == 0 {
            let mut payload = vec![0x10];
            payload.extend(VP8);
            video
                .send_rtp(
                    sdk::RtpPacket::new(
                        u32::from(i / 5),
                        rtp(
                            96,
                            i / 5,
                            (clock.elapsed().as_micros() * 90 / 1000) as u32,
                            0x45585601,
                            true,
                            &payload,
                        ),
                    )?,
                    options(),
                )
                .await?;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Ok(())
}
async fn receive(audio: sdk::Stream, video: sdk::Stream) -> Result<(Vec<u8>, Vec<u8>)> {
    let audio = async {
        let mut bytes = vec![];
        while bytes.len() < 1600 {
            if let sdk::MediaPacket::Rtp(packet) = audio
                .receive_packet(RequestOptions::new(Duration::from_secs(5))?)
                .await?
            {
                if packet.header().payload_type != 0 {
                    return Err("unexpected audio payload type".into());
                }
                bytes.extend(packet.payload());
            }
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(bytes)
    };
    let video = async {
        let mut assembler = connect_client::VideoFrameAssembler::default();
        loop {
            if let sdk::MediaPacket::Rtp(packet) = video
                .receive_packet(RequestOptions::new(Duration::from_secs(5))?)
                .await?
            {
                let packet = connect_client::NativeVideoPacket::parse(packet.bytes().to_vec())?;
                if let Some(frame) = assembler.push(&packet)? {
                    if !frame.keyframe || frame.encoded != VP8 {
                        return Err("unexpected video frame".into());
                    }
                    return Ok::<_, Box<dyn std::error::Error + Send + Sync>>(frame.encoded);
                }
            }
        }
    };
    let (audio, video) = tokio::try_join!(audio, video)?;
    Ok((audio, video))
}
async fn decode(output: &std::path::Path, audio: Vec<u8>, video: Vec<u8>) -> Result<Value> {
    tokio::fs::create_dir_all(output).await?;
    let a = output.join("received.mulaw");
    tokio::fs::write(&a, &audio).await?;
    let mut command = tokio::process::Command::new("ffmpeg");
    command
        .kill_on_drop(true)
        .args([
            "-v", "error", "-f", "mulaw", "-ar", "8000", "-ac", "1", "-i",
        ])
        .arg(&a)
        .args(["-f", "s16le", "pipe:1"]);
    let decoded = tokio::time::timeout(Duration::from_secs(10), command.output()).await??;
    if !decoded.status.success() || decoded.stdout.len() != audio.len() * 2 {
        return Err("independent audio decode failed".into());
    }
    let loud = decoded
        .stdout
        .chunks_exact(2)
        .filter(|v| i16::from_le_bytes([v[0], v[1]]).unsigned_abs() > 500)
        .count();
    if loud < 500 {
        return Err("decoded audio lacked identified signal".into());
    }
    let mut ivf = b"DKIF".to_vec();
    ivf.extend(0u16.to_le_bytes());
    ivf.extend(32u16.to_le_bytes());
    ivf.extend(b"VP80");
    ivf.extend(640u16.to_le_bytes());
    ivf.extend(360u16.to_le_bytes());
    for n in [10u32, 1, 1, 0] {
        ivf.extend(n.to_le_bytes());
    }
    ivf.extend((video.len() as u32).to_le_bytes());
    ivf.extend(0u64.to_le_bytes());
    ivf.extend(video);
    let v = output.join("received.ivf");
    tokio::fs::write(&v, ivf).await?;
    let mut command = tokio::process::Command::new("ffmpeg");
    command
        .kill_on_drop(true)
        .args(["-v", "error", "-f", "ivf", "-i"])
        .arg(v)
        .args([
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "pipe:1",
        ]);
    let decoded = tokio::time::timeout(Duration::from_secs(10), command.output()).await??;
    if !decoded.status.success()
        || decoded.stdout.len() != 640 * 360 * 3
        || decoded.stdout.iter().any(|v| *v >= 8)
    {
        return Err("independent identified video decode failed".into());
    }
    Ok(
        json!({"encoded_audio_bytes":audio.len(),"decoded_loud_samples":loud,"decoded_rgb_bytes":decoded.stdout.len()}),
    )
}
pub(super) async fn run(
    publisher: ClientConfig,
    subscriber: ClientConfig,
    output: PathBuf,
) -> Result<()> {
    let peer_identity = subscriber.control.identity_id.clone();
    let publisher = Client::connect_configured(publisher).await?;
    let subscriber = Client::connect_configured(subscriber).await?;
    let result = perform(&publisher, &subscriber, &peer_identity, &output).await;
    let (a, b) = tokio::join!(publisher.close(options()), subscriber.close(options()));
    a?;
    b?;
    let value = result?;
    emit(json!({"phase":"completed","summary":value,"owners_joined":true})).await
}
async fn perform(
    publisher: &Client,
    subscriber: &Client,
    peer: &str,
    output: &std::path::Path,
) -> Result<Value> {
    publisher
        .capabilities()
        .require(sdk::features::NATIVE_PROGRAMMATIC_MEDIA, options())
        .await?;
    let room = publisher
        .create_conversation(
            vec![peer.parse()?],
            sdk::ConversationPolicy::Persistent,
            mutation("external-create")?,
        )
        .await?
        .wait(options())
        .await?
        .value;
    let participant = room
        .refresh(options())
        .await?
        .participants
        .into_iter()
        .find(|p| p.identity_id.as_str() == peer)
        .ok_or("missing public Participant")?
        .id;
    tokio::fs::create_dir_all(output).await?;
    tokio::fs::write(output.join("message-intent.json"),serde_json::to_vec(&json!({"operation_id":"external-send","conversation_id":room.id(),"message_id":"external-message","to":[participant],"text":"external SDK message"}))?).await?;
    let sent = room
        .send(
            "external-message".parse()?,
            NonEmpty {
                first: participant.clone(),
                rest: vec![],
            },
            "external SDK message".into(),
            mutation("external-send")?,
        )
        .await?;
    let saved = sent.reference_json();
    sent.wait(options()).await?;
    publisher
        .restore(saved, publisher.operation_kinds().message_send)?
        .wait(options())
        .await?;
    let mut invitations = subscriber.incoming_invitations(options());
    let caps = json!({"audio":["g.711-mu"],"video":["vp8"]});
    let session = room
        .invite(
            "external-session".parse()?,
            vec![participant],
            sdk::SessionMedium::VoiceVideo,
            "conference".into(),
            caps.clone(),
            mutation("external-invite")?,
        )
        .await?
        .wait(options())
        .await?
        .value;
    let incoming = invitations
        .next()
        .await?
        .ok_or("missing actual invitation")?;
    if incoming.session.id() != session.id() {
        return Err("wrong incoming Session".into());
    }
    let receiving = incoming.session;
    let before = receiving.refresh(options()).await?;
    receiving
        .accept(caps, before.topology_revision, mutation("external-accept")?)
        .await?
        .wait(options())
        .await?;
    invitations.close().await?;
    let source = session
        .offer_connection(
            "external-source".parse()?,
            NonEmpty {
                first: stream(
                    "external-audio",
                    Medium::Audio,
                    Direction::Sendonly,
                    "g.711-mu",
                )?,
                rest: vec![stream(
                    "external-video",
                    Medium::Video,
                    Direction::Sendonly,
                    "vp8",
                )?],
            },
            mutation("external-source-offer")?,
        )
        .await?
        .wait(options())
        .await?
        .value;
    let target = receiving
        .offer_connection(
            "external-target".parse()?,
            NonEmpty {
                first: stream(
                    "external-monitor",
                    Medium::Audio,
                    Direction::Recvonly,
                    "g.711-mu",
                )?,
                rest: vec![stream(
                    "external-monitor-video",
                    Medium::Video,
                    Direction::Recvonly,
                    "vp8",
                )?],
            },
            mutation("external-target-offer")?,
        )
        .await?
        .wait(options())
        .await?
        .value;
    target
        .subscribe(
            NonEmpty {
                first: sdk::StreamSelector::Stream("external-audio".parse()?),
                rest: vec![sdk::StreamSelector::Stream("external-video".parse()?)],
            },
            sdk::StreamRevision::try_from(1)?,
            mutation("external-subscribe")?,
        )
        .await?
        .wait(options())
        .await?;
    let local = target
        .native_negotiation()?
        .ok_or("no owned target bindings")?;
    let audio = local
        .streams
        .iter()
        .find(|s| {
            s.medium == Medium::Audio
                && matches!(s.direction, Direction::Recvonly | Direction::Sendrecv)
        })
        .ok_or("no local audio sink")?
        .id
        .clone();
    let video = local
        .streams
        .iter()
        .find(|s| s.id.as_str() == "external-video")
        .ok_or("no selected video sink")?
        .id
        .clone();
    let (_, received) = tokio::try_join!(
        send(
            source.stream_by_id("external-audio".parse()?),
            source.stream_by_id("external-video".parse()?)
        ),
        receive(target.stream_by_id(audio), target.stream_by_id(video))
    )?;
    let decoded = decode(output, received.0, received.1).await?;
    let current = session.refresh(options()).await?;
    session
        .end(
            1000,
            "external journey complete".into(),
            current.topology_revision,
            mutation("external-end")?,
        )
        .await?
        .wait(options())
        .await?;
    Ok(
        json!({"message_id":"external-message","conversation_id":room.id(),"session_id":session.id(),"actual_invitation":true,"media":decoded,"session_state":session.refresh(options()).await?.state}),
    )
}
