//! Negative-only interleavings for actual native publication tests.
//! Compiled only with test-support; no wire API or media authority.
use super::*;
use connect_protocol::{Error, Result};
use rvoip_core::staged_streams::StreamController;
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) struct Retirement {
    controller: Arc<StreamController>,
    armed: Arc<AtomicBool>,
    during: bool,
}

pub(super) struct PendingRetirement {
    retirement: Retirement,
    thread: std::thread::JoinHandle<()>,
}

impl PendingRetirement {
    pub(super) fn wait_for_cancellation(&self) -> Result<()> {
        let until = std::time::Instant::now() + Duration::from_secs(1);
        while !self.retirement.controller.retirement_requested_for_test() {
            if std::time::Instant::now() >= until {
                return Err(Error::storage());
            }
            std::thread::yield_now();
        }
        // Real retire() has fenced packets, but the publication callback still
        // owns Core metadata and must keep physical retirement from finishing.
        if self.thread.is_finished() {
            return Err(Error::storage());
        }
        self.retirement.armed.store(false, Ordering::Release);
        Ok(())
    }

    pub(super) fn join(self) -> Result<()> {
        self.thread.join().map_err(|_| Error::storage())
    }
}

impl Policy {
    pub(crate) fn retire_replacement_publication_once_for_test(
        &self,
        record: &str,
        controller: Arc<StreamController>,
        during: bool,
    ) -> Result<Arc<AtomicBool>> {
        let mut retirements = self
            .replacement_publication_retirements
            .lock()
            .map_err(|_| Error::storage())?;
        if retirements.contains_key(record) {
            return Err(Error::forbidden());
        }
        let armed = Arc::new(AtomicBool::new(true));
        retirements.insert(
            record.into(),
            Retirement {
                controller,
                armed: armed.clone(),
                during,
            },
        );
        Ok(armed)
    }

    fn take_publication_retirement(
        &self,
        record: &str,
        during: bool,
    ) -> Result<Option<Retirement>> {
        let mut retirements = self
            .replacement_publication_retirements
            .try_lock()
            .map_err(|_| Error::new("authority-busy", "Publication fixture metadata busy"))?;
        if retirements.get(record).is_some_and(|r| r.during == during) {
            Ok(retirements.remove(record))
        } else {
            Ok(None)
        }
    }

    pub(crate) fn retire_at_publication_for_test(&self, record: &str) -> Result<()> {
        if let Some(retirement) = self.take_publication_retirement(record, false)? {
            retirement.controller.retire();
            retirement.armed.store(false, Ordering::Release);
        }
        Ok(())
    }

    pub(super) fn begin_publication_retirement_for_test(
        &self,
        record: &str,
    ) -> Result<Option<PendingRetirement>> {
        let Some(retirement) = self.take_publication_retirement(record, true)? else {
            return Ok(None);
        };
        let controller = retirement.controller.clone();
        let thread = std::thread::Builder::new()
            .name("publication-retirement".into())
            .spawn(move || controller.retire())
            .map_err(|_| Error::storage())?;
        Ok(Some(PendingRetirement { retirement, thread }))
    }
}
