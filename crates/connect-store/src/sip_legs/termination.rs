//! Connection events cannot acknowledge a worker's unfinished cleanup.
use crate::{now, Store};
use connect_protocol::{Error, Result};
use rusqlite::params;

/// Classification retained from the media adapter's terminal event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SipConnectionEnd {
    /// A normal remote or media connection end.
    Ended,
    /// The adapter observed cancellation; its cleanup may still fail.
    Cancelled,
    /// An authoritative connection failure.
    Failed,
}

impl Store {
    /// Persist a normal connection end or failure. Callers observing an
    /// adapter's cancellation should use [`Self::observe_sip_connection_end`]
    /// so its event cannot precede the worker's cleanup receipt.
    pub fn finish_sip_connection(
        &self,
        connection_id: &str,
        failed: bool,
        reason: &str,
    ) -> Result<()> {
        self.observe_sip_connection_end(
            connection_id,
            if failed {
                SipConnectionEnd::Failed
            } else {
                SipConnectionEnd::Ended
            },
            reason,
        )
    }

    /// Persist an observed connection termination without acknowledging local
    /// cancellation before its cleanup receipt. The outbound worker owns
    /// cleanup while a leg is `starting` or `ending`, and may retain an
    /// `uncertain` result. An adapter's cancellation event cannot replace any
    /// of those states: rvoip can publish it before cleanup succeeds or fails.
    /// Normal ends and failures remain authoritative; settled terminal states
    /// cannot be reopened by a later callback or worker result.
    pub fn observe_sip_connection_end(
        &self,
        connection_id: &str,
        end: SipConnectionEnd,
        reason: &str,
    ) -> Result<()> {
        self.connection()?
            .execute(
                "UPDATE sip_legs SET state=?1,ended_reason=?2,end_requested=0,updated_at=?3 WHERE connection_id=?4 AND state NOT IN ('ended','failed','cancelled') AND NOT (?5 AND state IN ('starting','ending','uncertain'))",
                params![
                    if end == SipConnectionEnd::Failed { "failed" } else { "ended" },
                    reason,
                    now(),
                    connection_id,
                    end == SipConnectionEnd::Cancelled,
                ],
            )
            .map_err(|_| Error::storage())?;
        Ok(())
    }
}
