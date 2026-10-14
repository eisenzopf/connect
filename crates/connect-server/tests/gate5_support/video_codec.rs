//! FFmpeg is an independent encoder/decoder, never part of the server process.
#![allow(clippy::unwrap_used)]
use connect_client::{NativeAvFrame, NativeVideoFrame, NativeVideoSession, VideoFrameAssembler};
use connect_native_video::{read_frame, read_header, write_frame, write_header};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{io::AsyncWriteExt, process::Command};

pub async fn generate(seconds: u64) -> Vec<Vec<u8>> {
    let output = tokio::time::timeout(
        Duration::from_secs(120),
        Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=640x360:rate=30",
                "-t",
                &seconds.to_string(),
                "-an",
                "-c:v",
                "libvpx",
                "-deadline",
                "realtime",
                "-cpu-used",
                "8",
                "-b:v",
                "1000k",
                "-g",
                "30",
                "-lag-in-frames",
                "0",
                "-f",
                "ivf",
                "pipe:1",
            ])
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        output.status.success(),
        "FFmpeg source failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut input = output.stdout.as_slice();
    read_header(&mut input).await.unwrap();
    let mut frames = Vec::new();
    while let Some(frame) = read_frame(&mut input).await.unwrap() {
        frames.push(frame);
    }
    assert_eq!(frames.len(), seconds as usize * 30);
    frames
}

pub struct Observation {
    pub frames: Vec<NativeVideoFrame>,
    pub audio: u64,
    pub elapsed: f64,
}
/// Exercise full-duplex streams while sharing one negotiated media handle.
pub async fn participate(
    mut session: NativeVideoSession,
    encoded: &[Vec<u8>],
    tone: u8,
    expected: u8,
    seconds: u64,
) -> (NativeVideoSession, Observation) {
    let mut frames = Vec::new();
    let mut audio_frames = 0;
    let mut assembler = VideoFrameAssembler::default();
    let mut video = tokio::time::interval(Duration::from_nanos(1_000_000_000 / 30));
    video.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut audio = tokio::time::interval(Duration::from_millis(20));
    audio.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let start = std::time::Instant::now();
    let stop = tokio::time::sleep(Duration::from_secs(seconds));
    tokio::pin!(stop);
    let mut index = 0;
    loop {
        tokio::select! {
            _=&mut stop=>break,
            _=video.tick()=>{session.send_vp8(&encoded[index%encoded.len()]).await.unwrap();index+=1;},
            _=audio.tick()=>session.send_pcmu(&[tone;160]).await.unwrap(),
            received=session.receive(Duration::from_secs(5))=>match received.unwrap() {
                NativeAvFrame::Audio(frame)=>{assert_eq!(frame.payload,vec![expected;160]);audio_frames+=1;},
                NativeAvFrame::Video(packet)=>if let Some(frame)=assembler.push(&packet).unwrap(){frames.push(frame);},
            }
        }
    }
    (
        session,
        Observation {
            frames,
            audio: audio_frames,
            elapsed: start.elapsed().as_secs_f64(),
        },
    )
}

pub async fn decode(path: &Path, observation: &Observation) -> usize {
    let mut output = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .await
        .unwrap();
    write_header(&mut output).await.unwrap();
    // A late join waits for the next keyframe; pre-keyframe deltas cannot count.
    let first = observation
        .frames
        .iter()
        .position(|f| f.keyframe)
        .expect("received keyframe");
    for frame in &observation.frames[first..] {
        write_frame(&mut output, frame).await.unwrap();
    }
    output.flush().await.unwrap();
    drop(output);
    let decoded = tokio::time::timeout(
        Duration::from_secs(30),
        Command::new("ffmpeg")
            .args([
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-i",
                path.to_str().unwrap(),
                "-f",
                "framemd5",
                "pipe:1",
            ])
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        decoded.status.success(),
        "independent decoder: {}",
        String::from_utf8_lossy(&decoded.stderr)
    );
    let text = String::from_utf8(decoded.stdout).unwrap();
    let hashes: Vec<_> = text
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| line.split(',').next_back().unwrap().trim())
        .collect();
    let changing = hashes.windows(2).filter(|pair| pair[0] != pair[1]).count();
    assert!(
        changing as f64 / observation.elapsed >= 24.0,
        "decoded moving rate {changing}/{}",
        observation.elapsed
    );
    assert!(
        observation.audio as f64 / observation.elapsed >= 40.0,
        "duplex audio rate"
    );
    changing
}
