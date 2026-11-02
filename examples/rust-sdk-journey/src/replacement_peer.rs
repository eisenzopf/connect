//! A physical replacement target uses only the public native client interface.
//! Its signed allocation/answer is separate from actual RTP and controller status.
use super::*;
use connect_client::canonical::{
    pack_sender_clock_datagram, payloads, CanonicalClient, CanonicalClientConfig, CommandScope,
    Envelope, Scope, SenderClockDatagram, SenderClockReport,
};
use std::time::{SystemTime, UNIX_EPOCH};
mod controls;
mod owner;
mod selected;
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
}
impl Source {
    fn validate(&self) -> Result<()> {
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

pub(super) async fn run(
    path: PathBuf,
    command: TransitionCommand,
    root: payloads::OperationUpdated,
    offer: payloads::ConnectionOffer,
    source: Source,
    output: PathBuf,
) -> Result<()> {
    source.validate()?;
    // Refuse an expired original program before allocation or any network IO.
    timeline::Program::at(
        source.program_start_unix_us,
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros(),
        source.seconds,
    )?;
    let offer_json = serde_json::to_value(&offer)?;
    let offered = offer_json["streams_offered"]
        .as_array()
        .ok_or("replacement offer streams missing")?;
    let audio = offered
        .iter()
        .filter(|s| s["kind"] == "audio")
        .collect::<Vec<_>>();
    let maximum = if source.video.is_some() { 2 } else { 1 };
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
        if v.len() != 1
            || v[0]["id"] != video.stream_id
            || v[0]["direction"] != "sendrecv"
            || v[0]["codec_preferences"] != json!(["vp8"])
            || audio[0]["id"] == video.stream_id
            || audio[0]["id"] == video.receive_audio_stream_id
            || audio[0]["id"] == video.receive_video_stream_id
            || offer_json["capabilities"]["video_codecs"]
                != json!([{"name":"vp8","params":{"sample_rate":90000,"channels":0}}])
            || !offer_json["capabilities"]["max_streams_per_connection"]
                .as_u64()
                .is_some_and(|v| (3..=8).contains(&v))
        {
            return Err("replacement video differs from the declared bounded AV profile".into());
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
    let mut rows = vec![];
    let mut sent = 0u32;
    let mut skipped = 0u32;
    let mut captured_bytes = 0usize;
    let mut phase = "allocation";
    let mut sent_video = 0u32;
    let mut skipped_video = 0u32;
    let mut video_open = false;
    let mut root_observed = false;
    let mut negotiated = Vec::<Value>::new();
    let mut owner_ready = false;
    let mut stop_requested = false;
    let mut program_origin = None;
    let mut source_start_audio_index = 0;
    let mut source_start_video_index = 0;
    let work = async {
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
        let handle=audio.handle;let pt=audio.pt;
        let video_binding=selected.iter().find(|s|s.raw["kind"]=="video").cloned();
        phase = "parameter-callback";
        let callback = negotiation(&mut client, &scope, "connection.offer", None).await?;
        let payload = &callback.original()["payload"];
        if selected::own(payload,&offer_json)? != selected
            || payload["by_participant"] != answer.original()["payload"]["by_participant"] {
            return Err("replacement callback changed negotiated resources".into());
        }
        client.answer_connection_offer(&callback, selected::answer(payload,&offer_json["capabilities"])?, &scope).await?;
        negotiated=selected.iter().map(|s|s.raw.clone()).collect();
        emit(json!({"phase":"target-negotiated","media_ready_inferred":false})).await?;
        phase = "physical-media";
        let anchor = tokio::time::Instant::now();
        let program = timeline::Program::at(source.program_start_unix_us,
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros(), source.seconds)?;
        let start = anchor.checked_sub(program.elapsed).ok_or("original program monotonic origin out of bounds")?;
        let unix = program.unix;
        program_origin = Some(unix);
        let until = start + Duration::from_secs(u64::from(source.seconds));
        let mut index = program.audio_index;
        let mut video_index = program.video_index;
        source_start_audio_index = index;
        source_start_video_index = video_index;
        let mut datagram = 0u32;
        let mut octets = 0u32;
        let mut rtp_seq = 0u16;
        let mut video_seq = 0u16;
        let mut video_octets = 0u32;
        let mut first_video_key = true;
        let mut last_video_clock = None;
        let mut root_revision = root.revision;
        let mut next_poll = start + Duration::from_millis(50);
        let mut next_owner = start;
        while tokio::time::Instant::now() < until {
            if source.owner_control && tokio::time::Instant::now() >= next_owner {
                phase = "physical-owner-control";
                if owner::stop(&output, &command.operation_id).await? {
                    stop_requested = true;
                    break;
                }
                if !owner_ready && sent > 0 && (source.video.is_none() || (video_open && sent_video > 0)) {
                    owner::ready(&output, &command.operation_id,
                        scope.connection_id.as_deref().ok_or("replacement candidate missing")?,
                        &negotiated, source.video.is_some()).await?;
                    owner_ready = true;
                }
                next_owner = tokio::time::Instant::now() + Duration::from_millis(20);
                phase = "physical-media";
            }
            let next_audio = if index<source.audio_packets.len() {start+Duration::from_millis(index as u64*20)} else {until};
            let next_video = source.video.as_ref().filter(|v|video_index<v.frames.len())
                .map(|_|start+Duration::from_micros(video_index as u64*1_000_000/30)).unwrap_or(until);
            let poll = if source.video.is_some() && !video_open {next_poll} else {until};
            let owner = if source.owner_control { next_owner } else { until };
            let next=next_audio.min(next_video).min(poll).min(owner).min(until);
            match timeline::receive_before_tick(next, client.receive_native_media_datagram()).await {
                Some(packet) => {
                    let packet=packet?;captured_bytes+=packet.len();
                    let maximum_rows=if source.video.is_some() {16000} else {4000};
                    if rows.len()>=maximum_rows || captured_bytes>16*1024*1024 {
                        return Err("replacement receive observation bound exceeded".into());
                    }
                    rows.push(json!({"elapsed_us":start.elapsed().as_micros(),"datagram":packet,
                        "original_root_completed_observed":root_observed}));
                }
                None => {
                    if let Some(video)=&source.video {
                        if !video_open && tokio::time::Instant::now()>=next_poll {
                            phase="original-completion-observation";
                            if let Some(admission)=controls::completion(&mut client,&command,&root,
                                scope.connection_id.as_deref().ok_or("replacement scope missing candidate")?,&device,&selected,&mut root_revision).await? {
                                root_observed=true;phase="original-post-commit-video-controls";
                                let incoming=controls::open(&mut client,&scope,&root,&admission,video,&selected,&offer_json["capabilities"],&output).await?;
                                negotiated.push(incoming.raw);video_open=true;
                                emit(json!({"phase":"target-video-controls-completed","original_root_observed":true,"decoded_media_inferred":false})).await?;
                            }
                            next_poll=tokio::time::Instant::now()+Duration::from_millis(50);phase="physical-media";
                        }
                    }
                    if index<source.audio_packets.len() && tokio::time::Instant::now()>=next_audio {
                        let elapsed=start.elapsed().as_millis();let timestamp=index as u32*960;
                        if elapsed.saturating_sub(index as u128*20)>=200 {skipped+=1;} else {
                            let bytes=&source.audio_packets[index];
                            client.send_native_media_datagram(&wire(handle,datagram,rtp_seq,timestamp,source.ssrc,pt,bytes))?;
                            datagram+=1;rtp_seq=rtp_seq.wrapping_add(1);sent+=1;octets+=bytes.len() as u32;
                            if sent==1 || index%25==0 {
                                client.send_native_media_datagram(&pack_sender_clock_datagram(&SenderClockDatagram {
                                    stream_local_id:handle,seq:datagram,report:SenderClockReport {ssrc:source.ssrc,rtp:timestamp,
                                        ntp:ntp(unix+index as u128*20_000),packets:sent,octets,cname:"g6-replacement-target".into()}})?)?;
                                datagram+=1;
                            }
                        }
                        index+=1;
                    }
                    if let (Some(video),Some(binding))=(&source.video,&video_binding) {
                        if video_index<video.frames.len() && tokio::time::Instant::now()>=next_video {
                            let frame=&video.frames[video_index];let original_us=video_index as u128*1_000_000/30;
                            if !video_open || start.elapsed().as_micros().saturating_sub(original_us)>=200_000
                                || (first_video_key && !video::key(frame)) {skipped_video+=1;} else {
                                let timestamp=video_index as u32*3000;let mut complete=true;
                                for packet in video::packets(binding,video.ssrc,timestamp,&mut video_seq,&mut datagram,frame) {
                                    if start.elapsed().as_micros().saturating_sub(original_us)>=200_000 {complete=false;skipped_video+=1;break;}
                                    client.send_native_media_datagram(&packet)?;sent_video+=1;video_octets+=(packet.len()-20) as u32;
                                }
                                if complete {
                                    first_video_key=false;
                                    if last_video_clock.is_none_or(|last|video_index-last>=15) {
                                        client.send_native_media_datagram(&pack_sender_clock_datagram(&SenderClockDatagram {
                                            stream_local_id:binding.handle,seq:datagram,report:SenderClockReport {ssrc:video.ssrc,rtp:timestamp,
                                                ntp:ntp(unix+original_us),packets:sent_video,octets:video_octets,cname:"g6-replacement-target".into()}})?)?;
                                        datagram+=1;last_video_clock=Some(video_index);
                                    }
                                }
                            }
                            video_index+=1;
                        }
                    }
                }
            }
        }
        if source.video.is_some() && (!video_open || sent_video==0) {return Err("replacement AV source ended without original completed video controls and original video packets".into());}
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    }
    .await;
    // Preserve negative observations too, and always join this physical owner.
    let diagnostic = if let Err(error) = &work {
        // The caller owns private stdout/logs, just as it owns media output.
        // Emit before shutdown can retire the original allocation.
        emit(json!({"phase":"target-work-stopped","failure_stage":phase,"message":error.to_string()})).await
    } else {
        Ok(())
    };
    let shutdown = client.shutdown().await;
    let result = json!({"format":"connect/native-replacement-peer-observation/1",
        "passed":work.is_ok() && shutdown.is_ok(),"sent_audio_packets":sent,"skipped_original_frames":skipped,
        "captured_bytes":captured_bytes,"received":rows,"original_client_joined":shutdown.is_ok(),
        "failure_stage":if work.is_err(){Some(phase)}else{None},
        "operation_completed_inferred":false,"decoded_media_verified":false,
        "video_enabled":source.video.is_some(),"original_root_completed_observed":root_observed,
        "video_controls_completed":video_open,"sent_video_packets":sent_video,"skipped_original_video_frames":skipped_video,
        "owner_control":source.owner_control,"owner_ready":owner_ready,"stop_requested":stop_requested,
        "program_start_unix_us":program_origin,"source_start_audio_index":source_start_audio_index,"source_start_video_index":source_start_video_index,
        "negotiated_streams":negotiated});
    file.write_all(&serde_json::to_vec(&result)?).await?;
    file.sync_all().await?;
    shutdown?;
    diagnostic?;
    work?;
    emit(json!({"phase":"target-joined","sent_audio_packets":sent,"captured_bytes":captured_bytes}))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

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
            },
            Source {
                seconds: 31,
                ssrc: 1,
                audio_packets: vec![vec![1]; 1550],
                video: None,
                owner_control: false,
                program_start_unix_us: None,
            },
            Source {
                seconds: 1,
                ssrc: 0,
                audio_packets: vec![vec![1]; 50],
                video: None,
                owner_control: false,
                program_start_unix_us: None,
            },
            Source {
                seconds: 1,
                ssrc: 1,
                audio_packets: vec![vec![1]; 49],
                video: None,
                owner_control: false,
                program_start_unix_us: None,
            },
            Source {
                seconds: 1,
                ssrc: 1,
                audio_packets: vec![vec![1; 1169]; 50],
                video: None,
                owner_control: false,
                program_start_unix_us: None,
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
