//! Binary side channel for Python and Node native UCTP/QUIC audio.
//!
//! Frames are `type:u8 || length:u32be || payload`. Audio payloads stay raw;
//! they never enter the JSON control channel.
use connect_client::{ClientConfig, ConnectClient};
use futures_util::{stream, Stream, StreamExt};
use std::{path::Path, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const SUBSCRIBE: u8 = 0x01;
pub const AUDIO: u8 = 0x02;
pub const CLOSE: u8 = 0x03;
pub const READY: u8 = 0x81;
pub const RECEIVED_AUDIO: u8 = 0x82;
pub const ENDED: u8 = 0x83;
pub const ERROR: u8 = 0xff;
const MAX_FRAME: usize = 4096;
type HelperFrame = (u8, Vec<u8>);
type InputFrame = std::io::Result<Option<HelperFrame>>;

pub async fn serve(
    path: &Path,
    conversation_id: &str,
    session_id: &str,
    remote_identity_id: &str,
) -> anyhow::Result<()> {
    let mut client = ConnectClient::open(ClientConfig::load(path)?).await?;
    let mut session = client
        .open_native_audio(conversation_id, session_id, remote_identity_id)
        .await?;
    let mut output = tokio::io::stdout();
    write_frame(&mut output, READY, session.publisher_stream_id.as_bytes()).await?;

    // `read_exact` is not cancellation-safe. Reading stdin directly in the
    // media select can consume a partial frame before incoming audio wins the
    // branch, corrupting the next frame boundary. `unfold` retains the pending
    // read inside the stream when `next()` loses the select race.
    let input_frames = input_frame_stream(tokio::io::stdin());
    tokio::pin!(input_frames);

    let run_result = loop {
        enum Next {
            Input(Option<InputFrame>),
            Audio(connect_client::Result<connect_client::NativeAudioFrame>),
        }
        let next = tokio::select! {
            frame = input_frames.next() => Next::Input(frame),
            frame = session.receive(Duration::from_secs(300)) => Next::Audio(frame),
        };
        match next {
            Next::Input(Some(Ok(Some((SUBSCRIBE, payload))))) => {
                let stream = std::str::from_utf8(&payload)
                    .map_err(|_| anyhow::anyhow!("invalid subscription stream id"))?;
                client.subscribe_native_audio(&session, stream).await?;
            }
            Next::Input(Some(Ok(Some((AUDIO, payload))))) if payload.len() == 160 => {
                session.send_pcmu(&payload).await?;
            }
            Next::Input(Some(Ok(Some((CLOSE, payload))))) if payload.is_empty() => break Ok(()),
            Next::Input(Some(Ok(None)) | None) => break Ok(()),
            Next::Input(Some(Ok(Some(_)))) => {
                write_frame(&mut output, ERROR, b"invalid-frame").await?;
                break Err(anyhow::anyhow!("invalid media helper frame"));
            }
            Next::Input(Some(Err(error))) => break Err(error.into()),
            Next::Audio(Ok(frame)) => {
                write_frame(&mut output, RECEIVED_AUDIO, &frame.payload).await?;
            }
            Next::Audio(Err(connect_client::ClientError::Timeout)) => {}
            Next::Audio(Err(connect_client::ClientError::MediaEnded)) => {
                write_frame(&mut output, ENDED, &[]).await?;
                break Ok(());
            }
            Next::Audio(Err(error)) => break Err(error.into()),
        }
    };
    let close_result = client.close_native_audio(&session).await;
    client.close().await;
    run_result.and(close_result.map_err(Into::into))
}

fn input_frame_stream<R>(input: R) -> impl Stream<Item = InputFrame>
where
    R: AsyncRead + Unpin,
{
    stream::unfold(input, |mut input| async move {
        let frame = read_frame(&mut input).await;
        Some((frame, input))
    })
}

async fn read_frame<R: AsyncRead + Unpin>(input: &mut R) -> std::io::Result<Option<HelperFrame>> {
    let kind = match input.read_u8().await {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    };
    let length = input.read_u32().await? as usize;
    if length > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "media frame exceeds bound",
        ));
    }
    let mut payload = vec![0; length];
    input.read_exact(&mut payload).await?;
    Ok(Some((kind, payload)))
}

async fn write_frame<W: AsyncWrite + Unpin>(
    output: &mut W,
    kind: u8,
    payload: &[u8],
) -> std::io::Result<()> {
    output.write_u8(kind).await?;
    output.write_u32(payload.len() as u32).await?;
    output.write_all(payload).await?;
    output.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn framing_is_bounded_and_binary_safe() {
        let (mut output, mut input) = tokio::io::duplex(32);
        write_frame(&mut output, AUDIO, &[0, 255, 10])
            .await
            .expect("fixture frame should be writable");
        assert_eq!(
            read_frame(&mut input)
                .await
                .expect("fixture frame should be readable"),
            Some((AUDIO, vec![0, 255, 10]))
        );

        let mut oversized = vec![AUDIO];
        oversized.extend_from_slice(&((MAX_FRAME + 1) as u32).to_be_bytes());
        assert_eq!(
            read_frame(&mut oversized.as_slice())
                .await
                .expect_err("oversized fixture must be rejected")
                .kind(),
            std::io::ErrorKind::InvalidData
        );
    }

    #[tokio::test]
    async fn input_reader_preserves_a_frame_when_next_is_cancelled() {
        let (mut writer, reader) = tokio::io::duplex(32);
        let frames = input_frame_stream(reader);
        tokio::pin!(frames);

        writer
            .write_all(&[AUDIO, 0, 0])
            .await
            .expect("partial header should be writable");
        tokio::select! {
            biased;
            frame = frames.next() => panic!("partial frame completed unexpectedly: {frame:?}"),
            _ = tokio::time::sleep(Duration::from_millis(1)) => {},
        }
        writer
            .write_all(&[0, 3, 7, 8, 9])
            .await
            .expect("remaining frame should be writable");
        assert_eq!(
            frames
                .next()
                .await
                .expect("frame stream should stay active")
                .expect("interleaved work must not corrupt the frame"),
            Some((AUDIO, vec![7, 8, 9]))
        );
    }
}
