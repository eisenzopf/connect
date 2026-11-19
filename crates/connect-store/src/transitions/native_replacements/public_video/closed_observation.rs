//! Historical actual whole-candidate close only. No preparation/permission
//! renewal, current-source read, raw Binding/Phase or caller closed bool.
use super::*;

pub struct NativeReplacementPublicVideoClosedExpected<'a> {
    original: &'a NativeReplacementUnpublishedRead,
    intent: &'a reservation::Intent,
    operation_id: &'a str,
}
impl NativeReplacementPublicVideoClosedExpected<'_> {
    pub fn original(&self) -> &NativeReplacementUnpublishedRead {
        self.original
    }
    pub fn intent(&self) -> &reservation::Intent {
        self.intent
    }
    pub fn operation_id(&self) -> &str {
        self.operation_id
    }
    pub fn matches_original_reservation(
        &self,
        reservation: &NativeReplacementPublicVideoReservation,
    ) -> bool {
        self.original.original.same_capture(&reservation.original)
            && self.original.saved == reservation.saved
            && self.original.original_expiry == reservation.expires_at
            && self.intent == &reservation.intent
    }
}
/// Trusted Host implementation MUST retain actual SAME original nonclone
/// ClosedOriginalCandidate (whole physical/backend/Core terminal joins), bound
/// to this exact historical original Root/reservation/context. Core cancellation,
/// empty queue, retired status, transport pointer or cached bool is insufficient.
/// No Store/current-policy call, await, close/retry IO or new authority admission
/// inside these callbacks. Close joins have ALREADY completed before this API.
pub trait NativeReplacementPublicVideoClosedGuard {
    fn reason(&self) -> Result<wire::DiscardReason>;
    fn publish(&self, commit: Box<dyn FnOnce() -> Result<()> + '_>) -> Result<()>;
}
pub trait NativeReplacementPublicVideoClosedObservation: Send + Sync {
    fn hold_closed(
        &self,
        expected: &NativeReplacementPublicVideoClosedExpected<'_>,
    ) -> Result<Option<Box<dyn NativeReplacementPublicVideoClosedGuard + '_>>>;
}
impl Store {
    /// Reacquire this exact cleanup-only Read AFTER physical joins. An expired
    /// original target/actor may close its owned resource; this method creates
    /// no target/current-source grant and cannot authorize OLD restoration.
    pub fn record_native_replacement_public_video_discarded(
        &self,
        read: &NativeReplacementUnpublishedRead,
        observation: &dyn NativeReplacementPublicVideoClosedObservation,
    ) -> Result<PublicVideoOperationSnapshot> {
        let p = &read.original;
        local_owner(self, &p.owner, &p.runtime)?;
        if !read.current() {
            return Err(Error::forbidden());
        }
        let mut c = self.try_connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        // Preserves exhaustive known-unpublished/CommitUnknown exclusions.
        // Typed NotSubmitted lineage is a separate required followthrough.
        let (saved, required) = read.historical(self, &tx)?;
        if saved != read.saved || !self.authority_covers(&read.held, &required) {
            return Err(Error::forbidden());
        }
        let r = super::runtime_observation::publication_reservation(self, &tx, p)?
            .ok_or_else(unsupported)?;
        let row = rows::by_record(&tx, &p.record)?;
        let expected = NativeReplacementPublicVideoClosedExpected {
            original: read,
            intent: &r.intent,
            operation_id: &row.record.operation_id,
        };
        let guard = observation.hold_closed(&expected)?.ok_or_else(|| {
            Error::new(
                "authority-busy",
                "Exact original closed-candidate owner unavailable",
            )
        })?;
        let reason = guard.reason()?;
        let (_, previous) = project(&tx, &row.record)?;
        match previous {
            None => append_terminal(self, &tx, &read.held, read, reason)?,
            Some(previous) => {
                let mut value = serde_json::to_value(&previous).map_err(|_| Error::storage())?;
                if value["phase"] == "activated" {
                    return Err(Error::forbidden());
                }
                let private_nonce:String=tx.query_row("SELECT private_readiness_nonce FROM native_replacement_public_video_phases WHERE operation_record_id=?1 ORDER BY ordinal DESC LIMIT 1",[&p.record],|row|row.get(0)).map_err(|_|Error::storage())?;
                let source_cut = value
                    .get("source_cut_id")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                value
                    .as_object_mut()
                    .ok_or_else(Error::storage)?
                    .remove("public_readiness");
                value["phase"] = serde_json::json!("discarded");
                value["source_cut_id"] = source_cut;
                value["reason"] = serde_json::to_value(reason).map_err(|_| Error::storage())?;
                // Native binding and optional known cut come ONLY from the
                // immutable original ledger, never inferred or supplied now.
                let next: wire::Phase = serde_json::from_value(value).map_err(|_| conflict())?;
                append_phase_with_unpublished(
                    self,
                    &tx,
                    &read.held,
                    &r,
                    &next,
                    &private_nonce,
                    Some(read),
                )?;
            }
        }
        let (intent, phase) = project(&tx, &row.record)?;
        let snapshot = PublicVideoOperationSnapshot {
            operation: row.record,
            intent,
            phase,
        };
        let successful_commit = std::cell::Cell::new(false);
        guard.publish(Box::new(|| {
            if !read.current() {
                return Err(Error::forbidden());
            }
            tx.commit().map_err(|_| Error::storage())?;
            successful_commit.set(true);
            Ok(())
        }))?;
        if !successful_commit.get() {
            return Err(Error::storage());
        }
        Ok(snapshot)
    }
}
