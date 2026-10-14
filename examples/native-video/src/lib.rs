//! External-codec boundary for the native example. The SDK and server do not
//! launch an encoder, choose capture devices, or depend on FFmpeg.
use anyhow::{bail, Context, Result};
use connect_client::NativeVideoFrame;
use connect_protocol::video_profile::MAX_VIDEO_FRAME_BYTES;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Parse a bounded, 640×360 VP8 IVF stream before accepting frame bytes.
pub async fn read_header(input: &mut (impl AsyncRead + Unpin)) -> Result<()> {
    let mut h = [0u8; 32];
    input.read_exact(&mut h).await.context("Read IVF header")?;
    if &h[..4] != b"DKIF"
        || h[4..8] != [0, 0, 32, 0]
        || &h[8..12] != b"VP80"
        || u16::from_le_bytes([h[12], h[13]]) != 640
        || u16::from_le_bytes([h[14], h[15]]) != 360
    {
        bail!("Expected VP8/640×360 IVF input");
    }
    Ok(())
}

/// Read exactly one bounded frame. Clean EOF differs from a truncated header.
pub async fn read_frame(input: &mut (impl AsyncRead + Unpin)) -> Result<Option<Vec<u8>>> {
    let mut header = [0u8; 12];
    if input.read(&mut header[..1]).await? == 0 {
        return Ok(None);
    }
    input
        .read_exact(&mut header[1..])
        .await
        .context("Truncated IVF frame header")?;
    let bytes = u32::from_le_bytes(header[..4].try_into()?) as usize;
    if bytes == 0 || bytes > MAX_VIDEO_FRAME_BYTES {
        bail!("IVF frame exceeds video profile");
    }
    let mut frame = vec![0u8; bytes];
    input
        .read_exact(&mut frame)
        .await
        .context("Truncated IVF frame")?;
    Ok(Some(frame))
}

/// Write a streaming IVF header. Independent decoders determine the frame count.
pub async fn write_header(output: &mut (impl AsyncWrite + Unpin)) -> Result<()> {
    let mut h = Vec::from(&b"DKIF\x00\x00\x20\x00VP80"[..]);
    h.extend_from_slice(&640u16.to_le_bytes());
    h.extend_from_slice(&360u16.to_le_bytes());
    h.extend_from_slice(&90_000u32.to_le_bytes());
    h.extend_from_slice(&1u32.to_le_bytes());
    h.extend_from_slice(&0u32.to_le_bytes());
    h.extend_from_slice(&0u32.to_le_bytes());
    output.write_all(&h).await?;
    Ok(())
}

/// Store one assembled VP8 frame for an independent decoder or ffplay.
pub async fn write_frame(
    output: &mut (impl AsyncWrite + Unpin),
    frame: &NativeVideoFrame,
) -> Result<()> {
    write_frame_at(output, frame, u64::from(frame.timestamp_rtp)).await
}

/// Use an application-normalized, wrap-aware presentation clock for live IVF.
pub async fn write_frame_at(
    output: &mut (impl AsyncWrite + Unpin),
    frame: &NativeVideoFrame,
    presentation_ticks: u64,
) -> Result<()> {
    if frame.encoded.is_empty() || frame.encoded.len() > MAX_VIDEO_FRAME_BYTES {
        bail!("Invalid assembled frame size");
    }
    // A single bounded write avoids separate filesystem handoffs for each
    // tiny header field while keeping the complete IVF record unchanged.
    let mut record = Vec::with_capacity(12 + frame.encoded.len());
    record.extend_from_slice(&(frame.encoded.len() as u32).to_le_bytes());
    record.extend_from_slice(&presentation_ticks.to_le_bytes());
    record.extend_from_slice(&frame.encoded);
    output.write_all(&record).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    #[tokio::test]
    async fn frame_record_preserves_exact_presentation_and_encoded_bytes() {
        let frame = NativeVideoFrame {
            timestamp_rtp: u32::MAX,
            keyframe: true,
            encoded: vec![0, 1, 2, 3],
        };
        let mut output = Vec::new();
        write_frame_at(&mut output, &frame, 90001).await.unwrap();
        assert_eq!(&output[..4], &4u32.to_le_bytes());
        assert_eq!(&output[4..12], &90001u64.to_le_bytes());
        assert_eq!(
            read_frame(&mut output.as_slice()).await.unwrap(),
            Some(frame.encoded)
        );
    }
    #[tokio::test]
    async fn clean_eof_and_truncated_or_oversized_input_are_distinct() {
        assert!(read_frame(&mut &b""[..]).await.unwrap().is_none());
        assert!(read_frame(&mut &b"\x01"[..]).await.is_err());
        let mut oversized = Vec::from(&(MAX_VIDEO_FRAME_BYTES as u32 + 1).to_le_bytes()[..]);
        oversized.extend_from_slice(&[0; 8]);
        assert!(read_frame(&mut oversized.as_slice()).await.is_err());
    }
}
