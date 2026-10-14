//! Explicit external capture/PCMU codec boundary, independent of live video.
use anyhow::{bail, Context, Result};
use std::{path::Path, process::Stdio};
use tokio::{
    io::AsyncReadExt,
    process::{Child, Command},
    sync::mpsc,
    task::JoinHandle,
};

pub struct AudioCapture {
    child: Child,
    reader: JoinHandle<Result<()>>,
    pub frames: mpsc::Receiver<[u8; 160]>,
}
impl AudioCapture {
    pub fn start(input: &str, ffmpeg: &Path, marker_expression: &str) -> Result<Self> {
        let mut command = Command::new(ffmpeg);
        command.args(["-nostdin", "-hide_banner", "-loglevel", "error"]);
        if let Some(frequency) = input
            .strip_prefix("tone:")
            .or_else(|| input.strip_prefix("pulse:"))
        {
            let frequency: u32 = frequency
                .parse()
                .context("Use tone:frequency, pulse:frequency or microphone:device")?;
            if !(100..=3000).contains(&frequency) {
                bail!("Tone must be from 100 to 3000 Hz");
            }
            command.args([
                "-re",
                "-readrate_initial_burst",
                "0",
                "-f",
                "lavfi",
                "-i",
                &format!("sine=frequency={frequency}:sample_rate=8000:samples_per_frame=160"),
            ]);
            if input.starts_with("pulse:") {
                command.args([
                    "-af",
                    &format!("aeval='val(0)*if({marker_expression},1,0.08)'"),
                ]);
            }
        } else if let Some(device) = input.strip_prefix("microphone:") {
            command.args(["-f", "avfoundation", "-i", &format!(":{device}")]);
        } else {
            bail!("Use tone:frequency, pulse:frequency or microphone:device");
        }
        command
            .args([
                "-vn",
                "-ar",
                "8000",
                "-ac",
                "1",
                "-c:a",
                "pcm_mulaw",
                "-flush_packets",
                "1",
                "-f",
                "mulaw",
                "pipe:1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .context("Start explicit FFmpeg audio capture")?;
        let mut output = child.stdout.take().context("Audio capture stdout")?;
        let (tx, frames) = mpsc::channel(2);
        let reader = tokio::spawn(async move {
            let mut cadence = crate::pacing::capture_cadence(std::time::Duration::from_millis(20));
            loop {
                let mut frame = [0u8; 160];
                output
                    .read_exact(&mut frame)
                    .await
                    .context("Audio capture ended")?;
                cadence.tick().await;
                if tx.send(frame).await.is_err() {
                    return Ok(());
                }
            }
        });
        Ok(Self {
            child,
            reader,
            frames,
        })
    }
    pub async fn stop(&mut self) {
        self.frames.close();
        self.reader.abort();
        let _ = (&mut self.reader).await;
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
}
impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.reader.abort();
        let _ = self.child.start_kill();
    }
}
