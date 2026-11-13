//! Exact one-submission reliable Data and current-owner end on actual peers.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
#[path = "representative_end_facts.rs"]
mod end_facts;

fn capabilities() -> Value {
    json!({"audio_codecs":[{"name":"opus","params":{"sample_rate":48000,"channels":1}}],
        "video_codecs":[{"name":"vp8","params":{"sample_rate":90000,"channels":0}}],
        "data_protocols":["binary"],"max_streams_per_connection":8,"video_feedback":"rtcp-pli-epoch/1"})
}

// One immutable original command only. An accepted/preparing observation is
// recovered using operation.get for the same ID; an error/unknown fails, never
// a second mutation, allocation, answer, or inferred completed operation.
async fn command(
    client: &mut CanonicalClient,
    kind: &str,
    payload: Value,
    scope: CommandScope,
    until: Instant,
) -> Value {
    let original_operation = scope.operation_id.as_ref().unwrap().clone();
    let mut reply = timeout_at(until, client.send_value(kind, payload, scope.clone()))
        .await
        .unwrap()
        .unwrap();
    let mut observed = None;
    let mut record = None;
    let mut offers = 0;
    let mut facts = 0;
    loop {
        assert!(
            Instant::now() < until,
            "original command deadline exhausted before next frame"
        );
        let frame = timeout_at(until, client.receive()).await.unwrap().unwrap();
        assert!(
            Instant::now() < until,
            "original command deadline exhausted after receive"
        );
        if frame.kind() == "connection.offer" {
            offers += 1;
            assert_eq!(offers, 1, "one original parameter preparation");
            assert_eq!(
                frame.scope(rvoip_uctp::v2::Scope::Operation),
                Some(original_operation.as_str())
            );
            let p = &frame.original()["payload"];
            let streams = p["substrate_setup"]["streams"].as_array().unwrap().iter().map(|s|json!({
                "id":s["strm_id"],"kind":s["kind"],"codec":s["codec"],"direction":match s["direction"].as_str() {
                    Some("sendonly")=>"recvonly",Some("recvonly")=>"sendonly",Some("sendrecv")=>"sendrecv",_=>panic!("unexpected prepared direction")
                }})).collect::<Vec<_>>();
            let answer = serde_json::from_value(json!({"by_participant":p["by_participant"],"substrate":"quic",
                "capabilities":capabilities(),"streams_answered":streams,"substrate_setup":p["substrate_setup"]})).unwrap();
            timeout_at(
                until,
                client.answer_connection_offer(&frame, answer, &scope),
            )
            .await
            .unwrap()
            .unwrap();
            continue;
        }
        match end_facts::classify(frame.original(),kind,&scope,&reply) {
            Ok(end_facts::Class::Reply)=>{},
            Ok(end_facts::Class::Live(name))=>{
                facts+=1;assert!(facts<=32,"original current-End live-fact bound");
                eprintln!("representative current-End live fact kind={name} cid={:?} sid={:?} connection={:?} operation={:?}; correlated_result_credit=false",
                    frame.scope(rvoip_uctp::v2::Scope::Conversation),frame.scope(rvoip_uctp::v2::Scope::Session),
                    frame.scope(rvoip_uctp::v2::Scope::Connection),frame.scope(rvoip_uctp::v2::Scope::Operation));
                continue;
            }
            Err(reason)=>panic!("original command correlation refused reason={reason} command={kind} original_operation={original_operation} current_request={reply} frame_kind={} frame_scope_cid={:?} frame_scope_sid={:?} frame_scope_connection={:?} frame_scope_operation={:?} frame_reply={:?} payload={}",
                frame.kind(),frame.scope(rvoip_uctp::v2::Scope::Conversation),frame.scope(rvoip_uctp::v2::Scope::Session),
                frame.scope(rvoip_uctp::v2::Scope::Connection),frame.scope(rvoip_uctp::v2::Scope::Operation),
                frame.scope(rvoip_uctp::v2::Scope::Reply),end_facts::diagnostic(frame.original())),
        }
        match frame.kind() {
            "operation.updated" => {
                assert!(observed.is_none());
                let p = &frame.original()["payload"];
                assert_eq!(p["operation_id"], original_operation);
                assert_eq!(p["command_type"], kind);
                let id = p["record_id"].as_str().unwrap();
                if let Some(old) = &record {
                    assert_eq!(old, id);
                }
                record = Some(id.to_owned());
                observed = Some(p.clone());
            }
            "ack" => {
                let value = observed
                    .take()
                    .expect("original update before matching ACK");
                match value["state"].as_str() {
                    Some("completed") => return value,
                    Some("accepted" | "preparing" | "ready" | "committing") => {}
                    _ => panic!("original {kind} did not complete: {}", value["state"]),
                }
                let mut read = scope.clone();
                read.operation_id = None;
                read.expected_revision = None;
                tokio::time::sleep_until(until.min(Instant::now() + Duration::from_millis(20)))
                    .await;
                reply = timeout_at(
                    until,
                    client.send_value(
                        "operation.get",
                        json!({"operation_id":original_operation}),
                        read,
                    ),
                )
                .await
                .unwrap()
                .unwrap();
            }
            "error" => panic!("original {kind} refused: {}", frame.original()["payload"]),
            other => panic!("unexpected original {kind} reply {other}"),
        }
    }
}

pub(super) async fn exchange(
    client: &mut CanonicalClient,
    movement: &TransitionCommand,
    target_id: String,
    committed_revision: u64,
    upstream: &sdk::Connection,
    downstream: &sdk::Connection,
    target_participant: &sdk::ParticipantId,
    downstream_participant: &sdk::ParticipantId,
    index: usize,
    phase: &str,
    until: Instant,
) {
    // Capture the original root/candidate from its completed record, not a new
    // generation inferred from a successful Data command.
    assert!(!target_id.is_empty());
    assert_ne!(target_participant, downstream_participant);
    let data_id = format!("representative-{phase}-data-{index}");
    let original_scope = CommandScope {
        conversation_id: Some(movement.conversation_id.clone()),
        session_id: Some(movement.session_id.clone()),
        connection_id: Some(target_id.clone()),
        operation_id: None,
        expected_revision: None,
    };
    let mut scope = original_scope.clone();
    scope.operation_id = Some(format!("representative-{phase}-data-unmute-{index}"));
    assert!(
        committed_revision > 2,
        "validated completed root must advance its private preparation revision"
    );
    scope.expected_revision = Some(committed_revision);
    let unmuted = command(
        client,
        "connection.update",
        json!({"action":"unmute","streams":[data_id]}),
        scope,
        until,
    )
    .await;
    assert_eq!(
        unmuted["result"]["stream_revision"],
        committed_revision + 1,
        "actual one original gate mutation"
    );
    let revision = unmuted["result"]["stream_revision"]
        .as_u64()
        .expect("actual post-unmute stream revision");
    let mut scope = original_scope.clone();
    scope.operation_id = Some(format!("representative-{phase}-data-select-{index}"));
    scope.expected_revision = Some(revision);
    command(client,"stream.subscribe",json!({"subscriptions":[{"strm_id":format!("representative-data-{}",SOURCE_FOR_RECEIVER[index])}]}),scope,until).await;
    let before = downstream.refresh(programme::request(until)).await.unwrap();
    downstream
        .subscribe(
            single(StreamSelector::Stream(data_id.parse().unwrap())),
            before.stream_revision.unwrap(),
            programme::mutation_at(
                &format!("representative-{phase}-downstream-select-{index}"),
                until,
            ),
        )
        .await
        .unwrap()
        .wait(programme::request(until))
        .await
        .unwrap();

    let incoming = (0..65536)
        .map(|n| ((n + index) % 251) as u8)
        .collect::<Vec<_>>();
    let outgoing = (0..65536)
        .map(|n| ((n * 3 + index + 1) % 251) as u8)
        .collect::<Vec<_>>();
    assert_ne!(incoming, outgoing);
    let incoming_message = format!("representative-{phase}-incoming-message-{index}");
    let incoming_operation = format!("representative-{phase}-incoming-data-{index}");
    let accepted = upstream
        .stream_by_id(
            format!("representative-data-{}", SOURCE_FOR_RECEIVER[index])
                .parse()
                .unwrap(),
        )
        .send_data(
            incoming_message.parse().unwrap(),
            single(target_participant.clone()),
            sdk::DataPayload::Binary(incoming.clone()),
            programme::mutation_at(&incoming_operation, until),
        )
        .await
        .unwrap()
        .wait(programme::request(until))
        .await
        .unwrap();
    assert_eq!(accepted.value.message_id.as_str(), incoming_message);
    assert_eq!(
        accepted.value.boundary,
        sdk::DataAcceptanceBoundary::CoreQueueAdmission
    );
    let delivered = timeout_at(until, client.receive()).await.unwrap().unwrap();
    assert_eq!(delivered.kind(), "message.send");
    assert_eq!(
        delivered.scope(rvoip_uctp::v2::Scope::Connection),
        Some(target_id.as_str())
    );
    let p = &delivered.original()["payload"];
    assert_eq!(p["msg_id"], incoming_message);
    assert_eq!(
        STANDARD.decode(p["body"].as_str().unwrap()).unwrap(),
        incoming
    );

    let outgoing_message = format!("representative-{phase}-outgoing-message-{index}");
    let mut scope = original_scope.clone();
    scope.operation_id = Some(format!("representative-{phase}-outgoing-data-{index}"));
    let admitted = command(client,"message.send",json!({"msg_id":outgoing_message,"label":data_id,
        "to":[downstream_participant],"content_type":"application/octet-stream","body_encoding":"base64",
        "body":STANDARD.encode(&outgoing),"reliability":{"mode":"reliable_ordered"}}),scope,until).await;
    assert_eq!(admitted["result"]["core_queue_admission"], true);
    let received = downstream
        .stream_by_id(data_id.parse().unwrap())
        .receive_data(programme::request(until))
        .await
        .unwrap();
    assert_eq!(received.message_id.as_str(), outgoing_message);
    assert_eq!(received.from, target_participant.clone());
    assert_eq!(received.payload, sdk::DataPayload::Binary(outgoing));
    assert!(Instant::now() < until, "same original Data cutoff");
    eprintln!("representative bidirectional-data index={index} phase={phase} incoming_exact_bytes=65536 outgoing_exact_bytes=65536 mutation_replays=0");
}

// Numeric fixture diagnostics only. This wrapper polls the SAME original
// future, returns its exact output and does not catch a panic, spawn an owner,
// cancel a request, create a new timeout or alter the original cutoff.
async fn trace_current_end_phase<F: std::future::Future>(
    index: usize,
    phase: &'static str,
    began: Instant,
    until: Instant,
    original: F,
) -> F::Output {
    let entered = Instant::now();
    eprintln!("representative current-End index={index} phase={phase} stage=entry programme_us={} remaining_us={}",
        entered.saturating_duration_since(began).as_micros(), until.saturating_duration_since(entered).as_micros());
    let result = original.await;
    let returned = Instant::now();
    eprintln!("representative current-End index={index} phase={phase} stage=returned programme_us={} elapsed_us={} remaining_us={}",
        returned.saturating_duration_since(began).as_micros(), returned.saturating_duration_since(entered).as_micros(), until.saturating_duration_since(returned).as_micros());
    result
}

pub(super) async fn end_current(
    originals: &[sdk::Connection],
    targets: &mut [Candidate],
    current: &[Option<usize>; 2],
    sessions: &[sdk::Session],
    until: Instant,
    began: Instant,
) {
    // The caller has reached the actual62s source boundary and issued stop to
    // every original packet owner. All six current original mutations start
    // once at that boundary and share its original67s retirement cutoff.
    let first_failure = Mutex::new(None);
    let first = &first_failure;
    let retained = async {
        let mut submissions = Vec::new();
        for index in [1usize, 2, 3, 5] {
            let original = originals[index].clone();
            submissions.push(settled(first, async move {
                let before = trace_current_end_phase(
                    index,
                    "retained-refresh",
                    began,
                    until,
                    original.refresh(programme::request(until)),
                )
                .await
                .unwrap();
                let operation = trace_current_end_phase(
                    index,
                    "retained-end",
                    began,
                    until,
                    original.end(
                        200,
                        "Synthetic workload complete".into(),
                        None,
                        programme::mutation_at(
                            &format!("representative-current-end-{index}"),
                            until,
                        ),
                    ),
                )
                .await
                .unwrap();
                trace_current_end_phase(
                    index,
                    "retained-wait",
                    began,
                    until,
                    operation.wait(programme::request(until)),
                )
                .await
                .unwrap();
                (original, before.generation)
            }));
        }
        futures_util::future::join_all(submissions).await
    };
    let moving = async {
        // These two exact original physical clients are independent owners.
        // split_at_mut prevents aliasing/duplicate raw receive drivers.
        let a = current[0].unwrap();
        let b = current[1].unwrap();
        assert!(a < b);
        let (left, right) = targets.split_at_mut(b);
        let (a, b) = tokio::join!(
            settled(
                first,
                end_moving(&mut left[a], 0, &sessions[0], until, began)
            ),
            settled(
                first,
                end_moving(&mut right[0], 4, &sessions[1], until, began)
            )
        );
        vec![a, b]
    };
    let (retained, moving) = tokio::join!(retained, moving);
    // Every original command caller settles before the exact first failure
    // escapes. A failed sibling cannot drop another original End/wait future.
    let original_failure = first_failure.into_inner().unwrap();
    if let Some(error) = original_failure {
        std::panic::resume_unwind(error);
    }
    let expected = retained
        .into_iter()
        .chain(moving)
        .map(|value| value.expect("original End completed"))
        .collect::<Vec<_>>();
    assert_eq!(expected.len(), 6);
    timeout_at(until, async {
        loop {
            // Current public snapshots, not successful End receipts or
            // fixture SQL, prove each exact current owner actually retired.
            let observations = futures_util::future::join_all(expected.iter().enumerate().map(
                |(position, (connection, generation))| async move {
                    // Diagnostic route order only: the four retained entries,
                    // followed by the two moving entries, exactly as collected above.
                    let index = [1usize, 2, 3, 5, 0, 4][position];
                    let current = trace_current_end_phase(
                        index,
                        "verify-retired",
                        began,
                        until,
                        connection.refresh(programme::request(until)),
                    )
                    .await
                    .unwrap();
                    assert_eq!(&current.id, connection.id());
                    assert_eq!(&current.session_id, connection.session_id());
                    current.state == "retired" && current.generation > *generation
                },
            ))
            .await;
            let all = observations.into_iter().all(|retired| retired);
            if all {
                break;
            }
            tokio::time::sleep_until(until.min(Instant::now() + Duration::from_millis(20))).await;
        }
    })
    .await
    .expect("all six actual current owners retire by original67s cutoff");
}

async fn end_moving(
    target: &mut Candidate,
    index: usize,
    session: &sdk::Session,
    until: Instant,
    began: Instant,
) -> (sdk::Connection, u64) {
    let preparation = target.preparation.as_ref().unwrap();
    let id = preparation.connection_id().unwrap().to_owned();
    let generation = preparation.target_generation();
    let scope = CommandScope {
        conversation_id: Some(session.conversation_id().as_str().into()),
        session_id: Some(session.id().as_str().into()),
        connection_id: Some(id.clone()),
        operation_id: Some(format!("representative-current-end-{index}")),
        expected_revision: None,
    };
    trace_current_end_phase(
        index,
        "moving-command",
        began,
        until,
        command(
            &mut target.client,
            "connection.end",
            json!({"reason_code":200,"reason":"Synthetic workload complete"}),
            scope,
            until,
        ),
    )
    .await;
    (session.connection_by_id(id.parse().unwrap()), generation)
}

async fn settled<F, T>(first: &Mutex<Option<Box<dyn std::any::Any + Send>>>, future: F) -> Option<T>
where
    F: std::future::Future<Output = T>,
{
    match AssertUnwindSafe(future).catch_unwind().await {
        Ok(value) => Some(value),
        Err(error) => {
            let mut recorded = first.lock().unwrap();
            if recorded.is_none() {
                *recorded = Some(error);
            }
            None
        }
    }
}

/// Failure-only scalar journal observation of the exact original key; never a
/// status-based mutation retry or an inference that an absent row is safe.
pub(super) fn selection_failure(
    f: &Fixture,
    connection: &sdk::Connection,
    operation: &str,
    stage: &str,
    error: &sdk::Error,
) {
    let observed = (|| -> rusqlite::Result<Value> {
        let db = rusqlite::Connection::open_with_flags(
            &f.config.database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        db.busy_timeout(Duration::ZERO)?;
        let mut statement=db.prepare("SELECT record_id,state,revision,expected_revision,result_json,problem_json FROM operations WHERE operation_id=?1 AND conversation_id=?2 AND session_id=?3 AND connection_id=?4 ORDER BY record_id LIMIT 8")?;
        let rows = statement
            .query_map(
                rusqlite::params![
                    operation,
                    connection.conversation_id().as_str(),
                    connection.session_id().as_str(),
                    connection.id().as_str()
                ],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, u64>(2)?,
                        r.get::<_, Option<u64>>(3)?,
                        r.get::<_, Option<String>>(4)?,
                        r.get::<_, Option<String>>(5)?,
                    ))
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut values = Vec::new();
        for (record, state, revision, expected, result, problem) in rows {
            let mut effects=db.prepare("SELECT id,provider,state,attempt,submission_generation FROM operation_effects WHERE operation_record_id=?1 ORDER BY id LIMIT 16")?;
            let effects=effects.query_map([&record],|r|Ok(json!({"id":r.get::<_,String>(0)?,"provider":r.get::<_,String>(1)?,"state":r.get::<_,String>(2)?,"attempt":r.get::<_,u64>(3)?,"submission_generation":r.get::<_,u64>(4)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
            values.push(json!({"record_id":record,"state":state,"revision":revision,"expected_revision":expected,"result_json":result,"problem_json":problem,"effects":effects}));
        }
        Ok(
            json!({"original_records":values,"scope":{"conversation":connection.conversation_id(),"session":connection.session_id(),"connection":connection.id()}}),
        )
    })();
    eprintln!("representative original initial selection failure operation={operation} stage={stage} kind={:?} certainty={:?} problem={:?} original_observation={:?} bounded_journal={observed:?}",error.kind,error.certainty,error.problem,error.observation);
}
