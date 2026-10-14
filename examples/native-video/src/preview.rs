//! Optional local rendering using ffplay. Slow rendering cannot back up media.
use anyhow::{Context, Result};
use connect_client::NativeVideoFrame;
use connect_native_video::{write_frame, write_header};
use std::process::Stdio;
use tokio::{
    process::{Child, Command},
    sync::mpsc,
    task::JoinHandle,
};

pub struct Preview {
    child: Child,
    writer: JoinHandle<Result<()>>,
    frames: mpsc::Sender<NativeVideoFrame>,
}
impl Preview {
    pub fn start() -> Result<Self> {
        let mut child = Command::new("ffplay")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-autoexit",
                "-f",
                "ivf",
                "-i",
                "pipe:0",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("Start ffplay preview; install the example's external renderer")?;
        let mut input = child.stdin.take().context("Renderer stdin")?;
        let (frames, mut rx) = mpsc::channel(2);
        let writer = tokio::spawn(async move {
            write_header(&mut input).await?;
            while let Some(frame) = rx.recv().await {
                write_frame(&mut input, &frame).await?;
            }
            Ok(())
        });
        Ok(Self {
            child,
            writer,
            frames,
        })
    }
    pub fn show(&self, frame: NativeVideoFrame) {
        let _ = self.frames.try_send(frame);
    }
    pub async fn stop(&mut self) {
        self.writer.abort();
        let _ = (&mut self.writer).await;
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
}
impl Drop for Preview {
    fn drop(&mut self) {
        self.writer.abort();
        let _ = self.child.start_kill();
    }
}
