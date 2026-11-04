//! A physical replacement target uses only the public native client interface.
//! Its signed allocation/answer is separate from actual RTP and controller status.
use super::*;
use connect_client::canonical::{
    pack_sender_clock_datagram, payloads, CanonicalClient, CanonicalClientConfig, CommandScope,
    Envelope, Scope, SenderClockDatagram, SenderClockReport,
};
use std::time::{SystemTime, UNIX_EPOCH};
mod controls;
mod data;
mod encoder;
mod media;
mod owner;
mod selected;
mod terminal;
mod timeline;
mod video;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Source {
    seconds: u16,
    ssrc: u32,
    // The parent preserves identifiable, externally encoded 20ms Opus originals.
    audio_packets: Vec<Vec<u8>>,
    video: Option<video::Video>,
    #[serde(default)]
    owner_control: bool,
    #[serde(default)]
    program_start_unix_us: Option<u64>,
    #[serde(default)]
    program_clock: Option<timeline::ProgrammeClock>,
    #[serde(default)]
    terminal_end_operation: Option<String>,
    #[serde(default)]
    data: Option<data::io::Contract>,
}
impl Source {
    fn validate(&self) -> Result<()> {
        if self.program_start_unix_us.is_some() != self.program_clock.is_some() {
            return Err(
                "original programme requires its monotonic anchor and wall label together".into(),
            );
        }
        if let Some(clock) = &self.program_clock {
            clock.validate()?;
        }
        if let Some(data) = &self.data {
            data.validate()?;
            if !self.owner_control
                || self.seconds != 62
                || self.program_start_unix_us.is_none()
                || self
                    .video
                    .as_ref()
                    .and_then(|video| video.receive_data_stream_id.as_deref())
                    != Some(data.incoming.stream_id.as_str())
                || self.terminal_end_operation.as_ref().is_some_and(|end| {
                    [&data.incoming.operation_id, &data.outgoing.operation_id].contains(&end)
                })
            {
                return Err("physical Data requires the original bounded owner programme and distinct operation keys".into());
            }
        }
        if self
            .terminal_end_operation
            .as_ref()
            .is_some_and(|operation| {
                !video::id(operation)
                    || !self.owner_control
                    || self.video.is_none()
                    || self.seconds != 62
                    || self.program_start_unix_us.is_none()
            })
        {
            return Err(
                "final owner requires the original bounded AV programme and distinct end key"
                    .into(),
            );
        }
        let maximum = if self.video.is_some() { 62 } else { 30 };
        if !(1..=maximum).contains(&self.seconds)
            || self.ssrc == 0
            || self.audio_packets.len() != usize::from(self.seconds) * 50
            || self
                .audio_packets
                .iter()
                .any(|v| v.is_empty() || v.len() > 1168)
        {
            return Err("replacement source exceeds the declared Opus profile".into());
        }
        if let Some(video) = &self.video {
            video.validate(self.seconds, self.ssrc)?;
        }
        Ok(())
    }
}

fn scope_matches(frame: &Envelope, scope: &CommandScope) -> bool {
    [
        (Scope::Conversation, scope.conversation_id.as_deref()),
        (Scope::Session, scope.session_id.as_deref()),
        (Scope::Connection, scope.connection_id.as_deref()),
        (Scope::Operation, scope.operation_id.as_deref()),
    ]
    .iter()
    .all(|(key, expected)| expected.is_some() && frame.scope(*key) == *expected)
}

// Static shape and bounded public error codes only. Never log a signed scope,
// payload, signature, arbitrary reason, or remote extension on rejection.
fn negotiation_refusal(
    frame: &Envelope,
    scope: &CommandScope,
    expected: &str,
    reply: Option<&str>,
) -> Value {
    let code = (frame.kind() == "error")
        .then(|| frame.original()["payload"]["code"].as_u64())
        .flatten()
        .filter(|code| (100..=599).contains(code));
    json!({"phase":"target-negotiation-refused","expected_kind":expected,
        "received_kind":frame.kind(),"scope_matches":scope_matches(frame,scope),
        "has_reply":frame.scope(Scope::Reply).is_some(),
        "reply_matches_expected":frame.scope(Scope::Reply)==reply,
        "error_code":code})
}

async fn negotiation(
    client: &mut CanonicalClient,
    scope: &CommandScope,
    kind: &str,
    reply: Option<&str>,
) -> Result<Envelope> {
    let until = tokio::time::Instant::now() + Duration::from_secs(8);
    tokio::time::timeout_at(until, async {
        for _ in 0..128 {
            let frame = client.receive().await?;
            if frame.kind() == kind {
                if !scope_matches(&frame, scope) || frame.scope(Scope::Reply) != reply {
                    emit(negotiation_refusal(&frame, scope, kind, reply)).await?;
                    return Err("replacement negotiation changed the original root".into());
                }
                return Ok(frame);
            }
            // These already verified unsolicited hints grant no readiness or
            // operation completion. The controller reads its original journal.
            if frame.scope(Scope::Reply).is_some()
                || !matches!(
                    frame.kind(),
                    "operation.updated"
                        | "connection.ready"
                        | "stream.opened"
                        | "stream.closed"
                        | "connection.quality"
                        | "session.started"
                        | "session.ended"
                        | "session.participant.joined"
                        | "session.participant.left"
                        | "recording.vcon-ready"
                )
            {
                emit(negotiation_refusal(&frame, scope, kind, reply)).await?;
                return Err("unexpected replacement negotiation response".into());
            }
        }
        Err("replacement negotiation hint bound exceeded".into())
    })
    .await?
}

fn wire(
    handle: u16,
    datagram: u32,
    seq: u16,
    timestamp: u32,
    ssrc: u32,
    pt: u8,
    bytes: &[u8],
) -> Vec<u8> {
    let mut packet = vec![1, 0];
    packet.extend(handle.to_be_bytes());
    packet.extend(datagram.to_be_bytes());
    packet.extend([0x80, pt]);
    packet.extend(seq.to_be_bytes());
    packet.extend(timestamp.to_be_bytes());
    packet.extend(ssrc.to_be_bytes());
    packet.extend(bytes);
    packet
}
fn ntp(unix_micros: u128) -> u64 {
    let seconds = unix_micros / 1_000_000 + 2_208_988_800;
    ((seconds as u64) << 32) | (((unix_micros % 1_000_000) << 32) / 1_000_000) as u64
}

fn validate_offer(offer_json: &Value, source: &Source) -> Result<()> {
    let offered = offer_json["streams_offered"]
        .as_array()
        .ok_or("replacement offer streams missing")?;
    let audio = offered
        .iter()
        .filter(|s| s["kind"] == "audio")
        .collect::<Vec<_>>();
    let data = offered
        .iter()
        .filter(|stream| stream["kind"] == "data")
        .collect::<Vec<_>>();
    if data.len() > 1 {
        return Err("replacement target offers more than one Data stream".into());
    }
    let protocols = if data.is_empty() {
        json!([])
    } else {
        json!(["binary"])
    };
    if offer_json["capabilities"]["data_protocols"] != protocols {
        return Err("replacement Data protocols differ from the original offer".into());
    }
    if let Some(stream) = data.first() {
        let id = stream["id"]
            .as_str()
            .filter(|id| video::id(id))
            .ok_or("invalid replacement Data identity")?;
        if stream["direction"] != "sendrecv"
            || stream["codec_preferences"] != json!(["binary"])
            || audio.iter().any(|audio| audio["id"] == id)
            || source.video.as_ref().is_some_and(|video| {
                [
                    video.stream_id.as_str(),
                    video.receive_audio_stream_id.as_str(),
                    video.receive_video_stream_id.as_str(),
                ]
                .contains(&id)
                    || video.receive_data_stream_id.as_deref() == Some(id)
            })
        {
            return Err("replacement Data format or identity changed".into());
        }
    }
    if source
        .data
        .as_ref()
        .is_some_and(|contract| data.len() != 1 || data[0]["id"] != contract.outgoing.stream_id)
    {
        return Err("physical Data publication differs from the actual offered resource".into());
    }
    let maximum = (if source.video.is_some() { 3 } else { 1 }) + data.len();
    let receives_data = source
        .video
        .as_ref()
        .is_some_and(|video| video.receive_data_stream_id.is_some());
    if receives_data && data.is_empty() {
        return Err("replacement incoming Data requires its declared binary capability".into());
    }
    let minimum_slots = maximum + usize::from(receives_data);
    if !offer_json["capabilities"]["max_streams_per_connection"]
        .as_u64()
        .is_some_and(|slots| (minimum_slots as u64..=8).contains(&slots))
    {
        return Err("replacement stream capacity cannot hold all original resources".into());
    }
    if offered.len() != maximum
        || audio.len() != 1
        || audio[0]["direction"] != "sendrecv"
        || audio[0]["codec_preferences"] != json!(["opus"])
        || offer_json["capabilities"]["source_clock"] != "rtcp-sr-sdes/1"
        || offer_json["capabilities"]["audio_codecs"]
            != json!([{"name":"opus","params":{"sample_rate":48000,"channels":1}}])
    {
        return Err(
            "replacement target requires its original duplex Opus/source-clock profile".into(),
        );
    }
    if let Some(video) = &source.video {
        let v = offered
            .iter()
            .filter(|s| s["kind"] == "video")
            .collect::<Vec<_>>();
        if v.len() != 2
            || ![
                (video.stream_id.as_str(), "sendonly"),
                (video.receive_video_stream_id.as_str(), "recvonly"),
            ]
            .iter()
            .all(|(id, direction)| {
                v.iter()
                    .filter(|v| {
                        v["id"] == *id
                            && v["direction"] == *direction
                            && v["codec_preferences"] == json!(["vp8"])
                    })
                    .count()
                    == 1
            })
            || audio[0]["id"] == video.stream_id
            || audio[0]["id"] == video.receive_audio_stream_id
            || audio[0]["id"] == video.receive_video_stream_id
            || offer_json["capabilities"]["video_feedback"] != "rtcp-pli-epoch/1"
            || offer_json["capabilities"]["video_codecs"]
                != json!([{"name":"vp8","params":{"sample_rate":90000,"channels":0}}])
        {
            return Err("replacement video differs from the declared bounded AV profile".into());
        }
    }
    Ok(())
}

pub(super) async fn run(
    path: PathBuf,
    command: TransitionCommand,
    root: payloads::OperationUpdated,
    offer: payloads::ConnectionOffer,
    source: Source,
    output: PathBuf,
) -> Result<()> {
    source.validate()?;
    if source.terminal_end_operation.as_deref() == Some(command.operation_id.as_str()) {
        return Err("final physical end reused the replacement operation key".into());
    }
    if source.data.as_ref().is_some_and(|data| {
        [&data.incoming.operation_id, &data.outgoing.operation_id]
            .iter()
            .any(|operation| operation.as_str() == command.operation_id)
    }) {
        return Err("physical Data reused the replacement root key".into());
    }
    // Refuse an expired original program before allocation or any network IO.
    if let Some(clock) = &source.program_clock {
        clock.join(
            source
                .program_start_unix_us
                .ok_or("original wall label missing")?,
            source.seconds,
        )?;
    }
    let offer_json = serde_json::to_value(&offer)?;
    validate_offer(&offer_json, &source)?;
    if let Some(video) = &source.video {
        let connect_client::transitions::TransitionIntent::Replace {
            video: Some(mapping),
            ..
        } = &command.intent
        else {
            return Err("video source requires explicit original replacement mapping".into());
        };
        if mapping.publications.len() != 1
            || mapping.receivers.len() != 1
            || mapping.publications[0].replacement != video.stream_id
            || mapping.receivers[0].replacement != video.receive_video_stream_id
        {
            return Err("video source differs from original replacement mapping".into());
        }
    }
    let parent = output
        .parent()
        .ok_or("private output directory required")?
        .canonicalize()?;
    if !parent.components().any(|v| v.as_os_str() == ".local") {
        return Err("replacement observation must remain private".into());
    }
    let mut file = std::fs::OpenOptions::new();
    file.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        file.mode(0o600);
    }
    let mut file = tokio::fs::File::from_std(file.open(&output)?);
    let config = CanonicalClientConfig::load(&path)?;
    let device = config.device_id.clone();
    let mut client = CanonicalClient::open(config).await?;
    let mut media = media::Observation::default();
    let mut phase = "allocation";
    let mut negotiated = Vec::<Value>::new();
    let mut owner_ready = false;
    let mut stop_requested = false;
    let mut terminal_end_receipt = None;
    let mut data_exchange = None;
    let mut encoder = None;
    let work = async {
        phase = "live-video-source";
        if let Some(video) = &source.video { encoder = Some(encoder::Encoder::open(&video.program).await?); }
        phase = "allocation";
        client
            .allocate_native_replacement_target(&command, &root, offer.clone())
            .await?;
        emit(json!({"phase":"target-allocated"})).await?;
        phase = "media-start";
        let (request, scope) = client
            .begin_native_replacement_media(&command, &root, offer)
            .await?;
        phase = "media-answer";
        let answer = negotiation(&mut client, &scope, "connection.answer", Some(&request)).await?;
        let selected = selected::own(&answer.original()["payload"], &offer_json)?;
        let audio = selected.iter().find(|s|s.raw["kind"]=="audio").ok_or("original audio binding missing")?.clone();
        audio.pt.ok_or("original audio RTP payload type missing")?;
        let video_binding=selected.iter().find(|s| source.video.as_ref().is_some_and(|v|s.id == v.stream_id)).cloned();
        phase = "parameter-callback";
        let callback = negotiation(&mut client, &scope, "connection.offer", None).await?;
        let payload = &callback.original()["payload"];
        if selected::own(payload,&offer_json)? != selected
            || payload["by_participant"] != answer.original()["payload"]["by_participant"] {
            return Err("replacement callback changed negotiated resources".into());
        }
        client.answer_connection_offer(&callback, selected::answer(payload,&offer_json["capabilities"])?, &scope).await?;
        negotiated=selected.iter().map(|s|s.raw.clone()).collect();
        if let Some(contract) = &source.data {
            data_exchange = Some(data::io::Exchange::new(contract.clone(), &scope, &command.operation_id,
                answer.original()["payload"]["by_participant"].as_str().ok_or("physical Data admitted participant missing")?)?);
            client.start_signaling_reader()?;
        }
        emit(json!({"phase":"target-negotiated","media_ready_inferred":false})).await?;
        phase = "physical-media";
        let io = if source.video.is_some() { media::Io::Video(client.claim_native_video_datagrams()?) }
            else { media::Io::Audio(client.claim_native_datagrams()?) };
        let mut pump = media::Pump::new(io, &source, &audio, video_binding.as_ref(), &mut media, encoder.as_mut())?;
        let start = pump.start;
        let until = pump.until;
        let mut root_revision = root.revision;
        let mut next_poll = start + Duration::from_millis(50);
        let mut next_owner = start;
        while tokio::time::Instant::now() < until {
            if source.owner_control && tokio::time::Instant::now() >= next_owner {
                phase = "physical-owner-control";
                if pump.joining_stop_read(owner::stop(&output, &command.operation_id)).await?? {
                    stop_requested = true;
                    break;
                }
                if tokio::time::Instant::now() >= until { break; }
                if let Some(data) = &mut data_exchange {
                    phase = "physical-data-exchange";
                    let root_observed = pump.observation.root_observed;
                    pump.during(data.tick(&mut client, root_observed, start, &output)).await??;
                    phase = "physical-owner-control";
                }
                if !owner_ready && pump.observation.sent > 0
                    && (source.video.is_none() || (pump.observation.video_open && pump.observation.sent_video > 0)) {
                    pump.during(owner::ready(&output, &command.operation_id,
                        scope.connection_id.as_deref().ok_or("replacement candidate missing")?,
                        &negotiated, source.video.is_some())).await??;
                    owner_ready = true;
                }
                next_owner = tokio::time::Instant::now() + Duration::from_millis(20);
                phase = "physical-media";
            }
            if let Some(video) = &source.video {
                if !pump.observation.video_open && tokio::time::Instant::now() >= next_poll {
                    phase = "original-completion-observation";
                    let pending = controls::completion(&mut client, &command, &root,
                        scope.connection_id.as_deref().ok_or("replacement scope missing candidate")?,
                        &device, &selected, &mut root_revision);
                    if let Some(admission) = pump.during(tokio::time::timeout_at(until, pending)).await??? {
                        let receiving = selected.iter().find(|binding| binding.id == video.receive_video_stream_id)
                            .ok_or("original receiving video binding missing")?;
                        pump.observe_root_completion(receiving.handle)?;
                        // Video is already transferred by the original activation. Data keeps
                        // distinct authorization and commands while the video pump progresses.
                        pump.observation.video_open = true;
                        phase = "original-post-commit-data-controls";
                        let incoming = pump.during(tokio::time::timeout_at(until,
                            controls::open(&mut client, &scope, &root, &admission, video,
                                &selected, &offer_json["capabilities"], &output))).await???;
                        negotiated.extend(incoming.into_iter().map(|binding| binding.raw));
                        pump.during(emit(json!({"phase":"target-video-transfer-admitted",
                            "original_root_observed":true,"decoded_media_inferred":false}))).await??;
                    }
                    next_poll = tokio::time::Instant::now() + Duration::from_millis(50);
                    phase = "physical-media";
                }
            }
            let poll = if source.video.is_some() && !pump.observation.video_open { next_poll } else { until };
            let owner = if source.owner_control { next_owner } else { until };
            pump.step(poll.min(owner)).await?;
        }
        if source.video.is_some() && (!pump.observation.video_open || pump.observation.sent_video == 0) {
            return Err("replacement AV source ended without original admitted video transfer and original video packets".into());
        }
        // All packet futures run inline and have settled. Drop the exclusive IO
        // owner before retaining only the original final control scope.
        drop(pump);
        if data_exchange.as_ref().is_some_and(|data| !data.completed()) {
            return Err("physical owner ended without its original exact Data exchange".into());
        }
        if let Some(operation) = &source.terminal_end_operation {
            phase = "original-final-owner-end";
            if stop_requested {
                return Err("final physical owner stopped before its original end; no teardown pass".into());
            }
            // No media send/receive occurs after the loop's original boundary.
            // Keep only control ownership under that same programme's5s cutoff.
            terminal_end_receipt = Some(terminal::io::run(
                &mut client, &scope, operation, start, &output).await?);
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    let encoder_join = match &mut encoder {
        Some(encoder) => encoder.stop_and_join().await.map(Some),
        None => Ok(None),
    };
    // Preserve negative observations too, and always join this physical owner.
    let diagnostic = if let Err(error) = &work {
        // The caller owns private stdout/logs, just as it owns media output.
        // Emit before shutdown can retire the original allocation.
        emit(json!({"phase":"target-work-stopped","failure_stage":phase,"message":error.to_string()})).await
    } else {
        Ok(())
    };
    let media::Observation {
        mut rows,
        sent,
        skipped,
        captured_bytes,
        sent_video,
        skipped_video,
        video_open,
        root_observed,
        root_observed_programme_us,
        program_origin,
        source_start_audio_index,
        source_start_video_index,
        control_packet_events,
        video_feedback_requests,
    } = media;
    // Epoch notices can arrive after datagrams. Preserve physical read order
    // in the evidence without renewing an arrival or RTP timestamp.
    rows.sort_by_key(|row| row["elapsed_us"].as_u64().unwrap_or(u64::MAX));
    let shutdown = client.shutdown().await;
    let result = json!({"format":"connect/native-replacement-peer-observation/1",
        "passed":work.is_ok() && shutdown.is_ok() && encoder_join.as_ref().is_ok_and(|joined| joined.as_ref().is_none_or(|joined| joined.joined && (joined.clean || joined.cancelled))),"encoder_process":encoder_join.as_ref().ok().and_then(|joined| joined.as_ref()),"sent_audio_packets":sent,"skipped_original_frames":skipped,
        "captured_bytes":captured_bytes,"received":rows,"packet_events_during_control":control_packet_events,"video_feedback_requests":video_feedback_requests,"original_client_joined":shutdown.is_ok(),
        "failure_stage":if work.is_err(){Some(phase)}else{None},
        "operation_completed_inferred":false,"decoded_media_verified":false,
        "video_enabled":source.video.is_some(),"original_root_completed_observed":root_observed,
        "root_completed_observed_programme_us":root_observed_programme_us,
        "video_transfer_admitted":video_open,"sent_video_packets":sent_video,"skipped_original_video_frames":skipped_video,
        "owner_control":source.owner_control,"owner_ready":owner_ready,"stop_requested":stop_requested,
        "program_clock":source.program_clock,"program_start_unix_us":program_origin,"source_start_audio_index":source_start_audio_index,"source_start_video_index":source_start_video_index,
        "terminal_end_requested":source.terminal_end_operation.is_some(),
        "terminal_end_receipt":terminal_end_receipt,
        "data_exchange_receipt":data_exchange.as_ref().and_then(|data| data.receipt()),
        "negotiated_streams":negotiated});
    file.write_all(&serde_json::to_vec(&result)?).await?;
    file.sync_all().await?;
    shutdown?;
    diagnostic?;
    work?;
    if encoder_join?.is_some_and(|joined| !joined.clean && !joined.cancelled) {
        return Err("encoder stopped without clean completion".into());
    }
    emit(json!({"phase":"target-joined","sent_audio_packets":sent,"captured_bytes":captured_bytes}))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn final_end_is_only_for_the_full_original_live_av_owner() {
        let make = || Source {
            seconds: 62,
            ssrc: 7,
            audio_packets: vec![vec![1]; 3100],
            owner_control: true,
            program_start_unix_us: Some(1),
            program_clock: Some(
                serde_json::from_value(
                    json!({"kind":"linux-monotonic-v1","domain":"fixture","origin_ns":"1"}),
                )
                .unwrap(),
            ),
            terminal_end_operation: Some("original-final-end".into()),
            data: None,
            video: Some(video::Video {
                stream_id: "camera".into(),
                receive_audio_stream_id: "remote-audio".into(),
                receive_video_stream_id: "remote-camera".into(),
                receive_data_stream_id: None,
                ssrc: 8,
                program: encoder::test_program(1860),
            }),
        };
        make().validate().unwrap();
        for change in 0..6 {
            let mut source = make();
            match change {
                0 => source.owner_control = false,
                1 => source.video = None,
                2 => source.seconds = 61,
                3 => source.program_start_unix_us = None,
                4 => source.terminal_end_operation = Some("foreign:id".into()),
                _ => source.terminal_end_operation = Some(String::new()),
            }
            assert!(source.validate().is_err());
        }
    }
    #[test]
    fn data_offer_profile_preserves_binary_identity_capacity_and_av_source_clocks() {
        let mut source = Source {
            seconds: 1,
            ssrc: 7,
            audio_packets: vec![vec![1]; 50],
            video: None,
            owner_control: false,
            program_start_unix_us: None,
            program_clock: None,
            terminal_end_operation: None,
            data: None,
        };
        let offer = json!({"capabilities":{"audio_codecs":[{"name":"opus","params":{"sample_rate":48000,"channels":1}}],
            "video_codecs":[],"data_protocols":["binary"],"max_streams_per_connection":2,"source_clock":"rtcp-sr-sdes/1"},
            "streams_offered":[{"id":"audio","kind":"audio","direction":"sendrecv","codec_preferences":["opus"]},
                {"id":"data","kind":"data","direction":"sendrecv","codec_preferences":["binary"]}]});
        source.validate().unwrap();
        validate_offer(&offer, &source).unwrap();
        for change in 0..5 {
            let mut wrong = offer.clone();
            match change {
                0 => wrong["capabilities"]["data_protocols"] = json!([]),
                1 => wrong["capabilities"]["max_streams_per_connection"] = json!(1),
                2 => wrong["streams_offered"][1]["id"] = json!("audio"),
                3 => wrong["streams_offered"][1]["codec_preferences"] = json!(["json"]),
                _ => wrong["streams_offered"][1]["direction"] = json!("sendonly"),
            }
            assert!(validate_offer(&wrong, &source).is_err());
        }
        source.video = Some(video::Video {
            stream_id: "camera".into(),
            receive_audio_stream_id: "remote-audio".into(),
            receive_video_stream_id: "remote-camera".into(),
            receive_data_stream_id: None,
            ssrc: 8,
            program: encoder::test_program(30),
        });
        source.validate().unwrap();
        let mut av = offer;
        av["capabilities"]["video_feedback"] = json!("rtcp-pli-epoch/1");
        av["capabilities"]["video_codecs"] =
            json!([{"name":"vp8","params":{"sample_rate":90000,"channels":0}}]);
        av["capabilities"]["max_streams_per_connection"] = json!(4);
        av["streams_offered"].as_array_mut().unwrap().extend([json!({"id":"camera","kind":"video","direction":"sendonly","codec_preferences":["vp8"]}), json!({"id":"remote-camera","kind":"video","direction":"recvonly","codec_preferences":["vp8"]})]);
        validate_offer(&av, &source).unwrap();
        av["capabilities"]["max_streams_per_connection"] = json!(3);
        assert!(validate_offer(&av, &source).is_err());
        av["capabilities"]["max_streams_per_connection"] = json!(4);
        av["streams_offered"][1]["id"] = json!("remote-camera");
        assert!(validate_offer(&av, &source).is_err());
        av["streams_offered"][1]["id"] = json!("data");
        source.video.as_mut().unwrap().receive_data_stream_id = Some("remote-data".into());
        source.validate().unwrap();
        assert!(validate_offer(&av, &source).is_err());
        av["capabilities"]["max_streams_per_connection"] = json!(5);
        validate_offer(&av, &source).unwrap();
        source.video.as_mut().unwrap().receive_data_stream_id = Some("camera".into());
        assert!(source.validate().is_err());
    }

    #[test]
    fn invalid_original_source_bounds_refuse_before_a_physical_peer_is_opened() {
        for source in [
            Source {
                seconds: 0,
                ssrc: 1,
                audio_packets: vec![],
                video: None,
                owner_control: false,
                program_start_unix_us: None,
                program_clock: None,
                terminal_end_operation: None,
                data: None,
            },
            Source {
                seconds: 31,
                ssrc: 1,
                audio_packets: vec![vec![1]; 1550],
                video: None,
                owner_control: false,
                program_start_unix_us: None,
                program_clock: None,
                terminal_end_operation: None,
                data: None,
            },
            Source {
                seconds: 1,
                ssrc: 0,
                audio_packets: vec![vec![1]; 50],
                video: None,
                owner_control: false,
                program_start_unix_us: None,
                program_clock: None,
                terminal_end_operation: None,
                data: None,
            },
            Source {
                seconds: 1,
                ssrc: 1,
                audio_packets: vec![vec![1]; 49],
                video: None,
                owner_control: false,
                program_start_unix_us: None,
                program_clock: None,
                terminal_end_operation: None,
                data: None,
            },
            Source {
                seconds: 1,
                ssrc: 1,
                audio_packets: vec![vec![1; 1169]; 50],
                video: None,
                owner_control: false,
                program_start_unix_us: None,
                program_clock: None,
                terminal_end_operation: None,
                data: None,
            },
        ] {
            assert!(source.validate().is_err());
        }
    }

    #[test]
    fn unoffered_codec_direction_and_hop_handle_are_not_media_readiness() {
        let good = json!({"substrate_setup":{"streams":[{"strm_id":"original-audio",
            "kind":"audio","direction":"sendrecv","stream_local_id":7,
            "codec":{"name":"opus","params":{"payload_type":111,"sample_rate":48000,"channels":1}}}]}});
        let offer = json!({"streams_offered":[{"id":"original-audio","kind":"audio"}]});
        assert!(selected::own(&good, &offer).is_ok());
        for (field, value) in [
            ("kind", json!("video")),
            ("direction", json!("sendonly")),
            ("stream_local_id", json!(0)),
            ("codec", json!({"name":"pcmu","params":{"payload_type":0}})),
        ] {
            let mut changed = good.clone();
            changed["substrate_setup"]["streams"][0][field] = value;
            assert!(selected::own(&changed, &offer).is_err());
        }
    }
}
