//! The owned engine job consumes its attempt only after the actual Core call
//! returns success. Retrying this receipt records that same outcome, never IO.
use super::*;

pub struct NativeStreamCommitted {
    lease: NativeStreamLease,
    outcome: CommittedOutcome,
}
enum CommittedOutcome {
    Streams {
        revision: u64,
        resources_changed: bool,
    },
    Codecs {
        revision: u64,
        resources_changed: bool,
        bindings: Vec<connect_protocol::native_streams::CodecBinding>,
    },
    Gates {
        revision: u64,
        streams: Vec<connect_protocol::native_gates::GateStream>,
    },
    Quality {
        interval_ms: u64,
    },
    Advertisement {
        payload: serde_json::Value,
    },
    Ended,
    Queued {
        message_id: String,
    },
}
impl std::fmt::Debug for NativeStreamCommitted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeStreamCommitted(redacted)")
    }
}
impl NativeStreamLease {
    /// Trusted host boundary after the actual native runtime control returned.
    /// Captured states are historical; retry records them without running Core.
    pub fn gates_committed(
        self,
        revision: u64,
        streams: Vec<connect_protocol::native_gates::GateStream>,
    ) -> Result<NativeStreamCommitted> {
        let payload = self.gate_payload.as_ref().ok_or_else(conflict)?;
        connect_protocol::native_gates::validate_observation(payload, &streams)?;
        if self.command != Command::Update
            || self.quality_interval_ms.is_some()
            || revision != self.expected_revision.saturating_add(1)
            || revision > 9_007_199_254_740_991
            || streams
                .iter()
                .any(|s| s.binding.connection_generation != self.generation)
        {
            return Err(conflict());
        }
        Ok(NativeStreamCommitted {
            lease: self,
            outcome: CommittedOutcome::Gates { revision, streams },
        })
    }
    /// Trusted host boundary after the actual native declaration returned.
    /// This records future-negotiation metadata, never a media mutation.
    /// Descriptor replacement and operation completion commit atomically below.
    pub fn advertisement_received(self) -> Result<NativeStreamCommitted> {
        if self.command != Command::Advertise {
            return Err(conflict());
        }
        let payload = self.capability_payload.clone().ok_or_else(conflict)?;
        connect_protocol::native_capabilities::validate_payload(&payload)?;
        Ok(NativeStreamCommitted {
            lease: self,
            outcome: CommittedOutcome::Advertisement { payload },
        })
    }
    /// Trusted host boundary after the actual native configuration returned.
    /// The interval is bound to this attempt's immutable original intent.
    pub fn quality_configured(self, interval_ms: u64) -> Result<NativeStreamCommitted> {
        if self.command != Command::Update || self.quality_interval_ms != Some(interval_ms) {
            return Err(conflict());
        }
        Ok(NativeStreamCommitted {
            lease: self,
            outcome: CommittedOutcome::Quality { interval_ms },
        })
    }
    /// Trusted host boundary after the actual native codec call returned.
    /// These are its captured binding observations, not current publication grants.
    pub fn codecs_committed(
        self,
        revision: u64,
        bindings: Vec<connect_protocol::native_streams::CodecBinding>,
    ) -> Result<NativeStreamCommitted> {
        connect_protocol::native_streams::validate_codec_bindings(&bindings)?;
        if self.command != Command::Update
            || self.quality_interval_ms.is_some()
            || self.gate_payload.is_some()
            || revision != self.expected_revision.saturating_add(1)
            || revision > 9_007_199_254_740_991
            || bindings
                .iter()
                .any(|b| b.connection_generation != self.generation)
        {
            return Err(conflict());
        }
        Ok(NativeStreamCommitted {
            lease: self,
            outcome: CommittedOutcome::Codecs {
                revision,
                bindings,
                resources_changed: true,
            },
        })
    }
    /// Trusted host boundary after the sealed Session codec executor returned.
    /// A no-change result retains its exact original revision and bindings; the
    /// recorder additionally requires this lease's durable Session batch link.
    pub fn session_codecs_committed(
        self,
        revision: u64,
        resources_changed: bool,
        bindings: Vec<connect_protocol::native_streams::CodecBinding>,
    ) -> Result<NativeStreamCommitted> {
        connect_protocol::native_streams::validate_codec_bindings(&bindings)?;
        if self.command != Command::Update
            || self.quality_interval_ms.is_some()
            || self.gate_payload.is_some()
            || revision != self.expected_revision + u64::from(resources_changed)
            || revision > 9_007_199_254_740_991
            || bindings
                .iter()
                .any(|b| b.connection_generation != self.generation)
        {
            return Err(conflict());
        }
        Ok(NativeStreamCommitted {
            lease: self,
            outcome: CommittedOutcome::Codecs {
                revision,
                resources_changed,
                bindings,
            },
        })
    }
    /// Trusted host boundary: call only with the actual returned native engine
    /// outcome, never an observed controller revision, wire field or inference.
    /// Consuming the lease leaves only a durable-recording receipt in custody.
    pub fn committed(
        self,
        revision: u64,
        resources_changed: bool,
    ) -> Result<NativeStreamCommitted> {
        if !matches!(
            self.command,
            Command::Subscribe | Command::Unsubscribe | Command::Reconcile
        ) || revision < self.expected_revision
            || revision > self.expected_revision.saturating_add(1)
            || revision > 9_007_199_254_740_991
            || (resources_changed && revision != self.expected_revision + 1)
        {
            return Err(conflict());
        }
        Ok(NativeStreamCommitted {
            lease: self,
            outcome: CommittedOutcome::Streams {
                revision,
                resources_changed,
            },
        })
    }
    /// Trusted host boundary: call only after the actual native publication
    /// returned queue-admission success. This is neither a physical writer
    /// outcome nor remote delivery. Consuming the lease prevents another call.
    pub fn queued(self) -> Result<NativeStreamCommitted> {
        if self.command != Command::Data {
            return Err(conflict());
        }
        let message_id = self.data_message.clone().ok_or_else(conflict)?;
        Ok(NativeStreamCommitted {
            lease: self,
            outcome: CommittedOutcome::Queued { message_id },
        })
    }
    /// Trusted host boundary: Core termination returned successfully and the
    /// original native adapter confirms that exact generation is absent.
    /// Neither a request field nor absence after an uncertain call proves this.
    pub fn ended(
        self,
        source: &dyn crate::NativeOfferRuntimeObservation,
    ) -> Result<NativeStreamCommitted> {
        if self.command != Command::End
            || source.observe()?
                != (crate::NativeOfferObservation::Retired {
                    runtime_connection_id: self.core_connection.clone(),
                    generation: self.generation,
                })
        {
            return Err(conflict());
        }
        Ok(NativeStreamCommitted {
            lease: self,
            outcome: CommittedOutcome::Ended,
        })
    }
}
pub(super) fn exact(
    c: &Connection,
    store: &Store,
    lease: &NativeStreamLease,
) -> Result<rows::Saved> {
    if lease.runtime != store.runtime_epoch() {
        return Err(Error::forbidden());
    }
    let matches: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM native_stream_operations n JOIN operations o ON o.record_id=n.operation_record_id JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=o.record_id JOIN connection_offers mapping ON mapping.connection_id=o.connection_id WHERE n.operation_record_id=?1 AND n.parent_record_id=?2 AND n.attempt_id=?3 AND n.effect_id=?4 AND n.attempt_runtime=?5 AND n.generation=?6 AND o.connection_id=?7 AND o.expected_revision IS ?8 AND o.expires_at=?9 AND mapping.runtime_connection_id=?10 AND e.provider='native-stream' AND e.submission_generation=1 AND o.command_type=?11)",params![lease.record,lease.parent,lease.attempt,lease.effect,lease.runtime,lease.generation,lease.connection,lease.requested_revision,lease.expires_at,lease.core_connection,lease.command.name()],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !matches {
        return Err(Error::forbidden());
    }
    rows::by_record(c, &lease.record)
}
impl Store {
    /// The effect may have committed. This retains its original identity and
    /// keeps the control reserved; it makes no claim about historical media.
    pub fn native_stream_attempt_uncertain(
        &self,
        lease: &NativeStreamLease,
    ) -> Result<OperationRecord> {
        let owner = self.require_runtime_owner()?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = exact(&tx, self, lease)?;
        if !row.record.state.terminal() && row.record.state != OperationState::Unknown {
            if row.record.state != OperationState::Preparing {
                return Err(conflict());
            }
            tx.execute(
                "UPDATE operation_effects SET state='unknown' WHERE id=?1 AND state='submitting'",
                [&lease.effect],
            )
            .map_err(|_| Error::storage())?;
            rows::change(&tx, &row, OperationState::Unknown, None, self)?;
            workers::invalidate(&tx, &lease.record)?;
        }
        if !owner.current() {
            return Err(Error::forbidden());
        }
        let result = rows::by_record(&tx, &lease.record)?.record;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
    /// Record an actual captured success even after its source expired or a
    /// cancellation raced the engine commit. This grants no new media rights.
    /// A retired slot stays retired; the operation records historical success.
    pub fn complete_native_stream(
        &self,
        receipt: &NativeStreamCommitted,
    ) -> Result<OperationRecord> {
        let lease = &receipt.lease;
        let owner = self.require_runtime_owner()?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = exact(&tx, self, lease)?;
        let mut result = match &receipt.outcome {
            CommittedOutcome::Gates { revision, streams } => {
                serde_json::json!({"connection_id":lease.connection,"connection_generation":lease.generation,"stream_revision":revision,"resources_changed":false,"action":lease.gate_payload.as_ref().ok_or_else(conflict)?["action"],"gate_streams":streams})
            }
            CommittedOutcome::Codecs {
                revision,
                bindings,
                resources_changed,
            } => {
                serde_json::json!({"connection_id":lease.connection,"stream_revision":revision,"resources_changed":resources_changed,"codec_bindings":bindings})
            }
            CommittedOutcome::Streams {
                revision,
                resources_changed,
            } => {
                serde_json::json!({"connection_id":lease.connection,"stream_revision":revision,"resources_changed":resources_changed})
            }
            CommittedOutcome::Quality { interval_ms } => {
                serde_json::json!({"connection_id":lease.connection,"connection_generation":lease.generation,"stream_revision":lease.expected_revision,"resources_changed":false,"quality_interval_ms":interval_ms})
            }
            CommittedOutcome::Advertisement { payload } => {
                serde_json::json!({"connection_id":lease.connection,"connection_generation":lease.generation,"stream_revision":lease.expected_revision,"resources_changed":false,"by_participant":payload["by_participant"],"capabilities":payload["capabilities"],"trigger":payload["trigger"],"advertisement_record_id":lease.record})
            }
            CommittedOutcome::Ended => {
                serde_json::json!({"connection_id":lease.connection,"ended":true})
            }
            CommittedOutcome::Queued { message_id } => {
                serde_json::json!({"connection_id":lease.connection,"message_id":message_id,"core_queue_admission":true})
            }
        };
        let codec_batch:Option<String>=tx.query_row("SELECT batch_record_id FROM session_codec_targets WHERE native_operation_record_id=?1",[&lease.record],|r|r.get(0)).optional().map_err(|_|Error::storage())?;
        if matches!(
            &receipt.outcome,
            CommittedOutcome::Codecs {
                resources_changed: false,
                ..
            }
        ) && codec_batch.is_none()
        {
            return Err(conflict());
        }
        if let Some(batch) = &codec_batch {
            result["session_codec_batch"] = serde_json::json!(batch);
        }
        if lease.command == Command::Reconcile {
            let intent: Intent =
                serde_json::from_str(row.intent.as_deref().ok_or_else(Error::storage)?)
                    .map_err(|_| Error::storage())?;
            result["subscription_record_id"] = intent.payload["subscription_record_id"].clone();
            result["server_reconciliation"] = serde_json::json!(true);
        }
        if row.record.state == OperationState::Completed {
            if row.record.result.as_ref() != Some(&result) {
                return Err(conflict());
            }
        } else {
            if !matches!(
                row.record.state,
                OperationState::Preparing | OperationState::Unknown
            ) {
                return Err(conflict());
            }
            let changed=tx.execute("UPDATE operation_effects SET state='confirmed' WHERE id=?1 AND state IN ('submitting','unknown')",[&lease.effect]).map_err(|_|Error::storage())?;
            if changed != 1 {
                return Err(conflict());
            }
            // Only advance the matching live original generation. Historical
            // completion after actual retirement must never revive the slot.
            let live:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM connections c JOIN native_connection_lifetimes l ON l.connection_id=c.id WHERE c.id=?1 AND c.generation=?2 AND c.observed_runtime_id=?3 AND c.state IN ('ready','active') AND l.operation_record_id=?4 AND l.retired=0 AND l.runtime_epoch=?3 AND l.generation=c.generation)",params![lease.connection,lease.generation,lease.runtime,lease.parent],|r|r.get(0)).map_err(|_|Error::storage())?;
            if let CommittedOutcome::Streams { revision, .. }
            | CommittedOutcome::Codecs { revision, .. }
            | CommittedOutcome::Gates { revision, .. } = &receipt.outcome
            {
                if live {
                    let changed=tx.execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.stream_revision',?2) WHERE id=?1 AND json_extract(readiness_json,'$.stream_revision')=?3",params![lease.connection,revision,lease.expected_revision]).map_err(|_|Error::storage())?;
                    if changed != 1 {
                        return Err(conflict());
                    }
                    if let CommittedOutcome::Codecs { bindings, .. } = &receipt.outcome {
                        tx.execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.codec_bindings',json(?2)) WHERE id=?1",params![lease.connection,serde_json::to_string(bindings).map_err(|_|Error::storage())?]).map_err(|_|Error::storage())?;
                    }
                    if let CommittedOutcome::Gates { .. } = &receipt.outcome {
                        // This is the latest bounded partial observation, not
                        // an unbounded history or a current media grant.
                        tx.execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.last_gate',json(?2)) WHERE id=?1", params![lease.connection, serde_json::to_string(&result).map_err(|_| Error::storage())?]).map_err(|_| Error::storage())?;
                    }
                }
            }
            if let CommittedOutcome::Quality { interval_ms } = &receipt.outcome {
                if live {
                    let changed = tx.execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.quality_interval_ms',?2) WHERE id=?1 AND json_extract(readiness_json,'$.stream_revision')=?3", params![lease.connection, interval_ms, lease.expected_revision]).map_err(|_|Error::storage())?;
                    if changed != 1 {
                        return Err(conflict());
                    }
                }
            }
            if let CommittedOutcome::Advertisement { payload } = &receipt.outcome {
                if live {
                    let descriptor = serde_json::json!({"by_participant":payload["by_participant"],"capabilities":payload["capabilities"],"trigger":payload["trigger"],"operation_record_id":lease.record});
                    let changed=tx.execute("UPDATE connections SET readiness_json=json_set(readiness_json,'$.advertised_capabilities',json(?2)) WHERE id=?1 AND json_extract(readiness_json,'$.stream_revision')=?3",params![lease.connection,serde_json::to_string(&descriptor).map_err(|_|Error::storage())?,lease.expected_revision]).map_err(|_|Error::storage())?;
                    if changed != 1 {
                        return Err(conflict());
                    }
                }
            }
            let mut row = row;
            if row.record.state == OperationState::Preparing {
                rows::change(&tx, &row, OperationState::Committing, None, self)?;
                row = rows::by_record(&tx, &lease.record)?;
            }
            rows::change(&tx, &row, OperationState::Completed, Some(result), self)?;
            close(&tx, &row)?;
            workers::invalidate(&tx, &lease.record)?;
        }
        if let Some(batch) = codec_batch {
            let state = match &receipt.outcome {
                CommittedOutcome::Codecs {
                    resources_changed: true,
                    ..
                } => "updated",
                CommittedOutcome::Codecs {
                    resources_changed: false,
                    ..
                } => "unchanged",
                _ => return Err(conflict()),
            };
            tx.execute("UPDATE session_codec_targets SET state=?2 WHERE native_operation_record_id=?1 AND state IN ('preparing','unknown')",params![lease.record,state]).map_err(|_|Error::storage())?;
            super::super::session_codecs::settle(&tx, &batch, self)?;
        }
        if !owner.current() {
            return Err(Error::forbidden());
        }
        let result = rows::by_record(&tx, &lease.record)?.record;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
}

impl Store {
    /// Trusted owned-job boundary: no effect was definitively submitted. This
    /// includes preflight refusal and atomic local cadence/declaration refusal.
    /// Possible IO or an interrupted call must retain uncertainty instead.
    pub fn fail_native_stream_without_effect(
        &self,
        lease: NativeStreamLease,
        problem: Error,
    ) -> Result<OperationRecord> {
        let owner = self.require_runtime_owner()?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = exact(&tx, self, &lease)?;
        if lease.command == Command::Dtmf {
            let frozen: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM native_dtmf_sources WHERE operation_record_id=?1)",
                    [&lease.record],
                    |r| r.get(0),
                )
                .map_err(|_| Error::storage())?;
            if frozen {
                return Err(conflict());
            }
        }
        if !matches!(
            row.record.state,
            OperationState::Preparing | OperationState::Unknown
        ) {
            return Err(conflict());
        }
        tx.execute("UPDATE operation_effects SET state='failed' WHERE id=?1 AND state IN ('submitting','unknown')",[&lease.effect]).map_err(|_|Error::storage())?;
        fail_before_effect(&tx, &row, &problem, self)?;
        workers::invalidate(&tx, &lease.record)?;
        if !owner.current() {
            return Err(Error::forbidden());
        }
        let result = rows::by_record(&tx, &lease.record)?.record;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(result)
    }
}
