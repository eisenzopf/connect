use super::*;
impl Store {
    /// HTTP resource assertions are checked under the same owned source set as
    /// signed operation recovery, without inventing a physical peer or key.
    pub fn execute_operation_action_claim(
        &self,
        credential: &str,
        claim: crate::PeerOperationClaim,
    ) -> Result<OperationRecord> {
        let authority = self.stable_authority(
            &crate::AuthorityWait::new(crate::authority_fences::DEFAULT_WAIT),
            || {
                let c = self.connection()?;
                let actor = actor_from_connection(&c, credential)?;
                peer::operation_source_scopes(&c, &actor, &claim, true)
            },
        )?;
        let result = self.execute_operation_action_inner(
            credential,
            claim.action.clone(),
            Some(&authority),
            None,
        )?;
        let c = self.connection()?;
        let actor = actor_from_connection(&c, credential)?;
        let sources = peer::operation_source_scopes(&c, &actor, &claim, true)?;
        if !self.authority_covers(&authority, &sources) {
            return Err(Error::forbidden());
        }
        Ok(result)
    }
    pub fn execute_operation_action(
        &self,
        credential: &str,
        action: OperationAction,
    ) -> Result<OperationRecord> {
        self.execute_operation_action_inner(credential, action, None, None)
    }
    /// Protected composition retains an already owned sufficient write proof.
    /// Current principal, target, replay and decision checks still run in the transaction.
    pub fn execute_operation_action_with_authority(
        &self,
        credential: &str,
        action: OperationAction,
        authority: &std::sync::Arc<crate::AuthorityGuardSet>,
    ) -> Result<OperationRecord> {
        self.execute_operation_action_inner(credential, action, Some(authority), None)
    }
    /// Signed canonical composition reuses its exact admitted write/read set.
    pub fn execute_operation_action_with_peer(
        &self,
        credential: &str,
        action: OperationAction,
        peer: &crate::PeerAuthorityGuard,
    ) -> Result<OperationRecord> {
        self.execute_operation_action_inner(
            credential,
            action,
            Some(Self::peer_scopes(peer)),
            Some(peer),
        )
    }
    fn execute_operation_action_inner(
        &self,
        credential: &str,
        action: OperationAction,
        held: Option<&std::sync::Arc<crate::AuthorityGuardSet>>,
        peer: Option<&crate::PeerAuthorityGuard>,
    ) -> Result<OperationRecord> {
        let (own, target, scope) = match &action {
            OperationAction::Get { operation_id } => (None, operation_id, "operation.get"),
            OperationAction::Cancel {
                operation_id,
                target_operation_id,
                ..
            } => (Some(operation_id), target_operation_id, "operation.cancel"),
            OperationAction::Reconcile {
                operation_id,
                target_operation_id,
                ..
            } => (
                Some(operation_id),
                target_operation_id,
                "operation.reconcile",
            ),
        };
        id(target)?;
        if let Some(id_) = own {
            id(id_)?;
            if id_ == target {
                return Err(invalid());
            }
        }
        if let OperationAction::Cancel {
            reason: Some(reason),
            ..
        } = &action
        {
            if reason.len() > 512 || reason.chars().any(|c| c.is_control()) {
                return Err(invalid());
            }
        }
        let authority = self.transition_operation_authority(credential, target, &[scope], held)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        if let Some(peer) = peer {
            self.require_peer_operation(&tx, peer, credential, &action)?;
        }
        let actor = actor_from_connection(&tx, credential)?;
        actor.require(scope)?;
        let tenant = authority::tenant(&tx, &actor)?;
        let mut row =
            rows::find(&tx, &tenant, &actor.identity_id, target)?.ok_or_else(Error::forbidden)?;
        authority::read(&tx, &actor, &row)?;
        admission::require_row(self, &authority, &row)?;
        if row.tombstoned {
            return Err(Error::new(
                "gone",
                "Operation result expired; ID remains reserved",
            ));
        }
        if own.is_none() {
            workers::recover_one(&tx, &row, &self.transition_runtime, self)?;
            row = rows::by_record(&tx, &row.record.record_id)?;
            let result = rows::view(&tx, &actor, row)?;
            actor_from_connection(&tx, credential)?;
            if let Some(peer) = peer {
                self.require_peer_source(&tx, peer, credential)?;
            }
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(result);
        }
        let own = own.ok_or_else(invalid)?;
        crate::operations::reject_control_id(&tx, &actor, own)?;
        // Cancel/reconcile are separate logical decisions, never new target intent.
        let mut value = serde_json::to_value(&action).map_err(|_| invalid())?;
        value
            .as_object_mut()
            .ok_or_else(invalid)?
            .remove("operation_id");
        let (hash, intent) = digest(&value)?;
        if let Some(saved) = rows::find(&tx, &tenant, &actor.identity_id, own)? {
            authority::read(&tx, &actor, &saved)?;
            if saved.digest != hash || saved.record.command_type != scope {
                return Err(conflict());
            }
            admission::require_row(self, &authority, &saved)?;
            let result = rows::view(&tx, &actor, saved)?;
            actor_from_connection(&tx, credential)?;
            if let Some(peer) = peer {
                self.require_peer_source(&tx, peer, credential)?;
            }
            tx.commit().map_err(|_| Error::storage())?;
            return Ok(result);
        }
        let decision: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM operation_decisions WHERE operation_record_id=?1)",
                [&row.record.record_id],
                |r| r.get(0),
            )
            .map_err(|_| Error::storage())?;
        if decision {
            return Err(invalid());
        }
        if let OperationAction::Reconcile {
            outcome,
            evidence_id,
            ..
        } = &action
        {
            id(evidence_id)?;
            if row.record.state.terminal() {
                if row.record.state != outcome.state() {
                    return Err(conflict());
                }
                let attributed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM operation_evidence WHERE observation_id=?1 AND operation_record_id=?2 AND outcome=?3 AND (?3!='completed' OR source_kind='local-state'))",params![evidence_id,row.record.record_id,outcome.state().as_str()],|r|r.get(0)).map_err(|_|Error::storage())?;
                if !attributed {
                    return Err(Error::forbidden());
                }
            }
        }
        let initial_revision = row.record.revision;
        let mut disposition = "already_terminal";
        if !row.record.state.terminal() {
            if !row.ledger
                && !row.native
                && !row.native_stream
                && !row.session_codecs
                && !row.received_dtmf
            {
                return Err(Error::forbidden());
            }
            match &action {
                OperationAction::Cancel { .. } => {
                    if row.received_dtmf {
                        disposition = received_dtmf::cancel(&tx, &row, self)?;
                    } else if row.session_codecs {
                        disposition = session_codecs::cancel(&tx, &row, self)?;
                    } else if row.initial_telephone {
                        disposition = initial_telephone::cancel(&tx, &row, self)?;
                    } else if row.native_stream {
                        disposition = native_streams::cancel(&tx, &row, self)?;
                    } else if row.native {
                        disposition = native_offers::cancel(&tx, &row, self)?;
                    } else {
                        if row.record.state == OperationState::Unknown {
                            effects::touch(&tx, &row.record.record_id, self)?;
                            row = rows::by_record(&tx, &row.record.record_id)?;
                        }
                        tx.execute(
                            "UPDATE operations SET cancel_requested=1 WHERE record_id=?1",
                            [&row.record.record_id],
                        )
                        .map_err(|_| Error::storage())?;
                        let handoff_started = row.native_handoff && tx.query_row("SELECT started FROM native_handoffs WHERE operation_record_id=?1",[&row.record.record_id],|r|r.get::<_,bool>(0)).map_err(|_|Error::storage())?;
                        let next = if handoff_started
                            || rows::unsettled(&tx, &row.record.record_id)?
                            || row.record.state == OperationState::Committing
                        {
                            if row.record.state == OperationState::Ready {
                                rows::change(&tx, &row, OperationState::Interrupted, None, self)?;
                                row = rows::by_record(&tx, &row.record.record_id)?;
                            }
                            OperationState::Unknown
                        } else {
                            OperationState::Cancelled
                        };
                        if next == OperationState::Cancelled {
                            tx.execute("UPDATE operation_effects SET state='cancelled' WHERE operation_record_id=?1 AND state='prepared'",[&row.record.record_id]).map_err(|_|Error::storage())?;
                        }
                        rows::change(&tx, &row, next, None, self)?;
                        workers::invalidate(&tx, &row.record.record_id)?;
                        disposition = if next == OperationState::Cancelled {
                            "cancelled_before_commit"
                        } else {
                            "reconciliation_required"
                        };
                    }
                }
                OperationAction::Reconcile {
                    outcome,
                    evidence_id,
                    ..
                } => {
                    if row.initial_telephone
                        || row.received_dtmf
                        || row.session_codecs
                        || row.native
                        || row.native_stream
                        || (row.native_consultation || row.native_provider)
                        || (row.native_return || row.native_handoff || row.native_replacement)
                    {
                        return Err(Error::new(
                            "reconciliation-required",
                            "Native exchange requires attributed local runtime evidence",
                        ));
                    }
                    id(evidence_id)?;
                    if !matches!(
                        row.record.state,
                        OperationState::Unknown | OperationState::Interrupted
                    ) {
                        return Err(conflict());
                    }
                    let evidence:Option<(u64,u64,String,String,Option<String>)>=tx.query_row("SELECT operation_revision,worker_generation,outcome,source_kind,completion_json FROM operation_evidence WHERE observation_id=?1 AND operation_record_id=?2",params![evidence_id,row.record.record_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(|_|Error::storage())?;
                    let (revision, generation, expected, source, completion) =
                        evidence.ok_or_else(Error::forbidden)?;
                    if revision != row.record.revision
                        || generation != row.generation
                        || expected != outcome.state().as_str()
                    {
                        return Err(conflict());
                    }
                    let result = if *outcome == ReconcileOutcome::Completed {
                        if source != "local-state"
                            || row
                                .record
                                .effects
                                .iter()
                                .any(|e| e.state != EffectState::Confirmed)
                        {
                            return Err(conflict());
                        }
                        let result: TransitionCompletion =
                            serde_json::from_str(completion.as_deref().ok_or_else(conflict)?)
                                .map_err(|_| Error::storage())?;
                        workers::completion_authority(&tx, &row, &result, self)?;
                        // Completion must not revive a removed or re-admitted participant epoch.
                        let changed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM operation_members m JOIN participants p ON p.participant_id=m.participant_id JOIN identities i ON i.id=p.identity_id WHERE m.operation_record_id=?1 AND (p.active=0 OR p.membership_epoch!=m.membership_epoch OR i.revoked=1))",[&row.record.record_id],|r|r.get(0)).map_err(|_|Error::storage())?;
                        if changed {
                            return Err(Error::forbidden());
                        }
                        Some(serde_json::to_value(result).map_err(|_| Error::storage())?)
                    } else {
                        if rows::unsettled(&tx, &row.record.record_id)? {
                            return Err(Error::new("reconciliation-required","Submitted effects require attributed settlement or confirmed compensation"));
                        }
                        None
                    };
                    rows::change(&tx, &row, outcome.state(), result, self)?;
                    workers::invalidate(&tx, &row.record.record_id)?;
                    disposition = "reconciled";
                }
                _ => return Err(invalid()),
            }
        }
        capacity(&tx, self, 1)?;
        let final_row = rows::by_record(&tx, &row.record.record_id)?;
        let result = serde_json::json!({"target_operation_id":target,"state_at_decision":final_row.record.state,"disposition":disposition,"evidence_id":match &action {OperationAction::Reconcile{evidence_id,..}=>Some(evidence_id),_=>None}});
        let record = format!("opr_{}", uuid::Uuid::new_v4());
        tx.execute("INSERT INTO operations(record_id,tenant_id,owner_id,operation_id,digest,command_type,conversation_id,session_id,intent_json,state,result_json,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'completed',?10,?11,?11)",params![record,tenant,actor.identity_id,own,hash,scope,row.record.conversation_id,row.record.session_id,intent,result.to_string(),crate::now()]).map_err(|_|Error::storage())?;
        tx.execute(
            "INSERT INTO operation_decisions VALUES(?1,?2,?3)",
            params![
                record,
                row.record.record_id,
                if scope == "operation.cancel" {
                    "cancel"
                } else {
                    "reconcile"
                }
            ],
        )
        .map_err(|_| Error::storage())?;
        if final_row.record.revision > initial_revision {
            tx.execute("UPDATE journal SET data_json=json_set(data_json,'$.decision_record_id',?2) WHERE operation_record_id=?1 AND resource_revision=?3",params![row.record.record_id,record,final_row.record.revision]).map_err(|_|Error::storage())?;
        } else {
            rows::event(&tx, &record, "operation_decision", self)?;
        }
        actor_from_connection(&tx, credential)?;
        let result = rows::by_record(&tx, &record)?.record;
        let stage = if scope == "operation.cancel" {
            "decision-cancel"
        } else {
            "decision-reconcile"
        };
        if let Some(peer) = peer {
            self.require_peer_source(&tx, peer, credential)?;
        }
        fault(&format!("{stage}:before-commit"), target);
        tx.commit().map_err(|_| Error::storage())?;
        fault(&format!("{stage}:after-commit"), target);
        Ok(result)
    }
    /// Explicit retention maintenance. Closed state alone is not permission to erase
    /// unresolved effects; keep digests and scope tombstones permanently.
    pub fn purge_transition_results(&self, limit: u32) -> Result<u32> {
        let kind = admission::BatchKind::Purge;
        let (ids, authority) = self.transition_batch_authority(kind, limit)?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let rows = self.transition_batch_rows(&tx, kind, &ids, &authority)?;
        for row in &rows {
            tx.execute("UPDATE operations SET intent_json=NULL,result_json=NULL,tombstoned=1 WHERE record_id=?1", [&row.id]).map_err(|_| Error::storage())?;
        }
        let count = rows.len() as u32;
        tx.commit().map_err(|_| Error::storage())?;
        Ok(count)
    }
}
