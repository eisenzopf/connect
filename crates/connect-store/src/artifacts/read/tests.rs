//! Observable I/O bounds and short-read errors, independent of blob size.
#![allow(clippy::unwrap_used)]
use super::read_at;
use std::io::{self, Cursor, Read, Seek, SeekFrom};

/// A virtual large file that rejects I/O outside the requested small window.
struct Window {
    offset: u64,
    length: usize,
    read: usize,
    positioned: bool,
}
impl Seek for Window {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        assert_eq!(position, SeekFrom::Start(self.offset));
        assert!(!self.positioned, "Unexpected second seek");
        self.positioned = true;
        Ok(self.offset)
    }
}
impl Read for Window {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        assert!(self.positioned);
        assert!(
            bytes.len() <= self.length - self.read,
            "Read exceeded requested window"
        );
        for (index, value) in bytes.iter_mut().enumerate() {
            *value = ((self.offset + (self.read + index) as u64) % 251) as u8;
        }
        self.read += bytes.len();
        Ok(bytes.len())
    }
}

#[test]
fn small_chunk_of_large_blob_never_reads_or_buffers_the_whole_blob() {
    for offset in [0, 8 * 1024 * 1024 + 11, 16 * 1024 * 1024 - 32 * 1024] {
        let mut reader = Window {
            offset,
            length: 32 * 1024,
            read: 0,
            positioned: false,
        };
        let content = read_at(&mut reader, offset, 32 * 1024).unwrap();
        assert_eq!(content.len(), 32 * 1024);
        assert_eq!(reader.read, content.len());
        assert!(content
            .iter()
            .enumerate()
            .all(|(i, byte)| *byte == ((offset + i as u64) % 251) as u8));
    }
}

#[test]
fn end_of_file_is_empty_and_truncated_read_is_an_error() {
    let mut reader = Cursor::new(b"0123456789".to_vec());
    assert_eq!(read_at(&mut reader, 7, 3).unwrap(), b"789");
    assert!(read_at(&mut reader, 10, 0).unwrap().is_empty());
    assert_eq!(
        read_at(&mut reader, 8, 3).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
}
