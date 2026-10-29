//! Independent decoder of language-owned captures; no SDK packet codec reused.
use super::*;
use std::{path::PathBuf, process::Stdio};
use tokio::io::AsyncReadExt;

pub(crate) fn fixtures() -> PathBuf {
    let path = PathBuf::from(std::env::var_os("CONNECT_GATE6_AV_FIXTURES").unwrap())
        .canonicalize()
        .unwrap();
    assert!(path.starts_with(
        std::env::current_dir()
            .unwrap()
            .join(".local")
            .canonicalize()
            .unwrap()
    ));
    path
}

async fn decoded(input: &Path, audio: bool) -> Vec<u8> {
    let executable = PathBuf::from(std::env::var_os("CONNECT_GATE6_FFMPEG").unwrap());
    assert!(executable.is_absolute() && executable.is_file());
    let mut command = tokio::process::Command::new(executable);
    command.args(["-nostdin", "-hide_banner", "-v", "error"]);
    if audio {
        command.args(["-f", "mulaw", "-ar", "8000", "-ac", "1"]);
    } else {
        command.args(["-f", "ivf"]);
    }
    command.arg("-i").arg(input);
    if audio {
        command.args(["-t", "0.6", "-f", "s16le", "pipe:1"]);
    } else {
        command.args([
            "-frames:v",
            "6",
            "-pix_fmt",
            "rgb24",
            "-f",
            "rawvideo",
            "pipe:1",
        ]);
    }
    let maximum = if audio { 9600 } else { 640 * 360 * 3 * 6 };
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let mut bytes = Vec::new();
        child
            .stdout
            .take()
            .unwrap()
            .take((maximum + 1) as u64)
            .read_to_end(&mut bytes)
            .await
            .unwrap();
        if bytes.len() > maximum {
            child.kill().await.unwrap();
        }
        let status = child.wait().await.unwrap();
        assert!(status.success());
        assert_eq!(bytes.len(), maximum);
        bytes
    })
    .await;
    if result.is_err() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    result.expect("Independent language decoder deadline")
}

pub(crate) async fn verify(prefix: &Path, frequency: f64, high: [bool; 3]) -> Value {
    let audio = PathBuf::from(format!("{}.mulaw", prefix.display()));
    let video = PathBuf::from(format!("{}.ivf", prefix.display()));
    assert_eq!(std::fs::metadata(&audio).unwrap().len(), 4800);
    assert!(std::fs::metadata(&video).unwrap().len() <= 4 * 1024 * 1024);
    let pcm = decoded(&audio, true).await;
    let samples: Vec<f64> = pcm
        .chunks_exact(2)
        .map(|v| f64::from(i16::from_le_bytes([v[0], v[1]])))
        .collect();
    let rms = (samples.iter().map(|x| x * x).sum::<f64>() / samples.len() as f64).sqrt();
    let energy = |frequency: f64| {
        let (sin, cos) = samples
            .iter()
            .enumerate()
            .fold((0.0, 0.0), |(sin, cos), (i, sample)| {
                let phase = 2.0 * std::f64::consts::PI * frequency * i as f64 / 8000.0;
                (sin + sample * phase.sin(), cos + sample * phase.cos())
            });
        sin * sin + cos * cos
    };
    let expected = energy(frequency);
    assert!(rms > 500.0 && expected > 1.0);
    let other = [440.0, 660.0, 880.0, 1100.0]
        .into_iter()
        .filter(|f| *f != frequency)
        .map(energy)
        .fold(1.0, f64::max);
    assert!(
        expected > 25.0 * other,
        "Decoded audio did not identify the expected remote source"
    );
    let rgb = decoded(&video, false).await;
    for frame in rgb.chunks_exact(640 * 360 * 3) {
        for (channel, high) in high.iter().enumerate() {
            let average = frame
                .iter()
                .skip(channel)
                .step_by(3)
                .map(|x| u64::from(*x))
                .sum::<u64>()
                / (640 * 360);
            assert!(
                if *high { average > 200 } else { average < 50 },
                "Decoded video did not identify the expected remote source"
            );
        }
    }
    json!({"identified_audio_hz":frequency,"decoded_audio_samples":samples.len(),"audio_rms":rms,"source_energy_ratio":expected/other,"identified_video_high_channels":high,"decoded_video_frames":6,"decoded_rgb_bytes":rgb.len(),"decoder_children_joined":2})
}
