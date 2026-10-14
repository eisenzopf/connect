//! Capture/encode outside Connect; participate through the public native SDK.
mod audio;
mod decode_continuity;
mod markers;
mod pacing;
mod playout;
mod preview;
mod report;
use anyhow::{bail, Context, Result};
use clap::Parser;
use connect_client::{ClientConfig, ConnectClient, NativeAvFrame, VideoFrameAssembler};
use connect_native_video::{read_frame, read_header, write_frame_at, write_header};
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::{io::AsyncWriteExt, process::Command, sync::mpsc};

#[derive(Parser)]
#[command(
    version,
    about = "Native UCTP/QUIC VP8 participant using external FFmpeg codecs"
)]
struct Arguments {
    /// Explicit external FFmpeg codec/capture executable.
    #[arg(long, default_value = "ffmpeg")]
    ffmpeg: PathBuf,
    /// Protected native client configuration; no credentials in arguments.
    #[arg(long)]
    config: PathBuf,
    #[arg(long)]
    conversation: String,
    #[arg(long)]
    session: String,
    #[arg(long)]
    peer: String,
    /// Existing VP8/640×360 IVF, generated test pattern, or explicit macOS camera ID.
    #[arg(long, default_value = "test")]
    input: String,
    /// Generated-source visual identity for independent fixture observers (0..3).
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..=3))]
    fixture_marker: Option<u8>,
    /// Incoming assembled frames. Private media belongs under ignored .local/.
    #[arg(long)]
    output: PathBuf,
    /// Optional private incoming raw 8 kHz PCMU capture for independent decoding.
    #[arg(long)]
    audio_output: Option<PathBuf>,
    /// Optional private numeric RTP/arrival journal; never contains media or keys.
    #[arg(long)]
    report: Option<PathBuf>,
    #[arg(long, default_value_t = 60)]
    seconds: u64,
    /// Explicit audio source. Human capture requires microphone:device-number.
    #[arg(long, default_value = "tone:440")]
    audio_input: String,
    /// Render incoming video locally with ffplay; never part of automatic checks.
    #[arg(long)]
    preview: bool,
    /// Set for native/native; direct WebRTC bridges require no subscription.
    #[arg(long)]
    native_peer: bool,
    #[arg(long, requires = "camera_mute_for")]
    camera_mute_at: Option<u64>,
    #[arg(long, requires = "camera_mute_at")]
    camera_mute_for: Option<u64>,
    #[arg(long, requires = "microphone_mute_for")]
    microphone_mute_at: Option<u64>,
    #[arg(long, requires = "microphone_mute_at")]
    microphone_mute_for: Option<u64>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let a = Arguments::parse();
    if !(1..=300).contains(&a.seconds) {
        bail!("Use a duration from 1 to 300 seconds");
    }
    for (at, duration) in [
        (a.camera_mute_at, a.camera_mute_for),
        (a.microphone_mute_at, a.microphone_mute_for),
    ] {
        if let (Some(at), Some(duration)) = (at, duration) {
            if duration == 0 || at.checked_add(duration).is_none_or(|end| end >= a.seconds) {
                bail!("Mute interval must end before the participation deadline");
            }
        }
    }
    if a.fixture_marker.is_some() && a.input != "test" && a.input != "test-pulse" {
        bail!("Fixture markers require generated test or test-pulse input");
    }
    let mut measurements = report::Measurements::new(a.report.as_deref()).await?;
    let mut audio_output = match &a.audio_output {
        Some(path) => Some(report::private_output(path).await?),
        None => None,
    };
    let marker_expression = markers::expression(a.seconds);
    if a.input == "test-pulse" {
        eprintln!("Generated synchronized markers: {}", markers::PROGRAM);
    }
    let mut encoder = Command::new(&a.ffmpeg);
    encoder.args(["-nostdin", "-hide_banner", "-loglevel", "error"]);
    if a.input == "test" || a.input == "test-pulse" {
        let mut generated = "testsrc2=size=640x360:rate=30".to_owned();
        if let Some(marker) = a.fixture_marker {
            for (x, y, high) in [
                (0, 0, marker & 1 != 0),
                (80, 0, marker & 2 != 0),
                (0, 45, marker & 1 == 0),
                (80, 45, marker & 2 == 0),
            ] {
                let color = if high { "0xc8c8c8" } else { "0x282828" };
                generated.push_str(&format!(
                    ",drawbox=x={x}:y={y}:w=80:h=45:color={color}:t=fill"
                ));
            }
        }
        // The synchronized full-white pulse deliberately covers the identity.
        // Observers exclude pulses from source classification, not from motion.
        if a.input == "test-pulse" {
            generated.push_str(&format!(
                ",drawbox=x=0:y=0:w=iw:h=ih:color=white:t=fill:enable='{marker_expression}'",
            ));
        }
        encoder.args([
            "-re",
            "-readrate_initial_burst",
            "0",
            "-f",
            "lavfi",
            "-i",
            &generated,
        ]);
    } else if let Some(camera) = a.input.strip_prefix("camera:") {
        encoder.args([
            "-f",
            "avfoundation",
            "-framerate",
            "30",
            "-video_size",
            "640x360",
            "-i",
            camera,
        ]);
    } else {
        encoder.args(["-re", "-readrate_initial_burst", "0", "-i", &a.input]);
    }
    encoder
        .args([
            "-an",
            "-vf",
            "scale=640:360,fps=30",
            "-c:v",
            "libvpx",
            "-deadline",
            "realtime",
            "-cpu-used",
            "8",
            "-b:v",
            "1000k",
            // Native QUIC has no negotiated keyframe feedback. Bound the
            // dependency chain to six frames (200ms at this 30fps encoder).
            "-g",
            "6",
            "-lag-in-frames",
            "0",
            "-flush_packets",
            "1",
            "-f",
            "ivf",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    // Open the accepted Session before capture, avoiding unbounded codec startup backlog.
    let mut output = report::private_output(&a.output).await?;
    write_header(&mut output).await?;
    let mut client = ConnectClient::open(ClientConfig::load(&a.config)?).await?;
    let mut session = client
        .open_native_video(&a.conversation, &a.session, &a.peer)
        .await?;
    let ready: Result<()> = if a.native_peer {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match client
                    .subscribe_native_video_participant(&session, &a.peer)
                    .await
                {
                    Ok(()) => break Ok::<_, connect_client::ClientError>(()),
                    Err(connect_client::ClientError::Application(error))
                        if error.code == "media-peer-not-ready" =>
                    {
                        tokio::time::sleep(Duration::from_millis(100)).await
                    }
                    Err(error) => break Err(error),
                }
            }
        })
        .await
        .context("Wait for the explicitly selected native participant")
        .and_then(|result| result.map_err(Into::into))
    } else {
        Ok(())
    };
    if let Err(error) = ready {
        let _ = session.end(&mut client).await;
        return Err(error);
    }
    let mut encoder = match encoder
        .spawn()
        .context("Start FFmpeg; install it as the example's external encoder")
    {
        Ok(encoder) => encoder,
        Err(error) => {
            let _ = session.end(&mut client).await;
            return Err(error);
        }
    };
    let mut encoded = encoder.stdout.take().context("Encoder stdout")?;
    let (tx, mut rx) = mpsc::channel(2);
    let reader = tokio::spawn(async move {
        read_header(&mut encoded).await?;
        let mut cadence = pacing::capture_cadence(Duration::from_nanos(1_000_000_000 / 30));
        while let Some(frame) = read_frame(&mut encoded).await? {
            cadence.tick().await;
            if tx.send(frame).await.is_err() {
                break;
            }
        }
        Ok::<_, anyhow::Error>(())
    });
    let mut audio_capture =
        match audio::AudioCapture::start(&a.audio_input, &a.ffmpeg, &marker_expression) {
            Ok(capture) => capture,
            Err(error) => {
                reader.abort();
                let _ = encoder.kill().await;
                let _ = encoder.wait().await;
                let _ = session.end(&mut client).await;
                return Err(error);
            }
        };
    let mut preview = if a.preview {
        match preview::Preview::start() {
            Ok(preview) => Some(preview),
            Err(error) => {
                audio_capture.stop().await;
                reader.abort();
                let _ = encoder.kill().await;
                let _ = encoder.wait().await;
                let _ = session.end(&mut client).await;
                return Err(error);
            }
        }
    } else {
        None
    };
    let mut assembler = VideoFrameAssembler::default();
    let mut playout = playout::Playout::new();
    measurements.begin();
    let stop = tokio::time::sleep(Duration::from_secs(a.seconds));
    tokio::pin!(stop);
    let mut end_reason = "error";
    let result: Result<()> = async {
        loop {
            // Poll public channel occupancy throughout participation. The SDK
            // combines these samples with its send/receive and pump samples.
            let _ = session.media_diagnostics();
            let elapsed = measurements.elapsed();
            let camera_muted = a.camera_mute_at.zip(a.camera_mute_for)
                .is_some_and(|(at, duration)| elapsed >= at as f64 && elapsed < (at + duration) as f64);
            let microphone_muted = a.microphone_mute_at.zip(a.microphone_mute_for)
                .is_some_and(|(at, duration)| elapsed >= at as f64 && elapsed < (at + duration) as f64);
            session.set_camera_muted(camera_muted);
            measurements.mute_transition("video", camera_muted);
            session.set_microphone_muted(microphone_muted);
            measurements.mute_transition("audio", microphone_muted);
            let next = playout.next_deadline().into_iter().chain(assembler.next_deadline()).min()
                .unwrap_or_else(|| std::time::Instant::now()+Duration::from_secs(1));
            let release = tokio::time::sleep_until(tokio::time::Instant::from_std(next));
            tokio::pin!(release);
            tokio::select! {
                _ = &mut stop => {end_reason = "deadline";break;},
                _ = tokio::signal::ctrl_c() => {end_reason = "interrupt";break;},
                _ = &mut release => {
                    assembler.expire_stale();
                    for frame in playout.ready(std::time::Instant::now()) {
                        match frame {
                            playout::Ready::Video(frame, received, presentation) => {
                                write_frame_at(&mut output, &frame, presentation).await?;
                                output.flush().await?;
                                measurements.video_at(frame.timestamp_rtp, presentation, frame.keyframe, frame.encoded.len(), received)?;
                                if let Some(preview)=&preview {preview.show(frame);}
                            },
                            playout::Ready::Audio(frame, received) => {
                                if let Some(output)=&mut audio_output {output.write_all(&frame.payload).await?;output.flush().await?;}
                                measurements.audio_at(frame.timestamp_rtp, frame.payload.len(), received)?;
                            },
                        }
                    }
                },
                frame = audio_capture.frames.recv() => {
                    let Some(frame)=frame else {end_reason="child-failure";bail!("Audio capture ended; check the selected source/device");};
                    session.send_pcmu(&frame).await?;
                },
                encoded = rx.recv() => {
                    let Some(encoded) = encoded else { end_reason="child-failure";bail!("Encoder ended before the participation deadline"); };
                    session.send_vp8(&encoded).await?;
                }
                received = session.receive(Duration::from_secs(5)) => {
                    match received {
                        Err(connect_client::ClientError::MediaEnded) => {end_reason = "peer-terminal";break;},
                        Err(error) => return Err(error.into()),
                        Ok(NativeAvFrame::Video(packet)) => {
                            measurements.packet(packet.rtp.len());
                            if let Some(frame) = assembler.push(&packet)? {
                                let range = assembler.completed_sequence_range()
                                    .context("Completed frame must retain its RTP sequence range")?;
                                playout.video(frame, range, std::time::Instant::now());
                            }
                        },
                        Ok(NativeAvFrame::Audio(frame)) => {
                            playout.audio(frame, std::time::Instant::now());
                        },
                    }
                }
            }
        }
        Ok(())
    }.await;
    let assembly = assembler.diagnostics();
    drop(assembler);
    playout.discard_live_buffers();
    let teardown = std::time::Instant::now();
    rx.close();
    reader.abort();
    let ended: Result<()> = match tokio::time::timeout(Duration::from_millis(4500), async {
        audio_capture.stop().await;
        if let Some(preview) = &mut preview {
            preview.stop().await;
        }
        let _ = reader.await;
        let _ = encoder.kill().await;
        let _ = encoder.wait().await;
        session.end(&mut client).await?;
        Ok::<_, anyhow::Error>(())
    })
    .await
    {
        Ok(result) => result,
        Err(_) => Err(anyhow::anyhow!(
            "Native media cleanup exceeded the owned 4.5-second deadline"
        )),
    };
    let sdk_media = report::sdk_media_json(&session);
    // Cancels local routes even after an uncertain reliable end; retain the error.
    drop(session);
    output.flush().await?;
    if let Some(output) = &mut audio_output {
        output.flush().await?;
    }
    measurements
        .finish(
            teardown.elapsed().as_secs_f64(),
            result.is_ok() && ended.is_ok(),
            end_reason,
            assembly,
            &playout.json(),
            &sdk_media,
        )
        .await?;
    result?;
    ended?;
    println!(
        "Native video ended; independently decode the retained IVF output with FFmpeg or ffplay."
    );
    Ok(())
}
