//! One independently owned real decoder, bounded by the original operation.
use connect_client::NativeVideoFrame;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    time::{timeout_at, Instant},
};
pub(super) struct Decoded {
    pub pid: u32,
    pub forced: bool,
    pub pixels: usize,
    pub luma_sum: u64,
}
pub(super) async fn decode(frame: &NativeVideoFrame, cutoff: Instant) -> Result<Decoded, String> {
    if Instant::now() >= cutoff {
        return Err("original replacement expired before decoder launch".into());
    }
    if !frame.keyframe || frame.encoded.is_empty() || frame.encoded.len() > 256 * 1024 {
        return Err("fresh private decoder requires one bounded received keyframe".into());
    }
    let mut ivf = Vec::from(&b"DKIF\0\0\x20\0VP80"[..]);
    ivf.extend(640u16.to_le_bytes());
    ivf.extend(360u16.to_le_bytes());
    for value in [90000u32, 1, 1, 0] {
        ivf.extend(value.to_le_bytes());
    }
    ivf.extend((frame.encoded.len() as u32).to_le_bytes());
    ivf.extend(u64::from(frame.timestamp_rtp).to_le_bytes());
    ivf.extend(&frame.encoded);
    let mut child = tokio::process::Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-threads",
            "1",
            "-f",
            "ivf",
            "-i",
            "pipe:0",
            "-vf",
            "crop=16:16:0:0",
            "-pix_fmt",
            "yuv420p",
            "-frames:v",
            "1",
            "-threads",
            "1",
            "-f",
            "rawvideo",
            "pipe:1",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("actual VP8 decoder unavailable: {e}"))?;
    let pipes = (
        child.id(),
        child.stdin.take(),
        child.stdout.take(),
        child.stderr.take(),
    );
    let (pid, mut input, output, errors) = match pipes {
        (Some(pid), Some(input), Some(output), Some(errors)) => (pid, input, output, errors),
        _ => {
            let kill = child.start_kill();
            let joined = child.wait().await;
            eprintln!(
                "native private decoder missing original custody: kill={kill:?} join={joined:?}"
            );
            return Err("actual decoder process/pipe custody missing".into());
        }
    };
    let mut output = output.take(385);
    let mut errors = errors.take(4097);
    let mut pcm = Vec::new();
    let mut stderr = Vec::new();
    let result = timeout_at(cutoff, async {
        tokio::try_join!(
            async move {
                input.write_all(&ivf).await?;
                input.shutdown().await?;
                drop(input); // actual EOF, not merely a flushed pipe
                Ok::<(), std::io::Error>(())
            },
            async { output.read_to_end(&mut pcm).await.map(|_| ()) },
            async { errors.read_to_end(&mut stderr).await.map(|_| ()) }
        )?;
        child.wait().await
    })
    .await;
    // Never assert/drop an original live child before explicit cancellation and
    // join. Cleanup after the original cutoff cannot produce an accepted proof.
    let mut forced = false;
    let (status, error) = match result {
        Ok(Ok(status)) => (Some(status), None),
        failure => {
            forced = true;
            let kill = child.start_kill();
            let joined = child.wait().await;
            eprintln!(
                "native private decoder: pid={pid} forced=true join={joined:?} kill={kill:?}"
            );
            (
                joined.ok(),
                Some(format!("original bounded VP8 decode failed: {failure:?}")),
            )
        }
    };
    if let Some(error) = error {
        return Err(error);
    }
    let status = status.ok_or("actual decoder was not joined")?;
    eprintln!(
        "native private decoder: pid={pid} joined=true status={status} forced={forced} stdout_bytes={} stderr_bytes={}",
        pcm.len(),
        stderr.len()
    );
    if !status.success() || pcm.len() != 384 || stderr.len() > 4096 {
        return Err(format!(
            "actual received VP8 decode refused: status={status}, pixels={}, stderr={}",
            pcm.len(),
            String::from_utf8_lossy(&stderr)
        ));
    }
    let luma_sum = pcm[..256].iter().map(|v| u64::from(*v)).sum::<u64>();
    // The selected source is the retained black synthetic programme. Its real
    // output pixels, not the uncompressed frame tag, establish fresh decoding.
    if !(15 * 256..=17 * 256).contains(&luma_sum) {
        return Err(format!("actual decoded source marker mismatch: {luma_sum}"));
    }
    Ok(Decoded {
        pid,
        forced,
        pixels: pcm.len(),
        luma_sum,
    })
}
