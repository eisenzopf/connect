//! A physical replacement target uses only the public native client interface.
//! Its signed allocation/answer is separate from actual RTP and controller status.
use super::*;
use connect_client::canonical::{
    pack_sender_clock_datagram, payloads, CanonicalClient, CanonicalClientConfig, CommandScope,
    Envelope, Scope, SenderClockDatagram, SenderClockReport,
};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Source {
    seconds: u16,
    ssrc: u32,
    // The parent preserves identifiable, externally encoded 20ms Opus originals.
    audio_packets: Vec<Vec<u8>>,
}
impl Source {
    fn validate(&self) -> Result<()> {
        if !(1..=30).contains(&self.seconds)
            || self.ssrc == 0
            || self.audio_packets.len() != usize::from(self.seconds) * 50
            || self
                .audio_packets
                .iter()
                .any(|v| v.is_empty() || v.len() > 1168)
        {
            return Err("replacement source exceeds the declared Opus profile".into());
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

fn stream(payload: &Value) -> Result<(u16, u8, Value)> {
    let streams = payload["substrate_setup"]["streams"]
        .as_array()
        .filter(|v| v.len() == 1)
        .ok_or("replacement requires exactly one negotiated audio stream")?;
    let s = &streams[0];
    let handle = s["stream_local_id"]
        .as_u64()
        .and_then(|v| u16::try_from(v).ok())
        .filter(|v| *v != 0)
        .ok_or("invalid replacement stream handle")?;
    let pt = s["codec"]["params"]["payload_type"]
        .as_u64()
        .and_then(|v| u8::try_from(v).ok())
        .filter(|v| *v < 128)
        .ok_or("invalid replacement payload type")?;
    if s["kind"] != "audio" || s["direction"] != "sendrecv" || s["codec"]["name"] != "opus" {
        return Err("replacement negotiation changed the offered medium or codec".into());
    }
    Ok((handle, pt, s.clone()))
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
    let offer_json = serde_json::to_value(&offer)?;
    let offered = offer_json["streams_offered"]
        .as_array()
        .filter(|v| v.len() == 1)
        .ok_or("one original replacement audio stream required")?;
    if offered[0]["kind"] != "audio"
        || offered[0]["direction"] != "sendrecv"
        || offered[0]["codec_preferences"] != json!(["opus"])
        || offer_json["capabilities"]["source_clock"] != "rtcp-sr-sdes/1"
        || offer_json["capabilities"]["audio_codecs"]
            != json!([{"name":"opus","params":{"sample_rate":48000,"channels":1}}])
    {
        return Err("replacement target requires explicit Opus/source-clock capability".into());
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
    let mut client = CanonicalClient::open(config).await?;
    let mut rows = vec![];
    let mut sent = 0u32;
    let mut skipped = 0u32;
    let mut captured_bytes = 0usize;
    let mut phase = "allocation";
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
        let (handle, pt, selected) = stream(&answer.original()["payload"])?;
        if selected["strm_id"] != offered[0]["id"] {
            return Err("replacement stream ID changed".into());
        }
        phase = "parameter-callback";
        let callback = negotiation(&mut client, &scope, "connection.offer", None).await?;
        let p = &callback.original()["payload"];
        let (callback_handle, callback_pt, callback_stream) = stream(p)?;
        if callback_handle != handle
            || callback_pt != pt
            || callback_stream != selected
            || p["by_participant"] != answer.original()["payload"]["by_participant"]
        {
            return Err("replacement parameter callback changed negotiated resources".into());
        }
        let response = json!({"by_participant":p["by_participant"],"substrate":"quic",
            "capabilities":offer_json["capabilities"],"streams_answered":[{"id":selected["strm_id"],
                "kind":"audio","direction":"sendrecv","codec":selected["codec"]}],
            "substrate_setup":p["substrate_setup"]});
        client
            .answer_connection_offer(&callback, serde_json::from_value(response)?, &scope)
            .await?;
        emit(json!({"phase":"target-negotiated","media_ready_inferred":false})).await?;
        phase = "physical-media";
        let start = tokio::time::Instant::now();
        let unix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros();
        let until = start + Duration::from_secs(u64::from(source.seconds));
        let mut index = 0usize;
        let mut datagram = 0u32;
        let mut octets = 0u32;
        let mut rtp_seq = 0u16;
        while tokio::time::Instant::now() < until {
            let next = (start + Duration::from_millis(index as u64 * 20)).min(until);
            match tokio::time::timeout_at(next, client.receive_native_media_datagram()).await {
                Ok(packet) => {
                    let packet = packet?;
                    captured_bytes += packet.len();
                    if rows.len() >= 4000 || captured_bytes > 16 * 1024 * 1024 {
                        return Err("replacement receive observation bound exceeded".into());
                    }
                    rows.push(json!({"elapsed_us":start.elapsed().as_micros(),"datagram":packet}));
                }
                Err(_) => {
                    if index >= source.audio_packets.len() {
                        break;
                    }
                    let elapsed = start.elapsed().as_millis();
                    let timestamp = index as u32 * 960;
                    if elapsed.saturating_sub(index as u128 * 20) >= 200 {
                        skipped += 1;
                        index += 1;
                        continue;
                    }
                    let bytes = &source.audio_packets[index];
                    client.send_native_media_datagram(&wire(
                        handle,
                        datagram,
                        rtp_seq,
                        timestamp,
                        source.ssrc,
                        pt,
                        bytes,
                    ))?;
                    datagram += 1;
                    rtp_seq = rtp_seq.wrapping_add(1);
                    sent += 1;
                    octets += bytes.len() as u32;
                    if index % 25 == 0 {
                        let report = SenderClockDatagram {
                            stream_local_id: handle,
                            seq: datagram,
                            report: SenderClockReport {
                                ssrc: source.ssrc,
                                rtp: timestamp,
                                ntp: ntp(unix + index as u128 * 20_000),
                                packets: sent,
                                octets,
                                cname: "g6-replacement-target".into(),
                            },
                        };
                        let packet = pack_sender_clock_datagram(&report)?;
                        client.send_native_media_datagram(&packet)?;
                        datagram += 1;
                    }
                    index += 1;
                }
            }
        }
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
        "operation_completed_inferred":false,"decoded_media_verified":false});
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
            },
            Source {
                seconds: 31,
                ssrc: 1,
                audio_packets: vec![vec![1]; 1550],
            },
            Source {
                seconds: 1,
                ssrc: 0,
                audio_packets: vec![vec![1]; 50],
            },
            Source {
                seconds: 1,
                ssrc: 1,
                audio_packets: vec![vec![1]; 49],
            },
            Source {
                seconds: 1,
                ssrc: 1,
                audio_packets: vec![vec![1; 1169]; 50],
            },
        ] {
            assert!(source.validate().is_err());
        }
    }

    #[test]
    fn unoffered_codec_direction_and_hop_handle_are_not_media_readiness() {
        let good = json!({"substrate_setup":{"streams":[{"strm_id":"original-audio",
            "kind":"audio","direction":"sendrecv","stream_local_id":7,
            "codec":{"name":"opus","params":{"payload_type":111}}}]}});
        assert!(stream(&good).is_ok());
        for (field, value) in [
            ("kind", json!("video")),
            ("direction", json!("sendonly")),
            ("stream_local_id", json!(0)),
            ("codec", json!({"name":"pcmu","params":{"payload_type":0}})),
        ] {
            let mut changed = good.clone();
            changed["substrate_setup"]["streams"][0][field] = value;
            assert!(stream(&changed).is_err());
        }
    }
}
