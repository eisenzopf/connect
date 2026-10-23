//! Ordered bounded final callbacks. A busy journal writer must not stop the
//! media loop from delivering the audio needed to complete native bootstrap.
use super::*;
use std::collections::VecDeque;
const LIMIT: usize = 32;
const BYTES: usize = 1024 * 1024;
#[derive(Clone)]
struct Entry {
    source: String,
    role: String,
    body: String,
}
type Writer = Arc<dyn Fn(Entry) -> JoinHandle<Result<()>> + Send + Sync>;
pub(super) struct Archive {
    entries: VecDeque<Entry>,
    bytes: usize,
    pending: Option<JoinHandle<Result<()>>>,
    writer: Writer,
    failed: Option<Error>,
}
impl Archive {
    pub fn new(
        store: Arc<Store>,
        context: PrivateProviderContext,
        receipt: Arc<PrivateProviderApplied>,
    ) -> Self {
        Self::with_writer(Arc::new(move |entry| {
            let s = store.clone();
            let c = context.clone();
            let p = receipt.clone();
            tokio::task::spawn_blocking(move || {
                s.try_record_private_provider_transcript(
                    &c,
                    &p,
                    &entry.source,
                    &entry.role,
                    "final",
                    &entry.body,
                )
                .map(|_| ())
            })
        }))
    }
    fn with_writer(writer: Writer) -> Self {
        Self {
            entries: VecDeque::new(),
            bytes: 0,
            pending: None,
            writer,
            failed: None,
        }
    }
    pub fn push(&mut self, source: String, role: String, body: String) -> Result<()> {
        let size = source
            .len()
            .saturating_add(role.len())
            .saturating_add(body.len());
        if self.entries.len() >= LIMIT || self.bytes.saturating_add(size) > BYTES {
            return Err(problem("private-transcript-capacity"));
        }
        self.bytes += size;
        self.entries.push_back(Entry { source, role, body });
        Ok(())
    }
    fn start(&mut self) {
        if self.pending.is_none() && self.failed.is_none() {
            if let Some(entry) = self.entries.front() {
                self.pending = Some((self.writer)(entry.clone()));
            }
        }
    }
    async fn completed(&mut self) -> Result<()> {
        let Some(work) = self.pending.take() else {
            return Ok(());
        };
        match work.await.map_err(|_| Error::storage())? {
            Ok(()) => {
                let entry = self.entries.pop_front().ok_or_else(Error::storage)?;
                self.bytes -= entry.source.len() + entry.role.len() + entry.body.len();
                Ok(())
            }
            Err(e) if matches!(e.code.as_str(), "authority-busy" | "storage-unavailable") => Ok(()),
            Err(e) => {
                self.failed = Some(e.clone());
                Err(e)
            }
        }
    }
    /// Called by the 20ms media tick. Never waits for unfinished Store work.
    pub async fn tick(&mut self) -> Result<()> {
        if self.pending.as_ref().is_some_and(|work| work.is_finished()) {
            self.completed().await?;
        }
        self.start();
        Ok(())
    }
    /// Release media reads first, then drain admitted Store work before provider
    /// cleanup. Cancellation never detaches an admitted write or starts a new
    /// one; uncommitted queued callbacks are explicitly unconfirmed.
    pub async fn finish(mut self, stop: &CancellationToken) -> Result<()> {
        let until = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            self.completed().await?;
            if let Some(e) = self.failed {
                return Err(e);
            }
            if self.entries.is_empty() {
                return Ok(());
            }
            if stop.is_cancelled() || tokio::time::Instant::now() >= until {
                return Err(problem("private-transcript-unconfirmed"));
            }
            self.start();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    };
    #[tokio::test]
    async fn pending_archive_does_not_block_media_ticks_and_preserves_order_and_capacity() {
        let permits = Arc::new(tokio::sync::Semaphore::new(0));
        let saved = Arc::new(Mutex::new(Vec::new()));
        let p = permits.clone();
        let output = saved.clone();
        let mut archive = Archive::with_writer(Arc::new(move |entry| {
            let p = p.clone();
            let output = output.clone();
            tokio::spawn(async move {
                let _permit = p.acquire().await.unwrap();
                output.lock().unwrap().push(entry.source);
                Ok(())
            })
        }));
        for n in 0..LIMIT {
            archive
                .push(
                    format!("source-{n}"),
                    "assistant".into(),
                    "actual final".into(),
                )
                .unwrap();
        }
        assert_eq!(
            archive
                .push("overflow".into(), "assistant".into(), "unconfirmed".into())
                .unwrap_err()
                .code,
            "private-transcript-capacity"
        );
        tokio::time::timeout(Duration::from_millis(100), async {
            for _ in 0..10 {
                archive.tick().await.unwrap();
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("media ticks blocked behind journal writer");
        assert!(saved.lock().unwrap().is_empty());
        permits.add_permits(1);
        archive.finish(&CancellationToken::new()).await.unwrap();
        assert_eq!(
            *saved.lock().unwrap(),
            (0..LIMIT)
                .map(|n| format!("source-{n}"))
                .collect::<Vec<_>>()
        );
    }
    #[tokio::test]
    async fn archive_retries_same_callback_only_after_busy_store_result() {
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let count = Arc::new(AtomicUsize::new(0));
        let recorded = attempts.clone();
        let mut archive = Archive::with_writer(Arc::new(move |entry| {
            let recorded = recorded.clone();
            let count = count.clone();
            tokio::spawn(async move {
                recorded.lock().unwrap().push(entry.source);
                if count.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err(Error::new("authority-busy", "fixture writer busy"))
                } else {
                    Ok(())
                }
            })
        }));
        archive
            .push(
                "same-logical-callback".into(),
                "assistant".into(),
                "actual final".into(),
            )
            .unwrap();
        archive.finish(&CancellationToken::new()).await.unwrap();
        assert_eq!(
            *attempts.lock().unwrap(),
            vec!["same-logical-callback", "same-logical-callback"]
        );
    }
    #[tokio::test]
    async fn cancellation_drains_admitted_write_and_does_not_start_queued_callback() {
        let permits = Arc::new(tokio::sync::Semaphore::new(0));
        let p = permits.clone();
        let started = Arc::new(AtomicUsize::new(0));
        let count = started.clone();
        let mut archive = Archive::with_writer(Arc::new(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            let p = p.clone();
            tokio::spawn(async move {
                let _permit = p.acquire().await.unwrap();
                Ok(())
            })
        }));
        for source in ["admitted", "queued"] {
            archive
                .push(source.into(), "assistant".into(), "final".into())
                .unwrap();
        }
        archive.tick().await.unwrap();
        let stop = CancellationToken::new();
        stop.cancel();
        let finishing = tokio::spawn(async move { archive.finish(&stop).await });
        tokio::task::yield_now().await;
        assert!(!finishing.is_finished(), "admitted writer was detached");
        assert_eq!(started.load(Ordering::SeqCst), 1);
        permits.add_permits(1);
        assert_eq!(
            finishing.await.unwrap().unwrap_err().code,
            "private-transcript-unconfirmed"
        );
        assert_eq!(started.load(Ordering::SeqCst), 1);
    }
}
