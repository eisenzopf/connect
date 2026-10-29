//! Bounded binary records for the managed helper's separate media channel.
//!
//! This is local helper IPC, not a UCTP envelope or a media ownership grant.
//! The typed adapter validates metadata and the original Client's binding.
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const HEADER_BYTES: usize = 17;
pub const MAX_METADATA_BYTES: usize = 16 * 1024;
/// Full reliable data payload. RTP retains its separate, smaller packet bound.
pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024;
const MAGIC: &[u8; 4] = b"C6M2";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Request = 0x01,
    Response = 0x81,
    Packet = 0x82,
    Closed = 0x83,
    Failure = 0xff,
}
impl TryFrom<u8> for Kind {
    type Error = std::io::Error;
    fn try_from(value: u8) -> std::io::Result<Self> {
        match value {
            0x01 => Ok(Self::Request),
            0x81 => Ok(Self::Response),
            0x82 => Ok(Self::Packet),
            0x83 => Ok(Self::Closed),
            0xff => Ok(Self::Failure),
            _ => Err(invalid("Unknown helper media record")),
        }
    }
}

/// Metadata is opaque here; a typed adapter must decode and validate it.
/// No media bytes enter the helper's JSON control channel.
#[derive(Clone, PartialEq, Eq)]
pub struct Frame {
    pub kind: Kind,
    pub request_id: u32,
    pub metadata: Vec<u8>,
    pub payload: Vec<u8>,
}
impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HelperMediaFrame")
            .field("kind", &self.kind)
            .field("request_id", &self.request_id)
            .field("metadata_bytes", &self.metadata.len())
            .field("payload_bytes", &self.payload.len())
            .finish()
    }
}
impl Frame {
    fn header(&self) -> std::io::Result<[u8; HEADER_BYTES]> {
        bounds(self.request_id, self.metadata.len(), self.payload.len())?;
        let mut header = [0; HEADER_BYTES];
        header[..4].copy_from_slice(MAGIC);
        header[4] = self.kind as u8;
        header[5..9].copy_from_slice(&self.request_id.to_be_bytes());
        header[9..13].copy_from_slice(&(self.metadata.len() as u32).to_be_bytes());
        header[13..17].copy_from_slice(&(self.payload.len() as u32).to_be_bytes());
        Ok(header)
    }
}
fn invalid(message: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message)
}
fn bounds(id: u32, metadata: usize, payload: usize) -> std::io::Result<()> {
    if id == 0 || metadata > MAX_METADATA_BYTES || payload > MAX_PAYLOAD_BYTES {
        return Err(invalid("Invalid helper media record bounds"));
    }
    Ok(())
}

/// One owned reader preserves partial bytes when an application wait is cancelled.
/// A malformed/truncated record fences the reader; it never scans for a new magic.
pub struct Reader<R> {
    input: R,
    header: [u8; HEADER_BYTES],
    header_used: usize,
    body: Vec<u8>,
    body_used: usize,
    failed: bool,
}
impl<R: AsyncRead + Unpin> Reader<R> {
    pub fn new(input: R) -> Self {
        Self {
            input,
            header: [0; HEADER_BYTES],
            header_used: 0,
            body: Vec::new(),
            body_used: 0,
            failed: false,
        }
    }
    pub async fn next(&mut self) -> std::io::Result<Option<Frame>> {
        if self.failed {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "Helper media reader is fenced",
            ));
        }
        let result = self.read_next().await;
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    async fn read_next(&mut self) -> std::io::Result<Option<Frame>> {
        while self.header_used < HEADER_BYTES {
            let count = self
                .input
                .read(&mut self.header[self.header_used..])
                .await?;
            if count == 0 {
                return if self.header_used == 0 {
                    Ok(None)
                } else {
                    Err(std::io::ErrorKind::UnexpectedEof.into())
                };
            }
            self.header_used += count;
        }
        if &self.header[..4] != MAGIC {
            return Err(invalid("Unsupported helper media profile"));
        }
        let kind = Kind::try_from(self.header[4])?;
        let word = |start| u32::from_be_bytes(self.header[start..start + 4].try_into().unwrap());
        let request_id = word(5);
        let metadata_length = word(9) as usize;
        let payload_length = word(13) as usize;
        bounds(request_id, metadata_length, payload_length)?;
        if self.body.is_empty() {
            self.body.resize(metadata_length + payload_length, 0);
        }
        while self.body_used < self.body.len() {
            let count = self.input.read(&mut self.body[self.body_used..]).await?;
            if count == 0 {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            self.body_used += count;
        }
        let mut metadata = std::mem::take(&mut self.body);
        let payload = metadata.split_off(metadata_length);
        self.header_used = 0;
        self.body_used = 0;
        Ok(Some(Frame {
            kind,
            request_id,
            metadata,
            payload,
        }))
    }
}

/// The writer actor must own this future through completion or retire the channel.
/// Cancelling an application wait must never abandon/restart a partial write.
pub async fn write<W: AsyncWrite + Unpin>(output: &mut W, frame: &Frame) -> std::io::Result<()> {
    let header = frame.header()?;
    output.write_all(&header).await?;
    output.write_all(&frame.metadata).await?;
    output.write_all(&frame.payload).await?;
    output.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn packet(id: u32) -> Frame {
        Frame {
            kind: Kind::Packet,
            request_id: id,
            metadata: br#"{"medium":"audio"}"#.to_vec(),
            payload: (0..160).map(|v| v as u8).collect(),
        }
    }
    async fn bytes(frame: &Frame) -> Vec<u8> {
        let mut output = Vec::new();
        write(&mut output, frame).await.unwrap();
        output
    }
    #[tokio::test]
    async fn cancelled_header_and_body_waits_preserve_the_original_two_boundaries() {
        let first = packet(71);
        let second = packet(72);
        let encoded = bytes(&first).await;
        for prefix in [7, HEADER_BYTES + 8] {
            let (mut writer, input) = tokio::io::duplex(1024);
            let mut reader = Reader::new(input);
            writer.write_all(&encoded[..prefix]).await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(5), reader.next())
                    .await
                    .is_err()
            );
            writer.write_all(&encoded[prefix..]).await.unwrap();
            write(&mut writer, &second).await.unwrap();
            writer.shutdown().await.unwrap();
            assert_eq!(reader.next().await.unwrap(), Some(first.clone()));
            assert_eq!(reader.next().await.unwrap(), Some(second.clone()));
            assert_eq!(reader.next().await.unwrap(), None);
        }
    }
    #[tokio::test]
    async fn untrusted_header_bounds_refuse_before_body_allocation_and_fence() {
        let header = packet(1).header().unwrap();
        for field in [0, 4, 5, 9, 13] {
            let mut bad = header;
            match field {
                0 => bad[0] = 0,
                4 => bad[4] = 0x7f,
                5 => bad[5..9].fill(0),
                _ => bad[field..field + 4].fill(0xff),
            }
            let mut reader = Reader::new(bad.as_slice());
            assert_eq!(
                reader.next().await.unwrap_err().kind(),
                std::io::ErrorKind::InvalidData
            );
            assert!(reader.body.is_empty());
            assert_eq!(
                reader.next().await.unwrap_err().kind(),
                std::io::ErrorKind::BrokenPipe
            );
        }
    }
    #[tokio::test]
    async fn truncated_original_record_never_becomes_clean_eof_or_resynchronizes() {
        let raw = bytes(&packet(8)).await;
        for end in [1, 4, HEADER_BYTES - 1, HEADER_BYTES, raw.len() - 1] {
            let mut reader = Reader::new(&raw[..end]);
            assert_eq!(
                reader.next().await.unwrap_err().kind(),
                std::io::ErrorKind::UnexpectedEof
            );
        }
        let mut reader = Reader::new(&raw[..0]);
        assert_eq!(reader.next().await.unwrap(), None);
    }
    #[tokio::test]
    async fn slow_consumer_backpressures_the_owned_writer_until_the_exact_record_drains() {
        let mut frame = packet(9);
        frame.metadata.resize(MAX_METADATA_BYTES, b' ');
        frame.payload.resize(MAX_PAYLOAD_BYTES, 0xa5);
        let expected = frame.clone();
        let (mut output, input) = tokio::io::duplex(16);
        let writer = tokio::spawn(async move { write(&mut output, &frame).await });
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(!writer.is_finished());
        let mut reader = Reader::new(input);
        assert_eq!(reader.next().await.unwrap(), Some(expected));
        writer.await.unwrap().unwrap();
        assert_eq!(reader.next().await.unwrap(), None);
    }
    #[tokio::test]
    async fn revised_profile_keeps_full_data_bound_and_refuses_previous_magic() {
        let mut frame = packet(4);
        frame.payload.resize(64 * 1024, 0x95);
        let mut raw = bytes(&frame).await;
        assert_eq!(&raw[..4], b"C6M2");
        assert_eq!(
            Reader::new(raw.as_slice()).next().await.unwrap(),
            Some(frame)
        );
        raw[..4].copy_from_slice(b"C6M1");
        let mut reader = Reader::new(raw.as_slice());
        assert!(reader.next().await.is_err());
        assert!(reader.body.is_empty());
    }
    #[tokio::test]
    async fn invalid_outbound_records_write_no_prefix() {
        for (metadata, payload, id) in [
            (MAX_METADATA_BYTES + 1, 0, 1),
            (0, MAX_PAYLOAD_BYTES + 1, 1),
            (0, 0, 0),
        ] {
            let frame = Frame {
                kind: Kind::Request,
                request_id: id,
                metadata: vec![0; metadata],
                payload: vec![0; payload],
            };
            let mut output = Vec::new();
            assert_eq!(
                write(&mut output, &frame).await.unwrap_err().kind(),
                std::io::ErrorKind::InvalidData
            );
            assert!(output.is_empty());
        }
    }
}
