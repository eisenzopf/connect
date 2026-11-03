//! Background lifetime observations grant no operation or packet authority.
use connect_protocol::Result;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Instant,
};
use tokio_util::sync::CancellationToken;

fn diagnostic(budget: &AtomicUsize) -> bool {
    tracing::enabled!(target: "connect_service::peer_authority_lifetime", tracing::Level::DEBUG)
        && budget
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                (n < 2048).then_some(n + 1)
            })
            .is_ok()
}

pub(super) fn observe(signal: &CancellationToken, observation: Result<()>, budget: &AtomicUsize) {
    if let Err(error) = observation {
        if matches!(
            error.code.as_str(),
            "storage-unavailable" | "authority-busy" | "authority-unavailable"
        ) {
            // Missing metadata is not a negative lifetime fact. Fresh command
            // and packet grants still refuse unavailable current authority;
            // the separately owned credential/key expiry timer is unchanged.
            if diagnostic(budget) {
                tracing::debug!(target: "connect_service::peer_authority_lifetime", code=%error.code,
                    "Peer lifetime observation deferred");
            }
        } else {
            signal.cancel();
            if diagnostic(budget) {
                tracing::debug!(target: "connect_service::peer_authority_lifetime", code=%error.code,
                    "Peer lifetime retired by current refusal");
            }
        }
    }
}

// Explicit debug lane only. Payload::kind supplies a closed static wire tag.
// No request IDs, scopes, credentials, configuration or payloads are captured.
pub(super) struct CommandTiming {
    kind: &'static str,
    started: Instant,
    admitted: Option<Instant>,
    budget: Arc<AtomicUsize>,
}
impl CommandTiming {
    pub(super) fn start(kind: &'static str, budget: Arc<AtomicUsize>) -> Option<Self> {
        tracing::enabled!(target: "connect_service::peer_authority_lifetime", tracing::Level::DEBUG)
            .then(|| Self {
                kind,
                started: Instant::now(),
                admitted: None,
                budget,
            })
    }
    pub(super) fn admitted(&mut self) {
        self.admitted = Some(Instant::now());
    }
}
impl Drop for CommandTiming {
    fn drop(&mut self) {
        let now = Instant::now();
        let admitted = self.admitted.unwrap_or(now);
        let held_ms = now.duration_since(admitted).as_millis();
        let admission_ms = admitted.duration_since(self.started).as_millis();
        if held_ms.max(admission_ms) < 100 {
            return;
        }
        // One shared per-policy cap; no unbounded diagnostic state or records.
        if !diagnostic(&self.budget) {
            return;
        }
        tracing::debug!(target: "connect_service::peer_authority_lifetime",
            command=self.kind, admission_ms=admission_ms as u64, held_ms=held_ms as u64,
            admitted=self.admitted.is_some(), "Peer command authority interval released");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use connect_protocol::Error;

    #[test]
    fn unavailable_metadata_can_recover_then_definite_revocation_is_terminal() {
        let signal = CancellationToken::new();
        let budget = AtomicUsize::new(0);
        for code in [
            "storage-unavailable",
            "authority-busy",
            "authority-unavailable",
        ] {
            observe(
                &signal,
                Err(Error::new(code, "synthetic unavailable metadata")),
                &budget,
            );
            assert!(!signal.is_cancelled());
        }
        observe(&signal, Ok(()), &budget);
        assert!(!signal.is_cancelled());
        observe(&signal, Err(Error::forbidden()), &budget);
        assert!(signal.is_cancelled());
        observe(&signal, Ok(()), &budget);
        assert!(signal.is_cancelled());
    }

    #[test]
    fn nonavailability_refusals_and_unknown_errors_retire() {
        for code in [
            "unauthorized",
            "forbidden",
            "not-found",
            "conflict",
            "unexpected",
        ] {
            let signal = CancellationToken::new();
            observe(
                &signal,
                Err(Error::new(code, "synthetic refusal")),
                &AtomicUsize::new(0),
            );
            assert!(signal.is_cancelled(), "{code}");
        }
    }
}
