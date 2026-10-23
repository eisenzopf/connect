//! Installation-owned historical flow facts, independent of hint completion.
use super::*;

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeConnectionFlowFact {
    pub runtime_connection_id: String,
    pub generation: u64,
    pub stream_revision: u64,
    pub credential_revision: u64,
    pub observed_at_millis: i64,
    pub streams: Vec<String>,
    pub negotiated_capabilities: serde_json::Value,
}
/// Trusted bridge from an engine-minted observation. No wire/body can construct
/// a bridge or obtain an installation-issued origin/current media reader.
pub trait NativeConnectionFlowObservation: Send + Sync {
    fn observe(&self) -> Result<NativeConnectionFlowFact>;
}
impl Store {
    /// Persist the first real flow observation for this original incarnation.
    /// Read custody is independently reacquired by the caller; this method
    /// never calls Core, sends a hint or grants IO. The first public ready
    /// observation atomically activates its Session and journals the original start.
    pub fn record_native_connection_flow(
        &self,
        origin: &NativeMediaOrigin,
        read: &NativeMediaRead,
        observation: &dyn NativeConnectionFlowObservation,
    ) -> Result<NativeConnectionFlowFact> {
        let fact = observation.observe()?;
        if !read.current()
            || read.target().connection.connection_id != origin.connection_id()
            || fact.runtime_connection_id != origin.runtime_connection_id()
            || fact.generation != origin.generation()
            || fact.stream_revision == 0
            || fact.credential_revision == 0
            || fact.observed_at_millis <= 0
            || fact.stream_revision > 9_007_199_254_740_991
            || fact.credential_revision > 9_007_199_254_740_991
            || fact.observed_at_millis > chrono::Utc::now().timestamp_millis()
            || fact.streams.is_empty()
            || fact.streams.len() > 64
            || !fact.negotiated_capabilities.is_object()
            || serde_json::to_vec(&fact.negotiated_capabilities)
                .map_err(|_| Error::storage())?
                .len()
                > 16384
            || fact
                .streams
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != fact.streams.len()
        {
            return Err(Error::forbidden());
        }
        for id in &fact.streams {
            connect_protocol::transitions::id(id)?;
        }
        let mut c = self.try_connection()?;
        let tx = c
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let (view, scopes, _) = facts(self, &tx, origin)?;
        if !self.authority_covers(read.interval(), &scopes)
            || !matches!(view.connection.state.as_str(), "ready" | "active")
            || !read.current()
        {
            return Err(Error::forbidden());
        }
        let old:Option<String>=tx.query_row("SELECT fact_json FROM native_connection_flow_facts WHERE connection_id=?1 AND generation=?2 AND runtime_epoch=?3",params![origin.connection_id(),origin.generation(),self.runtime_epoch()],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
        let original = match old {
            Some(raw) => serde_json::from_str(&raw).map_err(|_| Error::storage())?,
            None => {
                let count: u64 = tx
                    .query_row(
                        "SELECT COUNT(*) FROM native_connection_flow_facts",
                        [],
                        |r| r.get(0),
                    )
                    .map_err(|_| Error::storage())?;
                if count >= self.limits.operation_receipts {
                    return Err(Error::new(
                        "capacity-exceeded",
                        "Native flow fact capacity exhausted",
                    ));
                }
                tx.execute(
                    "INSERT INTO native_connection_flow_facts VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    params![
                        origin.connection_id(),
                        origin.generation(),
                        self.runtime_epoch(),
                        fact.runtime_connection_id,
                        origin.conversation_id(),
                        origin.session_id(),
                        origin.participant_id(),
                        origin.membership_epoch(),
                        serde_json::to_string(&fact).map_err(|_| Error::storage())?
                    ],
                )
                .map_err(|_| Error::storage())?;
                fact.clone()
            }
        };
        // The observation is historical. Its private engine witness need not
        // remain live, but its original attribution must not change mid-commit.
        if observation.observe()? != fact || !read.current() {
            return Err(Error::forbidden());
        }
        if view.private_consultation.is_none()
            && !view.handoff_preparation
            && !view.publication_suspended
        {
            crate::session_lifecycle::start(&tx, self, origin, &fact)?;
            crate::session_participant_lifecycle::joined(&tx, self, origin, &fact)?;
        }
        crate::session_reconnect::connected(&tx, origin.session_id())?;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(original)
    }
}
