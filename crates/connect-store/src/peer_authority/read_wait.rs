//! Isolated signed-control rendezvous, absent from shipping builds.
//! Signals ONLY the exact Store/current request's actual waited shared lock.
use super::*;
use crate::authority_fences::ReadWaitSignal;
use std::{
    collections::BTreeMap,
    sync::{Mutex, Weak},
};

struct ObservedRequest {
    request: PeerAuthorityRequest,
    signal: Arc<ReadWaitSignal>,
}
#[derive(Default)]
pub(crate) struct Registry(Mutex<BTreeMap<(String, String), Weak<ObservedRequest>>>);
/// Registration owns metadata only: no Store, file, SQL or authority proof.
pub struct PeerReadWaitProbe {
    registry: Arc<Registry>,
    key: (String, String),
    observed: Arc<ObservedRequest>,
}
impl Drop for PeerReadWaitProbe {
    fn drop(&mut self) {
        let mut entries = self.registry.0.lock().unwrap_or_else(|p| p.into_inner());
        if entries
            .get(&self.key)
            .is_some_and(|entry| Weak::ptr_eq(entry, &Arc::downgrade(&self.observed)))
        {
            entries.remove(&self.key);
        }
    }
}
fn key(request: &PeerAuthorityRequest) -> Result<(String, String)> {
    let PeerCommand::NativeStream(intent) = &request.command else {
        return Err(Error::forbidden());
    };
    if !matches!(
        intent.command,
        connect_protocol::native_streams::Command::Subscribe
            | connect_protocol::native_streams::Command::Unsubscribe
    ) {
        return Err(Error::forbidden());
    }
    intent.validate()?;
    Ok((
        request
            .credentials
            .last()
            .cloned()
            .ok_or_else(Error::forbidden)?,
        intent.operation_id.clone(),
    ))
}
fn same_request(a: &PeerAuthorityRequest, b: &PeerAuthorityRequest) -> bool {
    a.credentials == b.credentials
        && a.device == b.device
        && a.key_id == b.key_id
        && a.algorithm == b.algorithm
        && a.command == b.command
}
impl Registry {
    pub(super) fn wait(
        &self,
        request: &PeerAuthorityRequest,
        wait: &AuthorityWait,
    ) -> Result<AuthorityWait> {
        let Ok(key) = key(request) else {
            return Ok(wait.clone());
        };
        let observed = self
            .0
            .lock()
            .map_err(|_| Error::storage())?
            .get(&key)
            .and_then(Weak::upgrade);
        Ok(match observed {
            Some(observed) if same_request(&observed.request, request) => {
                wait.with_read_wait_signal(observed.signal.clone())
            }
            _ => wait.clone(),
        })
    }
}
impl Store {
    /// Observe a real Read WouldBlock for this exact Store, credentials, Device,
    /// key, resources and original signed subscription intent. No pause, bypass,
    /// deadline renewal or admission permission can be created by this probe.
    pub fn observe_peer_read_wait_for_test(
        &self,
        request: PeerAuthorityRequest,
        signal: std::sync::mpsc::Sender<()>,
    ) -> Result<PeerReadWaitProbe> {
        let key = key(&request)?;
        let mut entries = self
            .peer_read_waits
            .0
            .lock()
            .map_err(|_| Error::storage())?;
        entries.retain(|_, value| value.strong_count() > 0);
        if entries.get(&key).and_then(Weak::upgrade).is_some() {
            return Err(Error::new(
                "conflict",
                "Original signed-control probe already registered",
            ));
        }
        if entries.len() >= 16 {
            return Err(Error::new(
                "capacity-exceeded",
                "Signed-control probe capacity exceeded",
            ));
        }
        let observed = Arc::new(ObservedRequest {
            request,
            signal: Arc::new(ReadWaitSignal::new(signal)),
        });
        entries.insert(key.clone(), Arc::downgrade(&observed));
        Ok(PeerReadWaitProbe {
            registry: self.peer_read_waits.clone(),
            key,
            observed,
        })
    }
}
