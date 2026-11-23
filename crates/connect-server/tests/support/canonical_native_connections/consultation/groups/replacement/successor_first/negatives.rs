//! Actual signed/native invalidation of ONE already-captured original.
//! No private PCM replay, synthetic epoch, fake Busy, grant or release hook.
use super::*;
use futures_util::FutureExt;
#[derive(Clone, Copy, Debug)]
pub(in super::super) enum Case {
    Revoke,
    Abort,
    SelectionWithdraw,
}
async fn exercise(case: Case) {
    exercise_replacement_with_av_control(
        true,
        true,
        false,
        true,
        false,
        None,
        false,
        Some((false, false, false)),
        false,
        false,
        ActivationAudit::SuccessorNegative(case),
    )
    .await;
}
#[tokio::test]
async fn actual_signed_native_captured_successor_actor_revoke_never_releases_or_delivers_original()
{
    exercise(Case::Revoke).await;
}
#[tokio::test]
async fn actual_signed_native_captured_successor_signed_abort_never_releases_or_delivers_original()
{
    exercise(Case::Abort).await;
}
#[tokio::test]
async fn actual_signed_native_captured_successor_selection_withdraw_never_releases_or_delivers_original(
) {
    exercise(Case::SelectionWithdraw).await;
}

#[derive(Debug)]
struct AbortDecision {
    unsettled_before: usize,
    cancel_requested: bool,
    target_operation: String,
    state_at_decision: String,
    disposition: String,
}
pub(in super::super) struct Evidence {
    observer: SuccessorIngressObserverForTest,
    case: Case,
    record: String,
    candidate: String,
    source_handle: u16,
    sequence: u32,
    timestamp: u32,
    source_hash: u64,
    generations: (u64, u64),
    invalidated_while_fresh_us: u128,
    causal_cancelled_while_fresh_us: u128,
    private_cancelled: bool,
    durable_state: String,
    confirmed_commits: usize,
    operation: String,
    abort_decision: Option<AbortDecision>,
    selected_revision: Option<(u64, u64)>,
}
impl Evidence {
    // Called ONLY after the same original native peers, slots, movement watcher,
    // server and all server owners have joined in the existing parent fixture.
    pub(in super::super) fn validate(self) {
        let s = self
            .observer
            .snapshot()
            .expect("no live owner can hold observation mutex after joins");
        eprintln!("ACTUAL_SIGNED_NATIVE_SUCCESSOR_NEGATIVE case={:?} record={} snapshot={s:?} invalidated_while_fresh_us={} causal_cancelled_while_fresh_us={} cancelled={} durable_state={} selected_revision={:?}",self.case,self.record,self.invalidated_while_fresh_us,self.causal_cancelled_while_fresh_us,self.private_cancelled,self.durable_state,self.selected_revision);
        assert_eq!(s.missed, 0);
        assert_eq!(s.captures, 1);
        assert!(
            matches!(s.phase, None | Some("closed")),
            "original reservation owner/claim must close"
        );
        assert_eq!(s.releases, 0);
        assert_eq!(s.queued, 0);
        assert!(!s.packet_epoch_current_at_admission);
        assert_eq!(s.original_admitted_age_us, 0);
        assert!(s.outputs.is_empty(),"NO proof-associated original may succeed at any guarded target IO, including cleanup races");
        let wire = s
            .wire
            .expect("actual marked original must have been captured");
        assert!(wire.original_owner_exact);
        assert_eq!(wire.connection, self.candidate);
        assert_eq!(wire.stream_local_id, self.source_handle);
        assert_eq!(
            (wire.sequence, wire.timestamp, wire.ssrc),
            (self.sequence, self.timestamp, 0x4736_8f01)
        );
        assert_eq!(wire.payload_fingerprint, self.source_hash);
        assert_eq!(wire.payload_len, 160);
        assert_eq!(
            (wire.connection_generation, wire.stream_generation),
            self.generations
        );
        assert!(
            self.invalidated_while_fresh_us < 200_000,
            "invalidation must precede the original media-age boundary"
        );
        assert!(
            self.causal_cancelled_while_fresh_us < 200_000,
            "actual Root cancellation must be observed while the SAME original is still fresh"
        );
        assert!(self.private_cancelled);
        assert_ne!(self.durable_state, "completed");
        assert_eq!(self.confirmed_commits, 0);
        if let Some((before, after)) = self.selected_revision {
            assert_eq!(after, before + 1);
        }
        if matches!(self.case, Case::Abort) {
            let decision = self
                .abort_decision
                .expect("exact original signed cancel decision");
            eprintln!("ACTUAL_SIGNED_NATIVE_SUCCESSOR_ABORT decision={decision:?}");
            // actions::Cancel deliberately keeps reconciliation-required state
            // while confirmed preparation resources remain uncompensated. Do
            // not manufacture durable Cancelled from a local no-delivery fact.
            assert!(decision.unsettled_before > 0);
            assert!(decision.cancel_requested);
            assert_eq!(decision.target_operation, self.operation);
            assert_eq!(decision.state_at_decision, "unknown");
            assert_eq!(decision.disposition, "reconciliation_required");
            assert_eq!(self.durable_state, "unknown");
        }
    }
}
pub(in super::super) async fn verify(
    f: &Fixture,
    key: &str,
    command: &TransitionCommand,
    candidate: &str,
    peers: &mut [Peer],
    controller: &mut CanonicalHttpClient,
    custody: &connect_service::canonical_media_projection_fixture::PreparationCustodyProbe,
    case: Case,
) -> std::result::Result<Evidence, String> {
    // Existing signed helper assertions must not bypass original cleanup.
    let result = std::panic::AssertUnwindSafe(inner(
        f, key, command, candidate, peers, controller, custody, case,
    ))
    .catch_unwind()
    .await;
    custody.release();
    result.map_err(|_| format!("actual negative helper panicked: {case:?}"))?
}
async fn inner(
    f: &Fixture,
    key: &str,
    command: &TransitionCommand,
    candidate: &str,
    peers: &mut [Peer],
    controller: &mut CanonicalHttpClient,
    custody: &connect_service::canonical_media_projection_fixture::PreparationCustodyProbe,
    case: Case,
) -> std::result::Result<Evidence, String> {
    tokio::time::timeout(Duration::from_secs(5), custody.entered())
        .await
        .map_err(|_| "actual ManagedFinalPrepared not entered".to_owned())?;
    let until = custody
        .original_cutoff()
        .map_err(|e| format!("original cutoff {e:?}"))?;
    let original = tokio::time::timeout_at(until, status(f, key, &command.operation_id))
        .await
        .map_err(|_| "original status cutoff".to_owned())?;
    if original.state != OperationState::Preparing {
        return Err("different/advanced original Root".into());
    }
    let private = f
        .server
        .service
        .canonical_replacement_audio_observer_for_test(&original.record_id)
        .map_err(|e| format!("actual private observer {e:?}"))?;
    let binding = private
        .snapshot()
        .map_err(|e| format!("actual private snapshot {e:?}"))?
        .audio
        .ok_or("actual private native binding absent")?
        .target;
    let readiness = peers[2]
        .client
        .native_replacement_readiness()
        .map_err(|e| format!("signed readiness {e:?}"))?;
    if readiness.record_id() != original.record_id
        || readiness.operation_id() != Some(command.operation_id.as_str())
        || readiness.audio().stream_local_id != peers[2].handle
        || readiness.audio().payload_type != 0
    {
        return Err("different signed original/native role".into());
    }
    let observer = f
        .server
        .service
        .canonical_replacement_successor_observer_for_test(&original.record_id)
        .map_err(|e| format!("actual reservation {e:?}"))?;
    if snap(&observer, until).await?.phase != Some("armed") {
        return Err("actual slot not original unclaimed ARMED".into());
    }
    let selected_before =
        matches!(case, Case::SelectionWithdraw).then(|| stream_revision(f, "codec-source"));
    let sent_at = tokio::time::Instant::now();
    let (sequence, timestamp, source_hash) = send_one(&mut peers[2], pattern(0x52), 0x4736_8f01)?;
    // Test observation bound only; never renews input age or operation authority.
    let causal_until = until.min(sent_at + Duration::from_millis(200));
    loop {
        let s = snap(&observer, until).await?;
        if s.phase == Some("held") && s.captures == 1 {
            if s.queued != 0
                || s.releases != 0
                || s.packet_epoch_current_at_admission
                || !s.outputs.is_empty()
            {
                return Err("unreleased marked original acquired authority or output".into());
            }
            let wire = s
                .wire
                .as_ref()
                .ok_or("captured native original identity absent")?;
            if wire.connection != candidate
                || wire.stream_local_id != peers[2].handle
                || wire.sequence != sequence
                || wire.timestamp != timestamp
                || wire.payload_fingerprint != source_hash
                || !wire.original_owner_exact
            {
                return Err("wrong original captured".into());
            }
            break;
        }
        if tokio::time::Instant::now() >= until || sent_at.elapsed() >= Duration::from_millis(200) {
            return Err("marked original not captured while fresh".into());
        }
        tokio::task::yield_now().await;
    }
    let mut selected_revision = None;
    let mut abort_unsettled_before = None;
    match case {
        Case::Revoke => {
            // SAME actual actor credential used by the original signed Root;
            // existing final_control::Revoke uses this authoritative path.
            let admin = f
                .server
                .store
                .authenticate(&f.admin)
                .map_err(|e| format!("admin {e:?}"))?;
            let store = f.server.store.clone();
            let key = key.to_owned();
            // Always join this original blocking owner, even if it returns late.
            tokio::task::spawn_blocking(move || store.revoke(&admin, &key))
                .await
                .map_err(|e| format!("revoke owner {e:?}"))?
                .map_err(|e| format!("actual revoke {e:?}"))?;
        }
        Case::Abort => {
            // Snapshot the actual existing cancellation condition BEFORE the
            // signed mutation, while the same marked original is still fresh.
            let db = rusqlite::Connection::open(&f.config.database).map_err(|e| e.to_string())?;
            let unsettled:usize=db.query_row("SELECT count(*) FROM unsettled_transition_operations WHERE operation_record_id=?1",[&original.record_id],|r|r.get(0)).map_err(|e|e.to_string())?;
            drop(db);
            if unsettled == 0 {
                return Err("fully prepared original has no unsettled cancellation lineage".into());
            }
            abort_unsettled_before = Some(unsettled);
            tokio::time::timeout_at(until,controller.request_value("operation.cancel",
                json!({"operation_id":command.operation_id,"reason":"cancel actual captured ordinary successor before final cut"}),
                CommandScope {conversation_id:Some(command.conversation_id.clone()),session_id:Some(command.session_id.clone()),connection_id:command.connection_id.clone(),operation_id:Some("cancel-captured-original-successor".into()),..Default::default()}))
                .await.map_err(|_|"signed cancellation original cutoff".to_owned())?.map_err(|e|format!("signed cancellation {e:?}"))?.ack().map_err(|e|format!("signed cancellation ACK {e:?}"))?;
        }
        Case::SelectionWithdraw => {
            // Legal existing stream.unsubscribe, NOT invalid subscriptions[].
            // The receiver credential already owns this explicit subscription.
            let before = selected_before.ok_or("selection revision missing")?;
            let callback = tokio::time::timeout_at(
                until,
                stream_control(
                    &mut peers[0].client,
                    &command.conversation_id,
                    "codec-source",
                    "withdraw-old-while-original-captured",
                    before,
                    "stream.unsubscribe",
                    json!({"strm_ids":["codec-monitor"]}),
                ),
            )
            .await
            .map_err(|_| "signed selection original cutoff".to_owned())?;
            if callback.is_some() {
                return Err("unsubscribe unexpectedly renegotiated the native binding".into());
            }
            let after = stream_revision(f, "codec-source");
            if after != before.checked_add(1).ok_or("selection revision overflow")? {
                return Err("actual selected programme did not change exactly once".into());
            }
            selected_revision = Some((before, after));
        }
    }
    let invalidated_while_fresh_us = sent_at.elapsed().as_micros();
    if invalidated_while_fresh_us >= 200_000 || tokio::time::Instant::now() >= until {
        return Err(
            "invalidation was late: cannot claim a fresh original authorization negative".into(),
        );
    }
    custody.release();
    // Actual Root/route cancellation and a noncommitted model must be
    // observed BEFORE this SAME original reaches its age boundary. A later
    // cancellation plus zero output could otherwise be explained by expiry.
    let causal_cancelled_while_fresh_us = loop {
        match private.snapshot() {
            Ok(s)
                if s.cancelled
                    && s.committed != Some(true)
                    && s.audio.as_ref().is_none_or(|audio| {
                        !audio.private_current
                            && audio.source_muted
                            && audio.ordinary_programme_empty
                    }) =>
            {
                // Sample AFTER the actual state predicate. A scheduling delay
                // across the boundary cannot turn a late observation into proof.
                let observed_at = tokio::time::Instant::now();
                if observed_at >= causal_until {
                    return Err(
                        "actual Root cancellation was not observed while original remained fresh"
                            .into(),
                    );
                }
                break observed_at.duration_since(sent_at).as_micros();
            }
            Ok(_) => {}
            Err(e) if e.code == "authority-busy" => {}
            Err(e) => return Err(format!("actual cancelled original {e:?}")),
        }
        if tokio::time::Instant::now() >= causal_until {
            return Err("actual original did not cancel without model commit before its unchanged age/cutoff".into());
        }
        tokio::task::yield_now().await;
    };
    let (durable_state, confirmed_commits, abort_decision): (String, usize, Option<AbortDecision>) = {
        let db = rusqlite::Connection::open(&f.config.database).map_err(|e| e.to_string())?;
        let (state,confirmed):(String,usize)=db.query_row("SELECT o.state,(SELECT count(*) FROM operation_effects e WHERE e.operation_record_id=o.record_id AND e.provider='native-replacement-commit' AND e.state='confirmed') FROM operations o WHERE o.record_id=?1 AND o.operation_id=?2",rusqlite::params![&original.record_id,&command.operation_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
        let decision = if let Some(unsettled_before) = abort_unsettled_before {
            let (requested,target_operation,state_at_decision,disposition):(bool,String,String,String)=db.query_row("SELECT target.cancel_requested,json_extract(own.result_json,'$.target_operation_id'),json_extract(own.result_json,'$.state_at_decision'),json_extract(own.result_json,'$.disposition') FROM operations own JOIN operation_decisions d ON d.operation_record_id=own.record_id AND d.action='cancel' JOIN operations target ON target.record_id=d.target_record_id AND target.owner_id=own.owner_id AND target.tenant_id=own.tenant_id AND target.conversation_id=own.conversation_id AND target.session_id=own.session_id WHERE target.record_id=?1 AND target.operation_id=?2 AND own.operation_id='cancel-captured-original-successor' AND own.command_type='operation.cancel' AND own.state='completed'",rusqlite::params![&original.record_id,&command.operation_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|e|e.to_string())?;
            Some(AbortDecision {
                unsettled_before,
                cancel_requested: requested,
                target_operation,
                state_at_decision,
                disposition,
            })
        } else {
            None
        };
        (state, confirmed, decision)
    }; // readonly SQL owner ends before parent cleanup; no new authority.
    Ok(Evidence {
        observer,
        case,
        record: original.record_id,
        candidate: candidate.into(),
        source_handle: peers[2].handle,
        sequence,
        timestamp,
        source_hash,
        generations: (binding.connection_generation, binding.generation),
        invalidated_while_fresh_us,
        causal_cancelled_while_fresh_us,
        private_cancelled: true,
        durable_state,
        confirmed_commits,
        operation: command.operation_id.clone(),
        abort_decision,
        selected_revision,
    })
}
