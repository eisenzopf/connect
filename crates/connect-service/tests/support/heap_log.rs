//! Shared test-only capped file writer. No in-server JSON sample retention.
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
const LIMIT: usize = 4 * 1024 * 1024;
#[derive(Clone)]
pub struct Capture(Arc<Mutex<(std::fs::File, usize, bool)>>);
impl Capture {
    pub fn new(path: &std::path::Path) -> Self {
        Self(Arc::new(Mutex::new((
            std::fs::File::create(path).unwrap(),
            0,
            false,
        ))))
    }
    pub fn within_bound(&self) -> bool {
        let state = self.0.lock().unwrap();
        !state.2 && state.1 <= LIMIT
    }
}
impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut state = self.0.lock().unwrap();
        if bytes.len() > LIMIT.saturating_sub(state.1) {
            state.2 = true;
            return Err(io::Error::other("diagnostic log bound"));
        }
        state.0.write_all(bytes)?;
        state.1 += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().unwrap().0.flush()
    }
}
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Self;
    fn make_writer(&'a self) -> Self {
        self.clone()
    }
}
pub fn rows(path: &std::path::Path) -> Vec<serde_json::Value> {
    let bytes = std::fs::read(path).unwrap();
    assert!(bytes.len() <= LIMIT);
    String::from_utf8(bytes)
        .unwrap()
        .lines()
        .filter_map(|line| {
            let row: serde_json::Value = serde_json::from_str(line).unwrap();
            row["fields"]["snapshot"]
                .as_str()
                .map(|value| serde_json::from_str(value).unwrap())
        })
        .collect()
}
