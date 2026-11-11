//! Actual signed Service->Core continuation, real Store fences and old-route IO.
use super::*;
use connect_service::canonical_media_projection_fixture::{
    PreparationCustodyPhase, PreparationCustodyProbe,
};
use connect_store::{AuthorityMode, AuthorityScope};
use futures_util::FutureExt;
use std::{
    panic::AssertUnwindSafe,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

type Rows = std::collections::BTreeMap<String, (String, String, u64, String)>;
fn rows(f: &Fixture, record: &str) -> Rows {
    let sql = rusqlite::Connection::open(&f.config.database).unwrap();
    let mut statement = sql.prepare("SELECT s.phase,s.attempt_id,s.effect_id,e.attempt,e.state FROM native_replacement_media_steps s JOIN operation_effects e ON e.id=s.effect_id WHERE s.operation_record_id=?1 ORDER BY s.phase").unwrap();
    let rows = statement
        .query_map([record], |r| {
            Ok((r.get(0)?, (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
        })
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    rows
}
fn same_attempts(before: &Rows, after: &Rows) {
    for (phase, (attempt, effect, count, _)) in before {
        let current = after.get(phase).expect("original phase disappeared");
        assert_eq!(
            (&current.0, &current.1, current.2),
            (attempt, effect, *count)
        );
        assert_eq!(current.2, 1, "original phase was submitted more than once");
    }
    assert!(after.values().all(|row| row.2 == 1));
}

// Same actual ffmpeg/libvpx validation as the existing fixture, but retain
// the decoder child across a timeout so its original process always joins.
async fn decode_old_route(
    directory: &Path,
    artifact: &str,
    frame: &[u8],
    cutoff: tokio::time::Instant,
) -> std::result::Result<(), String> {
    let mut input = Vec::from(&b"DKIF\0\0\x20\0VP80"[..]);
    input.extend(640u16.to_le_bytes());
    input.extend(360u16.to_le_bytes());
    for number in [90000u32, 1, 1, 0] {
        input.extend(number.to_le_bytes());
    }
    input.extend((frame.len() as u32).to_le_bytes());
    input.extend(0u64.to_le_bytes());
    input.extend(frame);
    let path = directory.join(artifact);
    init::private_write(&path, &input).map_err(|e| format!("original decoder input: {e}"))?;
    let mut child = tokio::process::Command::new("ffmpeg")
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args([
            "-vf",
            "crop=16:16:0:0",
            "-pix_fmt",
            "yuv420p",
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("original decoder spawn: {e}"))?;
    let pid = child.id();
    let pipes = (child.stdout.take(), child.stderr.take());
    let (Some(mut stdout), Some(mut stderr)) = pipes else {
        let killed = child.start_kill();
        let joined = child.wait().await;
        return Err(format!(
            "original decoder pipes missing: kill={killed:?} join={joined:?}"
        ));
    };
    let mut output = Vec::new();
    let mut error = Vec::new();
    let result = tokio::time::timeout_at(cutoff, async {
        tokio::join!(
            stdout.read_to_end(&mut output),
            stderr.read_to_end(&mut error),
            child.wait()
        )
    })
    .await;
    match result {
        Err(_) => {
            let killed = child.start_kill();
            let joined = child.wait().await;
            return Err(format!("original decoder exceeded original cutoff: pid={pid:?} forced=true kill={killed:?} join={joined:?}"));
        }
        Ok((out, err, status)) => {
            out.map_err(|e| format!("original decoder stdout: {e}"))?;
            err.map_err(|e| format!("original decoder stderr: {e}"))?;
            let status = match status {
                Ok(status) => status,
                Err(error) => {
                    let killed = child.start_kill();
                    let joined = child.wait().await;
                    return Err(format!(
                        "original decoder wait failed: {error}; kill={killed:?} join={joined:?}"
                    ));
                }
            };
            if !status.success() || output.len() != 384 {
                return Err(format!(
                    "original decoder failed: status={status} bytes={} stderr_bytes={}",
                    output.len(),
                    error.len()
                ));
            }
            let mean = output[..256].iter().map(|v| u64::from(*v)).sum::<u64>() as f64 / 256.;
            if (mean - 60.).abs() > 3. {
                return Err("original forwarded frame decoded another source marker".into());
            }
            eprintln!("original old-route decoder joined: pid={pid:?} forced=false status={status} luma_mean={mean}");
        }
    }
    Ok(())
}

/// The failure assertion is deliberately an immediate real CID Read admission,
/// before SQL, packet warming or a receive timeout can obscure old Write custody.
async fn held_route(
    f: &Fixture,
    probe: &PreparationCustodyProbe,
    command: &TransitionCommand,
    root: &connect_client::canonical::payloads::OperationUpdated,
    original: &readiness::OriginalPreparation,
    sender: &sdk::Stream,
    receiver: &sdk::Stream,
    key: &[u8],
    sequence: &mut u16,
    previous: &sdk::VideoReceiveEpoch,
    revoke: bool,
) -> std::result::Result<(), String> {
    let mut withdrawal = None;
    let mut snapshot = None;
    let mut held = None;
    let mut old_stamp = None;
    let mut old_epoch = None;
    let mut observed_phase = None;
    let committed = Arc::new(AtomicBool::new(false));
    let work = AssertUnwindSafe(async {
        let entered_until = original.cutoff;
        tokio::time::timeout_at(entered_until,probe.entered()).await.expect("actual production phase not entered");
        let observed = probe.observation().unwrap();
        observed_phase = Some(observed.phase);
        assert_eq!(observed.operation_id,command.operation_id);
        assert_eq!(observed.record_id,original.record_id);
        assert!(observed.expires_at <= original.expires_at);
        let cutoff = observed.cutoff.min(original.cutoff);
        assert!(tokio::time::Instant::now() < cutoff);
        let scope = AuthorityScope::conversation(&command.conversation_id,AuthorityMode::Read).unwrap();
        let read = loop {
            match f.server.store.try_acquire_authority(std::slice::from_ref(&scope)) {
                Err(error) if error.code == "authority-busy"
                    && error.message == "Current authority reader registry is busy"
                    && tokio::time::Instant::now() < cutoff => tokio::task::yield_now().await,
                result => break result,
            }
        };
        assert!(read.is_ok(),"actual {:?} Invocation retains exclusive Conversation Write across Core boundary: {:?}", observed.phase,read.as_ref().err().map(|e|&e.code));
        drop(read);
        let before = rows(f,&original.record_id);
        assert_eq!(before.len(),if observed.phase == PreparationCustodyPhase::Attach {1} else {2});
        assert_eq!(before["attach"].2,1);
        assert!(!before.contains_key("readiness"));
        let weak = Arc::downgrade(&f.server.store);
        let target = observed.target.clone();
        let source = observed.source.clone();
        let enroll_until = cutoff.min(tokio::time::Instant::now()+Duration::from_secs(1));
        snapshot = Some(loop {
            let weak = weak.clone(); let target_origin = target.clone(); let source_origin = source.clone();
            match f.server.store.try_register_native_media_snapshot_for(&target,Some(&source),move || {
                weak.upgrade().ok_or_else(connect_protocol::Error::forbidden)?
                    .try_native_media_read(&target_origin,Some(&source_origin))
            }) {
                Ok(snapshot) => break snapshot,
                Err(error) if error.code == "authority-busy" && tokio::time::Instant::now() < enroll_until => tokio::task::yield_now().await,
                Err(error) => panic!("actual original route enrollment refused: {}",error.code),
            }
        });
        held = Some(loop {
            match snapshot.as_ref().unwrap().admit() {
                Ok(read) => break read,
                Err(error) if error.code == "authority-busy" && tokio::time::Instant::now() < enroll_until => tokio::time::sleep(Duration::from_millis(1)).await,
                Err(error) => panic!("actual original route publication refused while phase Read held: {}",error.code),
            }
        });
        old_stamp = held.as_ref().unwrap().snapshot_stamp();
        old_epoch = held.as_ref().unwrap().packet_epoch();
        assert!(held.as_ref().unwrap().current() && held.as_ref().unwrap().permits_subscriptions() && old_epoch.as_ref().unwrap().current());
        // Original captured key bytes, fresh outer RTP/sequence and actual old
        // route/receiver epoch; never another encoder capture or readiness.
        let bytes = tokio::time::timeout_at(cutoff,async {
            publish_timestamp(sender,key,1500,sequence).await;
            let (bytes,epoch) = receive_timestamp(receiver,1500).await;
            assert_eq!(bytes,key); assert_eq!(&epoch,previous);
            bytes
        }).await.expect("original old-route frame did not forward under held phase");
        // Own decoder cancellation/join, rather than cancelling a future that
        // owns a child at the outer packet timeout boundary.
        decode_old_route(f.temp.path(), "custody-held-phase-key.ivf", &bytes, cutoff).await.unwrap();
        let now = readiness::original_cutoff(f,root);
        readiness::assert_same_original_preparation(original,&now);
        if revoke {
            let store = f.server.store.clone();
            let admin = f.admin.clone();
            let credential = f.client.credential.clone();
            let writer_committed = committed.clone();
            withdrawal = Some(tokio::task::spawn_blocking(move || {
                let admin = store.authenticate(&admin)?;
                let credential = store.authenticate(&credential)?.credential_id;
                let result = store.revoke(&admin,&credential);
                writer_committed.store(result.is_ok(),Ordering::Release);
                result
            }));
            tokio::time::timeout_at(cutoff,async {
                while old_epoch.as_ref().unwrap().current() { tokio::task::yield_now().await; }
            }).await.expect("actual matching withdrawal did not invalidate original epoch");
            assert!(!held.as_ref().unwrap().current(),"a retained IO fence cannot keep its revoked grant live");
            assert!(snapshot.as_ref().unwrap().admit_original(old_stamp.unwrap()).is_err());
            assert!(snapshot.as_ref().unwrap().admit().is_err());
            assert!(!committed.load(Ordering::Acquire),"withdrawal committed while actual IO fences still held");
            drop(held.take());
            // The real production phase Invocation remains held until release;
            // the old epoch is already refused, never a borrowed future grant.
            assert!(!committed.load(Ordering::Acquire));
            same_attempts(&before,&rows(f,&original.record_id));
        } else {
            assert!(snapshot.as_ref().unwrap().admit().unwrap().current());
            same_attempts(&before,&rows(f,&original.record_id));
        }
    }).catch_unwind().await;
    drop(held.take());
    probe.release();
    let left = tokio::time::timeout_at(original.cutoff, probe.left())
        .await
        .is_ok();
    let withdrawn = match withdrawal {
        Some(join) => Some(
            join.await
                .map_err(|e| format!("original withdrawal join: {e}"))?
                .map_err(|e| format!("original withdrawal: {}", e.code)),
        ),
        None => None,
    };
    if let Some(result) = withdrawn {
        result?;
    }
    if let (Some(snapshot), Some(stamp), Some(epoch)) = (&snapshot, old_stamp, &old_epoch) {
        if revoke && committed.load(Ordering::Acquire) {
            assert!(!epoch.current());
            assert!(snapshot.admit_original(stamp).is_err());
        }
    }
    if !left {
        return Err("actual original phase did not leave its bounded barrier".into());
    }
    if work.is_ok() && !revoke && observed_phase == Some(PreparationCustodyPhase::Activate) {
        // No Ready has been sent yet. Observe only the original physical
        // activation's durable capture, before any final ownership cut.
        tokio::time::timeout_at(original.cutoff, async {
            loop {
                let captured = rows(f, &original.record_id);
                if let Some(row) = captured.get("activate") {
                    if row.3 == "confirmed" {
                        return Ok(());
                    }
                    if row.3 == "unknown" {
                        return Err("original private activation became uncertain".to_owned());
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .map_err(|_| "original private activation capture did not settle".to_owned())??;
        let snapshot = snapshot.as_ref().ok_or("original route snapshot missing")?;
        let stamp = old_stamp.ok_or("original route stamp missing")?;
        let admitted = snapshot.admit_original(stamp).map_err(|_| {
            "private activation revoked the original queued public media epoch".to_owned()
        })?;
        if !admitted.current() || !old_epoch.as_ref().is_some_and(|e| e.current()) {
            return Err("original public media epoch became stale after private activation".into());
        }
        let (bytes, epoch) = tokio::time::timeout_at(original.cutoff, async {
            publish_timestamp(sender, key, 1501, sequence).await;
            receive_timestamp(receiver, 1501).await
        })
        .await
        .map_err(|_| "original video stopped after private activation capture".to_owned())?;
        if bytes != key || &epoch != previous {
            return Err("private activation changed original decoded media identity".into());
        }
        decode_old_route(
            f.temp.path(),
            "custody-after-activate-key.ivf",
            &bytes,
            original.cutoff,
        )
        .await?;
        drop(admitted);
    }
    match work {
        Ok(()) => Ok(()),
        Err(panic) => {
            let message = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "original phase assertion".into());
            Err(message)
        }
    }
}

pub(super) async fn exercise(
    f: &Fixture,
    mode: u8,
    probe: &PreparationCustodyProbe,
    controller: &mut CanonicalHttpClient,
    command: &TransitionCommand,
    root: &connect_client::canonical::payloads::OperationUpdated,
    client: &mut CanonicalClient,
    io: &mut NativeVideoDatagrams,
    encoder: &mut Encoder,
    old_sender: &sdk::Stream,
    retained_receiver: &sdk::Stream,
    retained_sender: &sdk::Stream,
    old_receiver: &sdk::Stream,
    old_sequence: &mut u16,
    retained_sequence: &mut u16,
    target_sequence: &mut u16,
    key: &[u8],
    previous: &sdk::VideoReceiveEpoch,
    old_receiver_epoch: &sdk::VideoReceiveEpoch,
    publisher: &sdk::Client,
    subscriber: &sdk::Client,
    retained_connection: &sdk::Connection,
    voice: u16,
    incoming: u16,
    camera: u16,
    original: &readiness::OriginalPreparation,
) -> std::result::Result<(), String> {
    let revoke = matches!(mode, 28 | 29 | 33);
    let attach = matches!(mode, 26 | 28 | 32 | 33);
    let work = AssertUnwindSafe(async {
        if attach {
            held_route(
                f,
                probe,
                command,
                root,
                original,
                old_sender,
                retained_receiver,
                key,
                old_sequence,
                previous,
                revoke,
            )
            .await?;
            if revoke {
                return Ok(());
            }
        }
        phase(f, controller, command, 5).await;
        if revoke {
            // Natural decoder join before withdrawing authority; this helper
            // deliberately withholds the unchanged Ready until our boundary.
            let ready = readiness::private_bootstrap(
                f,
                controller,
                command,
                root,
                client,
                io,
                encoder,
                retained_sender,
                retained_sequence,
                incoming,
                original.cutoff,
                false,
                14,
                original,
                old_receiver,
                old_receiver_epoch,
            )
            .await
            .expect("actual private decode did not produce original Ready");
            // Reuse the existing bounded replay of this SAME frozen Ready.
            // Withdrawal has not begun yet; eventual Ready success is required
            // before entering the real Signal/old-route revocation assertions.
            readiness::submit(client, &ready, controller, command, root, original.cutoff).await;
            held_route(
                f,
                probe,
                command,
                root,
                original,
                old_sender,
                retained_receiver,
                key,
                old_sequence,
                previous,
                true,
            )
            .await?;
            return Ok(());
        }
        let activate = readiness::activate(
            f,
            controller,
            command,
            root,
            client,
            io,
            encoder,
            retained_sender,
            retained_sequence,
            voice,
            incoming,
            camera,
            retained_receiver,
            publisher,
            subscriber,
            retained_connection,
            previous,
            target_sequence,
            original.cutoff,
        );
        let (done, _) = if attach {
            activate.await
        } else {
            let (activated, audit) = tokio::join!(
                activate,
                held_route(
                    f,
                    probe,
                    command,
                    root,
                    original,
                    old_sender,
                    retained_receiver,
                    key,
                    old_sequence,
                    previous,
                    false
                )
            );
            audit?;
            activated
        };
        assert_eq!(done["state"], "completed");
        assert_eq!(done["record_id"], original.record_id);
        let after = rows(f, &original.record_id);
        assert_eq!(after.len(), 3);
        assert!(after.values().all(|row| row.2 == 1 && row.3 == "confirmed"));
        Ok::<_, String>(())
    })
    .catch_unwind()
    .await;
    probe.release();
    match work {
        Ok(result) => result,
        Err(_) => {
            Err("actual original signed/Core continuation failed (see retained assertion)".into())
        }
    }
}

/// Called only after target, both SDK owners and the actual server have joined.
pub(super) fn assert_settled(database: &Path, record: &str, mode: u8) {
    let db = rusqlite::Connection::open(database).unwrap();
    let mut statement = db.prepare("SELECT s.phase,e.attempt,e.state FROM native_replacement_media_steps s JOIN operation_effects e ON e.id=s.effect_id WHERE s.operation_record_id=?1 ORDER BY s.phase").unwrap();
    let rows = statement
        .query_map([record], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert!(!rows.is_empty());
    assert!(
        rows.iter()
            .all(|row| row.1 == 1 && matches!(row.2.as_str(), "confirmed" | "unknown")),
        "joined original owner left repeated or unrecorded phase custody: {rows:?}"
    );
    if matches!(mode, 28 | 29 | 33) {
        assert_eq!(
            rows.len(),
            if mode == 28 { 1 } else { 2 },
            "withdrawn authority entered another phase"
        );
        assert!(!rows.iter().any(|row| row.0 == "readiness"));
        let commits:u64 = db.query_row("SELECT count(*) FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit' AND started=1",[record],|r|r.get(0)).unwrap();
        assert_eq!(commits, 0, "revocation authorized a new original movement");
    } else {
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|row| row.2 == "confirmed"));
    }
}

#[derive(Debug, PartialEq, Eq)]
struct AllocationStep {
    attempt_id: String,
    effect_id: String,
    attempt: u64,
    submission_generation: u64,
    state: String,
    observation: bool,
}

fn allocation_step(f: &Fixture, record: &str) -> rusqlite::Result<AllocationStep> {
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    db.busy_timeout(Duration::ZERO)?;
    db.query_row(
        "SELECT s.attempt_id,s.effect_id,e.attempt,e.submission_generation,e.state,s.observation_json IS NOT NULL FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=s.operation_record_id WHERE s.operation_record_id=?1 AND s.phase='allocate' AND s.started=1",
        [record], |row| Ok(AllocationStep {attempt_id:row.get(0)?,effect_id:row.get(1)?,attempt:row.get(2)?,submission_generation:row.get(3)?,state:row.get(4)?,observation:row.get(5)?}),
    )
}

pub(super) async fn allocation_capture(
    f: &Fixture,
    probe: &PreparationCustodyProbe,
    client: &mut CanonicalClient,
    command: &TransitionCommand,
    root: &connect_client::canonical::payloads::OperationUpdated,
    offer: Value,
    sender: &sdk::Stream,
    receiver: &sdk::Stream,
    original_key: &[u8],
    sequence: &mut u16,
    original_epoch: &sdk::VideoReceiveEpoch,
    original: &readiness::OriginalPreparation,
    withdraw_operator: Option<&str>,
) -> std::result::Result<(), String> {
    use readiness::surviving_epoch::SqlOwner;
    let allocation = AssertUnwindSafe(client.allocate_native_replacement_target(
        command,
        root,
        serde_json::from_value(offer).unwrap(),
    ))
    .catch_unwind();
    let observe = async {
        let mut sql = None;
        let mut before = None;
        let observed = AssertUnwindSafe(async {
            tokio::time::timeout_at(original.cutoff, probe.entered())
                .await
                .map_err(|_| "original allocation capture did not reach its barrier".to_owned())?;
            if withdraw_operator.is_none() {
                sql = Some(SqlOwner::acquire(f.server.store.clone()));
            }
            let step = allocation_step(f, &original.record_id)
                .map_err(|e| format!("original allocation metadata while SQL held: {e}"))?;
            assert_eq!(
                (
                    &*step.state,
                    step.attempt,
                    step.submission_generation,
                    step.observation
                ),
                ("submitting", 1, 1, false)
            );
            before = Some(step);
            if let Some(operator) = withdraw_operator {
                // Withdraw the source operation's independent operator
                // credential while the exact target's signed request is
                // parked. Its physical signer remains live for the refusal.
                let store = f.server.store.clone();
                let admin = f.admin.clone();
                let operator = operator.to_owned();
                tokio::task::spawn_blocking(move || {
                    let admin = store.authenticate(&admin)?;
                    store.revoke(&admin, &operator)
                })
                .await
                .map_err(|e| format!("original operator withdrawal join: {e}"))?
                .map_err(|e| format!("original operator withdrawal: {}", e.code))?;
                probe.release();
                tokio::time::timeout_at(original.cutoff, probe.left())
                    .await
                    .map_err(|_| "original withdrawal barrier did not release".to_owned())?;
                return Ok(());
            }
            probe.release();
            tokio::time::timeout_at(original.cutoff, probe.left())
                .await
                .map_err(|_| "original allocation capture barrier did not release".to_owned())?;
            let media_until = original
                .cutoff
                .min(tokio::time::Instant::now() + Duration::from_millis(200));
            let (bytes, epoch) = tokio::time::timeout_at(media_until, async {
                publish_timestamp(sender, original_key, 1500, sequence).await;
                receive_timestamp(receiver, 1500).await
            })
            .await
            .map_err(|_| {
                "allocation capture starved actual original video while primary SQL was held"
                    .to_owned()
            })?;
            assert_eq!(&epoch, original_epoch);
            assert_eq!(&bytes, original_key);
            decode_old_route(
                f.temp.path(),
                "custody-allocation-capture-key.ivf",
                &bytes,
                media_until,
            )
            .await?;
            let held = allocation_step(f, &original.record_id)
                .map_err(|e| format!("allocation step after actual old-route decoding: {e}"))?;
            assert_eq!(
                before.as_ref(),
                Some(&held),
                "allocation capture changed while primary SQL was held"
            );
            Ok::<(), String>(())
        })
        .catch_unwind()
        .await;
        let sql_joined = sql.as_mut().is_none_or(SqlOwner::finish);
        drop(sql);
        probe.release();
        let result = observed.unwrap_or_else(|_| {
            Err("actual old media/capture assertion failed (see retained panic)".into())
        });
        (result, before, sql_joined)
    };
    // If the old route fails, keep the actual signed command in flight and
    // release its primary SQL blocker before waiting for the original reply.
    let (allocation, (observed, before, sql_joined)) = tokio::join!(allocation, observe);
    if !sql_joined {
        return Err("original primary SQL owner failed to join".into());
    }
    let allocation = allocation.map_err(|_| "original signed allocation panicked".to_owned())?;
    match (withdraw_operator, allocation) {
        (Some(_), Err(connect_client::ClientError::Application(error)))
            if error.code == "forbidden" => {}
        (Some(_), _) => {
            return Err(
                "original allocation did not return its scoped forbidden after withdrawal".into(),
            );
        }
        (None, Ok(reply))
            if reply.record_id.as_ref() == root.record_id.as_ref()
                && reply.operation_id == root.operation_id => {}
        (None, Ok(_)) => return Err("original allocation replied for another record".into()),
        (None, Err(error)) => {
            return Err(format!(
                "original signed allocation did not settle: {error}"
            ));
        }
    }
    let before = before.ok_or_else(|| "original started allocation was not observed".to_owned())?;
    let after = allocation_step(f, &original.record_id)
        .map_err(|e| format!("confirmed allocation metadata: {e}"))?;
    if (&after.attempt_id, &after.effect_id) != (&before.attempt_id, &before.effect_id)
        || (
            &*after.state,
            after.attempt,
            after.submission_generation,
            after.observation,
        ) != ("confirmed", 1, 1, true)
    {
        return Err(format!(
            "original allocation was not captured exactly once: {before:?} to {after:?}"
        ));
    }
    observed
}
