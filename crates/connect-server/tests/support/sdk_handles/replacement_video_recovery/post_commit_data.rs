//! Same signed post-commit Data command, real authority exclusion and delivery.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use connect_store::{AuthorityMode, AuthorityScope, AuthorityWait};
use futures_util::FutureExt;
use std::{panic::AssertUnwindSafe, sync::atomic::Ordering};
use tokio::sync::oneshot;

type TestResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
const OPERATION: &str = "postcommit-data-subscription";

async fn reached(
    probe: &std::sync::atomic::AtomicUsize,
    expected: usize,
    until: tokio::time::Instant,
) -> TestResult<()> {
    loop {
        let actual = probe.load(Ordering::Acquire);
        if actual == expected {
            return Ok(());
        }
        if actual > expected || tokio::time::Instant::now() >= until {
            return Err(format!(
                "original admission rendezvous expected={expected} actual={actual}"
            )
            .into());
        }
        tokio::time::sleep_until(until.min(tokio::time::Instant::now() + Duration::from_millis(1)))
            .await;
    }
}

/// The caller joins all original physical peers and encoder before asserting
/// this result. This child additionally always releases and joins its writer.
pub(super) async fn audit(
    f: &Fixture,
    done: &Value,
    original_scope: &CommandScope,
    capabilities: &Value,
    replacement: &mut CanonicalClient,
    retained: &sdk::Connection,
    original_cutoff: tokio::time::Instant,
) -> TestResult<()> {
    let mut release = None;
    let mut writer = None;
    let result = AssertUnwindSafe(async {
    let root: connect_client::canonical::payloads::OperationUpdated =
        serde_json::from_value(done.clone())?;
    let original = readiness::original_cutoff(f, &root);
    let until = original_cutoff.min(tokio::time::Instant::now() + Duration::from_secs(8));
    let mut scope = original_scope.clone();
    scope.operation_id = Some(OPERATION.into());
    // Start, private media preparation and activation have all completed.
    // Read that exact committed generation; the earlier preparation revision
    // cannot authorize a new post-commit subscription.
    let preparation = replacement.native_replacement_readiness()?;
    let activation = preparation.activation_from_completed(done)?;
    let stream_revision = activation.stream_revision;
    assert_eq!(done["result"]["readiness"]["stream_revision"], stream_revision);
    assert!(stream_revision > preparation.stream_revision());
    scope.expected_revision = Some(stream_revision);
    let probe = f
        .server
        .service
        .arm_canonical_preparation_registry_probe_for_test(&format!(
            "subscription-admission:{OPERATION}"
        ))?;
    let scene_probe=f.server.service.arm_canonical_preparation_registry_probe_for_test(
        &format!("subscription-scene:{OPERATION}"))?;

        assert_eq!(done["state"], "completed");
        assert_eq!(done["result"]["generation"], 2);
        let original_phases = readiness::original_video_phase_rows(f, &original.record_id);
        let request = tokio::time::timeout_at(until, replacement.send_value(
            "stream.subscribe",
            json!({"subscriptions":[{"strm_id":"feedback-monitor"},
                {"strm_id":"feedback-monitor-voice"},{"strm_id":"feedback-monitor-data"}]}),
            scope.clone(),
        )).await??;
        let mut reading = false;
        let mut reply_to = request;
        let mut offers = 0;
        let mut observed = None;
        let mut record = None;
        let mut completed = false;
        for _ in 0..128 {
            let frame = tokio::time::timeout_at(until, replacement.receive()).await??;
            if frame.kind() == "connection.offer" {
                offers += 1;
                assert_eq!(offers, 1, "one original parameter preparation");
                assert!(!reading);
                assert_eq!(frame.scope(rvoip_uctp::v2::Scope::Operation), Some(OPERATION));
                assert_eq!(frame.scope(rvoip_uctp::v2::Scope::Connection), scope.connection_id.as_deref());
                let p = &frame.original()["payload"];
                assert_eq!(p["substrate_setup"]["revision"], stream_revision + 1);
                let streams = p["substrate_setup"]["streams"].as_array().ok_or("prepared streams missing")?
                    .iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],
                        "direction":match s["direction"].as_str() {Some("sendonly")=>"recvonly",Some("recvonly")=>"sendonly",Some("sendrecv")=>"sendrecv",_=>panic!("unexpected prepared direction")},
                        "codec":s["codec"]})).collect::<Vec<_>>();
                let answer = serde_json::from_value(json!({"by_participant":p["by_participant"],
                    "substrate":"quic","capabilities":capabilities,
                    "streams_answered":streams,"substrate_setup":p["substrate_setup"]}))?;
                tokio::time::timeout_at(until, replacement.answer_connection_offer(&frame, answer, &scope)).await??;
                reached(&probe, 1, until).await?;
                // No original planning/reply reader remains at this exact
                // production boundary. This is a real scoped file writer.
                let store = f.server.store.clone();
                let cid = scope.conversation_id.clone().ok_or("missing conversation")?;
                let (entered_tx, entered_rx) = oneshot::channel();
                let (release_tx, release_rx) = oneshot::channel();
                release = Some(release_tx);
                writer = Some(tokio::task::spawn_blocking(move || {
                    let held = store.acquire_authority(
                        &[AuthorityScope::conversation(&cid, AuthorityMode::Write)?],
                        &AuthorityWait::new(until.saturating_duration_since(tokio::time::Instant::now())),
                    )?;
                    let _ = entered_tx.send(());
                    // Sender drop also releases on every failure/panic path.
                    let _ = release_rx.blocking_recv();
                    drop(held);
                    connect_protocol::Result::Ok(())
                }));
                tokio::time::timeout_at(until, entered_rx).await??;
                probe.store(2, Ordering::Release);
                reached(&probe, 3, until).await?;
                release.take().unwrap().send(()).map_err(|_|"original writer already ended")?;
                writer.take().unwrap().await??;
                // The SAME prepared parameter exchange now reaches fresh
                // policy admission. Exclude only its subsequent complete scene
                // projection with a real primary SQL owner; no new offer,
                // operation/effect ID, physical candidate, or cutoff is issued.
                reached(&scene_probe,1,until).await?;
                let store=f.server.store.clone();
                let(entered_tx,entered_rx)=oneshot::channel();
                let(release_tx,release_rx)=oneshot::channel();
                release=Some(release_tx);
                writer=Some(tokio::task::spawn_blocking(move ||store.with_primary_sql_held_for_test(|| {
                    let _=entered_tx.send(());let _=release_rx.blocking_recv();
                })));
                tokio::time::timeout_at(until,entered_rx).await??;
                scene_probe.store(2,Ordering::Release);
                // Phase3 requires the actual Store Busy AND release of the
                // exact freshly admitted scope set after all partial reads.
                reached(&scene_probe,3,until).await?;
                release.take().unwrap().send(()).map_err(|_|"original scene SQL owner already ended")?;
                writer.take().unwrap().await??;
                reached(&scene_probe,4,until).await?;
                continue;
            }
            assert_eq!(frame.scope(rvoip_uctp::v2::Scope::Reply), Some(reply_to.as_str()));
            match frame.kind() {
                "operation.updated" => {
                    assert!(observed.is_none(), "one update before its original ACK");
                    let p = &frame.original()["payload"];
                    assert_eq!(p["operation_id"], OPERATION);
                    assert_eq!(p["command_type"], "stream.subscribe");
                    assert_eq!(p["expected_revision"], stream_revision);
                    let id = p["record_id"].as_str().ok_or("missing original record")?;
                    if let Some(old) = &record { assert_eq!(old, id); }
                    record = Some(id.to_owned());
                    observed = Some(p.clone());
                }
                "ack" => {
                    let current = observed.take().ok_or("ACK without original operation")?;
                    match current["state"].as_str() {
                        Some("completed") => {
                            assert_eq!(current["result"]["connection_generation"], 2);
                            assert_eq!(current["result"]["stream_revision"], stream_revision + 1);
                            completed = true;
                            break;
                        }
                        Some("accepted"|"preparing"|"ready"|"committing") => {},
                        _ => return Err(format!("original subscription did not complete: {current}; offers={offers}; admission_probe={}",probe.load(Ordering::Acquire)).into()),
                    }
                    let mut read = scope.clone();read.operation_id=None;read.expected_revision=None;
                    tokio::time::sleep_until(until.min(tokio::time::Instant::now()+Duration::from_millis(20))).await;
                    reply_to = tokio::time::timeout_at(until, replacement.send_value(
                        "operation.get", json!({"operation_id":OPERATION}), read,
                    )).await??;
                    reading = true;
                }
                "error" => return Err(format!("original subscription wire refusal: {}",frame.original()["payload"]).into()),
                _ => return Err(format!("unexpected original frame {}",frame.kind()).into()),
            }
        }
        assert_eq!(offers, 1);
        assert!(completed, "original subscription response frame bound");
        assert_eq!(probe.load(Ordering::Acquire), 4);
        assert_eq!(scene_probe.load(Ordering::Acquire),4);
        let sql = rusqlite::Connection::open_with_flags(&f.config.database,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let (rows,effects,attempt,state,generation,participant):(u64,u64,u64,String,u64,String)=sql.query_row(
            "SELECT COUNT(DISTINCT o.record_id),COUNT(e.id),MAX(e.attempt),o.state,c.generation,c.participant_id FROM operations o JOIN operation_effects e ON e.operation_record_id=o.record_id JOIN connections c ON c.id=o.connection_id WHERE o.operation_id=?1 AND o.record_id=?2",
            rusqlite::params![OPERATION,record.as_deref().ok_or("missing completed record")?],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
        assert_eq!((rows,effects,attempt,state.as_str(),generation),(1,1,1,"completed",2));
        let confirmed:u64=sql.query_row("SELECT COUNT(*) FROM operation_effects WHERE operation_record_id=?1 AND state='confirmed'",[&original.record_id],|r|r.get(0))?;
        assert_eq!(confirmed,7,"original completed movement effect count changed");
        drop(sql);
        assert_eq!(readiness::original_video_phase_rows(f,&original.record_id),original_phases);
        readiness::assert_same_original_preparation(&original,&readiness::original_cutoff(f,&root));
        let recipient: sdk::ParticipantId = participant.parse()?;
        let bytes=(0..65536).map(|n|(n%251)as u8).collect::<Vec<_>>();
        let request = RequestOptions::new(until.saturating_duration_since(tokio::time::Instant::now()))?;
        let mut mutation = mutation("postcommit-data-publication");mutation.request=request;
        let operation = retained.stream_by_id("feedback-monitor-data".parse()?).send_data(
            "postcommit-data-message".parse()?, single(recipient),sdk::DataPayload::Binary(bytes.clone()),mutation,
        ).await?;
        let accepted=operation.wait(RequestOptions::new(until.saturating_duration_since(tokio::time::Instant::now()))?).await?;
        assert_eq!(accepted.value.message_id.as_str(),"postcommit-data-message");
        let delivered = tokio::time::timeout_at(until,replacement.receive()).await??;
        assert_eq!(delivered.kind(),"message.send");
        let p=&delivered.original()["payload"];
        assert_eq!(p["msg_id"],"postcommit-data-message");
        assert_eq!(STANDARD.decode(p["body"].as_str().ok_or("binary body missing")?)?,bytes);
        assert!(tokio::time::Instant::now()<until);
        Ok::<(),Box<dyn std::error::Error+Send+Sync>>(())
    }).catch_unwind().await;
    drop(release.take());
    let joined: TestResult<()> = if let Some(writer) = writer.take() {
        match writer.await {
            Ok(value) => value.map_err(Into::into),
            Err(error) => Err(error.into()),
        }
    } else {
        Ok(())
    };
    match result {
        Err(original) => {
            if let Err(error) = joined {
                eprintln!("secondary original postcommit SQL join failure: {error}");
            }
            std::panic::resume_unwind(original)
        }
        Ok(Err(original)) => {
            if let Err(error) = joined {
                eprintln!("secondary original postcommit SQL join failure: {error}");
            }
            Err(original)
        }
        Ok(Ok(value)) => {
            joined?;
            Ok(value)
        }
    }
}
