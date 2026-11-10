//! Six actual signed peers, one withheld original preparation and a real
//! command-scoped Read WouldBlock. No mutation is retried by this fixture.
use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc, Arc,
};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

type TestResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
const FIRST: &str = "overlap-first-subscription";
const SECOND: &str = "overlap-second-subscription";

#[tokio::test]
async fn signed_six_native_setup_eddsa_subscriptions_overlap_preparation_and_real_writer() {
    exercise("EdDSA", false).await;
}
#[tokio::test]
async fn signed_six_native_setup_es256_subscriptions_overlap_preparation_and_real_writer() {
    exercise("ES256", false).await;
}
#[tokio::test]
async fn signed_waiting_eddsa_subscription_rechecks_video_withdrawal_without_preparation_replay() {
    exercise("EdDSA", true).await;
}
#[tokio::test]
async fn signed_waiting_es256_subscription_rechecks_video_withdrawal_without_preparation_replay() {
    exercise("ES256", true).await;
}

fn capabilities() -> Value {
    let mut capabilities = av_capabilities();
    capabilities["data_protocols"] = json!(["binary"]);
    capabilities
}
fn scope(cid: &str, target: usize, operation: &str) -> CommandScope {
    CommandScope {
        conversation_id: Some(cid.into()),
        session_id: Some("native-session".into()),
        connection_id: Some(format!("overlap-connection-{target}")),
        operation_id: Some(operation.into()),
        expected_revision: Some(2),
    }
}
fn selection() -> Value {
    json!({"subscriptions":[{"strm_id":"overlap-audio-0"},
        {"strm_id":"overlap-video-0"},{"strm_id":"overlap-data-0"}]})
}
fn resource_scope(frame: &Envelope, scope: &CommandScope, reading: bool) -> bool {
    frame.scope(rvoip_uctp::v2::Scope::Conversation) == scope.conversation_id.as_deref()
        && frame.scope(rvoip_uctp::v2::Scope::Session) == scope.session_id.as_deref()
        && frame.scope(rvoip_uctp::v2::Scope::Connection) == scope.connection_id.as_deref()
        && frame.scope(rvoip_uctp::v2::Scope::Operation)
            == if reading {
                None
            } else {
                scope.operation_id.as_deref()
            }
}
#[derive(Debug)]
struct Outcome {
    state: Option<String>,
    problem: Option<Value>,
    record: Option<String>,
    offers: usize,
    result: Option<Value>,
}

async fn original(
    client: &mut CanonicalClient,
    scope: CommandScope,
    until: tokio::time::Instant,
    mut prepared: Option<(oneshot::Sender<()>, oneshot::Receiver<()>)>,
    start: Option<oneshot::Receiver<()>>,
) -> TestResult<Outcome> {
    if let Some(start) = start {
        tokio::time::timeout_at(until, start).await??;
    }
    // This is the only mutation submission. All later sends below are read-only.
    let original = tokio::time::timeout_at(
        until,
        client.send_value("stream.subscribe", selection(), scope.clone()),
    )
    .await??;
    let mut reply_to = original.clone();
    let mut reading = false;
    let operation = scope
        .operation_id
        .as_deref()
        .ok_or("missing original subscription ID")?;
    let mut out = Outcome {
        state: None,
        problem: None,
        record: None,
        offers: 0,
        result: None,
    };
    let mut revision = 0;
    let mut response_seen = false;
    let mut frames = 0;
    loop {
        if tokio::time::Instant::now() >= until {
            return Err("same original control cutoff".into());
        }
        let frame = tokio::time::timeout_at(until, client.receive_control()).await??;
        frames += 1;
        if frames > 128 {
            return Err("original response frame bound".into());
        }
        if frame.kind() == "connection.offer" {
            if reading || !resource_scope(&frame, &scope, false) || out.offers != 0 {
                return Err("new or foreign preparation for original subscription".into());
            }
            out.offers += 1;
            if let Some((entered, release)) = prepared.take() {
                let _ = entered.send(());
                // No original authorization lives in this fixture channel.
                // Even a failed coordinator closes it and lets original cleanup run.
                let _ = tokio::time::timeout_at(until, release).await?;
            }
            let p = &frame.original()["payload"];
            let streams = p["substrate_setup"]["streams"].as_array().ok_or("missing prepared streams")?
                .iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],
                    "direction":match s["direction"].as_str() {Some("sendonly")=>"recvonly",Some("recvonly")=>"sendonly",_=>"sendrecv"},"codec":s["codec"]})).collect::<Vec<_>>();
            let answer = serde_json::from_value(json!({"by_participant":p["by_participant"],
                "substrate":"quic","capabilities":capabilities(),"streams_answered":streams,
                "substrate_setup":p["substrate_setup"]}))?;
            tokio::time::timeout_at(
                until,
                client.answer_connection_offer(&frame, answer, &scope),
            )
            .await??;
            continue;
        }
        if frame.scope(rvoip_uctp::v2::Scope::Reply) != Some(reply_to.as_str())
            || !resource_scope(&frame, &scope, reading)
        {
            return Err("original subscription response correlation changed".into());
        }
        if frame.kind() == "error" {
            let p = &frame.original()["payload"];
            out.problem =
                Some(json!({"code":p["code"],"category":p["category"],"reason":p["reason"]}));
            return Ok(out);
        }
        match frame.kind() {
            "operation.updated" if !response_seen => {
                let p = &frame.original()["payload"];
                let current_revision = p["revision"].as_u64().ok_or("missing original revision")?;
                let record = p["record_id"]
                    .as_str()
                    .filter(|r| !r.is_empty())
                    .ok_or("missing original record")?;
                if p["operation_id"] != operation
                    || p["command_type"] != "stream.subscribe"
                    || p["expected_revision"] != 2
                    || current_revision < revision
                    || out.record.as_deref().is_some_and(|old| old != record)
                {
                    return Err("original operation/record/precondition/revision changed".into());
                }
                revision = current_revision;
                out.record = Some(record.into());
                out.state = p["state"].as_str().map(str::to_owned);
                out.problem = p.get("problem").cloned();
                out.result = p.get("result").cloned();
                response_seen = true;
            }
            "ack" if response_seen => {
                match out.state.as_deref() {
                    Some("completed" | "failed" | "cancelled" | "interrupted" | "unknown") => {
                        return Ok(out)
                    }
                    Some("accepted" | "preparing" | "ready" | "committing") => {}
                    _ => return Err("unsupported original state".into()),
                }
                tokio::time::sleep_until(
                    (tokio::time::Instant::now() + Duration::from_millis(20)).min(until),
                )
                .await;
                if tokio::time::Instant::now() >= until {
                    return Err("same original control cutoff".into());
                }
                let mut read = scope.clone();
                read.operation_id = None;
                read.expected_revision = None;
                reply_to = tokio::time::timeout_at(
                    until,
                    client.send_value("operation.get", json!({"operation_id":operation}), read),
                )
                .await??;
                reading = true;
                frames = 0;
                response_seen = false;
            }
            _ => return Err("unexpected original subscription response".into()),
        }
    }
}

// Only bounded readonly evidence, never the public mutation or a recovery call.
fn counts(f: &Fixture, operation: &str) -> TestResult<(u64, u64, u64)> {
    let c = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    c.busy_timeout(Duration::ZERO)?;
    Ok(c.query_row("SELECT COUNT(DISTINCT o.record_id),COUNT(e.id),COALESCE(MAX(e.attempt),0) FROM operations o LEFT JOIN operation_effects e ON e.operation_record_id=o.record_id WHERE o.operation_id=?1", [operation], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?)
}
#[derive(Debug)]
struct History {
    record: String,
    state: String,
    problem: Option<Value>,
    result: Option<Value>,
    effects: Vec<(String, String, u64)>,
}
fn history(
    f: &Fixture,
    cid: &str,
    operation: &str,
    connection: &str,
) -> TestResult<Option<History>> {
    use rusqlite::OptionalExtension;
    let mut c = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    c.busy_timeout(Duration::ZERO)?;
    let tx = c.transaction()?;
    let row:Option<(String,String,Option<String>,Option<String>,String,String,String,String,Option<u64>)>=tx.query_row(
        "SELECT record_id,state,problem_json,result_json,command_type,conversation_id,session_id,connection_id,expected_revision FROM operations WHERE operation_id=?1",
        [operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).optional()?;
    let Some((record, state, problem, result, command, room, session, target, revision)) = row
    else {
        return Ok(None);
    };
    if command != "stream.subscribe"
        || room != cid
        || session != "native-session"
        || target != connection
        || revision != Some(2)
    {
        return Err("original subscription durable identity changed".into());
    }
    let effects = {
        let mut query=tx.prepare("SELECT id,state,attempt FROM operation_effects WHERE operation_record_id=?1 ORDER BY id")?;
        let rows = query
            .query_map([&record], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    tx.commit()?;
    Ok(Some(History {
        record,
        state,
        problem: problem.map(|v| serde_json::from_str(&v)).transpose()?,
        result: result.map(|v| serde_json::from_str(&v)).transpose()?,
        effects,
    }))
}
fn video_withdrawn_audio_current(f: &Fixture) -> TestResult<bool> {
    let c = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    c.busy_timeout(Duration::ZERO)?;
    let mut q=c.prepare("SELECT cap.capabilities_json FROM connections n JOIN session_presence_capabilities cap ON cap.session_id=n.session_id AND cap.participant_id=n.participant_id AND cap.membership_epoch=n.membership_epoch WHERE n.id IN ('overlap-connection-1','overlap-connection-2') ORDER BY n.id")?;
    let rows = q
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if rows.len() != 2 {
        return Ok(false);
    }
    for row in rows {
        let value: Value = serde_json::from_str(&row)?;
        if value.get("video").is_some()
            || !value["audio"]
                .as_array()
                .is_some_and(|audio| !audio.is_empty())
        {
            return Ok(false);
        }
    }
    Ok(true)
}
async fn withdrawn_history(
    f: &Fixture,
    cid: &str,
    until: tokio::time::Instant,
) -> TestResult<(History, Option<History>)> {
    loop {
        if tokio::time::Instant::now() >= until {
            return Err("same original withdrawal recording cutoff".into());
        }
        let first = history(f, cid, FIRST, "overlap-connection-1")?
            .ok_or("original prepared subscription disappeared")?;
        let second = history(f, cid, SECOND, "overlap-connection-2")?;
        if first.state == "unknown"
            && second
                .as_ref()
                .is_none_or(|record| matches!(record.state.as_str(), "failed" | "unknown"))
        {
            return Ok((first, second));
        }
        if second.as_ref().is_some_and(|record| {
            !matches!(
                record.state.as_str(),
                "accepted" | "preparing" | "failed" | "unknown"
            )
        }) {
            return Err("waiting original entered an unsupported withdrawal outcome".into());
        }
        if !matches!(first.state.as_str(), "preparing" | "unknown") {
            return Err(
                "prepared original must settle conservative Unknown, never another outcome".into(),
            );
        }
        tokio::time::sleep_until(
            (tokio::time::Instant::now() + Duration::from_millis(20)).min(until),
        )
        .await;
    }
}
fn public_problem(out: &Outcome, code: u64, category: &str, reason: &str) -> bool {
    out.problem
        .as_ref()
        .is_some_and(|p| p["code"] == code && p["category"] == category && p["reason"] == reason)
}
fn assert_withdrawal(
    first: &Outcome,
    second: &Outcome,
    initial: &(String, String),
    evidence: (History, Option<History>),
) {
    // Fresh invocation refusal maps forbidden; a Core selected-plan refusal
    // maps reconciliation-required. Neither branch permits transient503,
    // timeout, authentication failure, arbitrary Failed or missing evidence.
    assert!(
        public_problem(first, 403, "auth", "forbidden")
            || public_problem(first, 409, "state", "reconciliation_required"),
        "prepared original must return its semantic fresh-current/activation refusal: {first:?}"
    );
    let (record, pending) = evidence;
    assert_eq!(record.record, initial.0);
    assert_eq!(record.state, "unknown");
    assert!(record.result.is_none());
    assert_eq!(
        record.effects,
        vec![(initial.1.clone(), "unknown".into(), 1)]
    );
    if let Some(pending) = pending {
        if pending.state == "unknown" {
            // Target policy is checked at Core staging as well as preflight.
            // Store may conservatively reserve the original once-only attempt
            // before that current permission refusal; it must emit NO offer.
            assert!(
                public_problem(second, 409, "state", "reconciliation_required")
                    || public_problem(second, 403, "auth", "forbidden"),
                "waiting current Core policy refusal must be semantic, not transient: {second:?}"
            );
            assert_eq!(pending.effects.len(), 1);
            assert_eq!(
                (pending.effects[0].1.as_str(), pending.effects[0].2),
                ("unknown", 1)
            );
            assert!(pending.result.is_none());
            assert_ne!(pending.record, record.record);
            if let Some(observed) = &second.record {
                assert_eq!(observed, &pending.record);
            }
            return;
        }
        assert_eq!(
            pending.state, "failed",
            "waiting original must be definitively refused before an effect"
        );
        assert!(pending.effects.is_empty() && pending.result.is_none());
        let code = pending
            .problem
            .as_ref()
            .and_then(|p| p["code"].as_str())
            .expect("original failed policy problem");
        assert!(
            matches!(code, "forbidden" | "incompatible-capabilities"),
            "unexpected current-policy failure: {pending:?}"
        );
        assert_ne!(pending.record, record.record);
        if let Some(observed) = &second.record {
            assert_eq!(observed, &pending.record);
        }
        if second.state.as_deref() == Some("failed")
            && second
                .problem
                .as_ref()
                .is_some_and(|p| p["code"].as_str() == Some(code))
        {
            // Actual original operation.updated Failed+ACK, not a new command.
        } else {
            let expected = if code == "forbidden" {
                (403, "auth", "forbidden")
            } else {
                (488, "media", "incompatible_capabilities")
            };
            assert!(
                public_problem(second, expected.0, expected.1, expected.2),
                "waiting policy refusal must match original failed record: {second:?}"
            );
        }
    } else {
        assert_eq!(second.state, None);
        assert_eq!(second.record, None);
        assert!(
            public_problem(second, 403, "auth", "forbidden"),
            "pre-admission Video refusal must be semantic403, never503: {second:?}"
        );
    }
}

fn live_revisions(f: &Fixture) -> TestResult<Vec<(bool, u64, u64)>> {
    let c = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    c.busy_timeout(Duration::ZERO)?;
    ["overlap-connection-1","overlap-connection-2"].into_iter().map(|id|
        Ok(c.query_row("SELECT state IN ('ready','active'),generation,json_extract(readiness_json,'$.stream_revision') FROM connections WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?)
    ).collect()
}
async fn audio_progress(counter: &[AtomicUsize; 2], until: tokio::time::Instant) -> TestResult<()> {
    let before = counter
        .each_ref()
        .map(|counter| counter.load(Ordering::Acquire));
    loop {
        if counter
            .iter()
            .zip(before)
            .all(|(counter, before)| counter.load(Ordering::Acquire) > before)
        {
            return Ok(());
        }
        if tokio::time::Instant::now() >= until {
            return Err("both original Audio routes must progress inside original cutoff".into());
        }
        tokio::time::sleep_until(
            (tokio::time::Instant::now() + Duration::from_millis(5)).min(until),
        )
        .await;
    }
}

async fn coordinate(
    f: &Fixture,
    cid: &str,
    until: tokio::time::Instant,
    withdraw: bool,
    entered: oneshot::Receiver<()>,
    release_first: oneshot::Sender<()>,
    start_second: oneshot::Sender<()>,
    audio: Arc<[AtomicUsize; 2]>,
    probe_request: connect_store::PeerAuthorityRequest,
) -> TestResult<(String, String)> {
    let mut owner = None;
    let result: TestResult<(String, String)> = async {
        tokio::time::timeout_at(until, entered).await??;
        audio_progress(&audio, until).await?; // First real offer is still unanswered.
        if counts(f, FIRST)? != (1, 1, 1) {
            return Err("first original preparation not owned exactly once".into());
        }
        if live_revisions(f)? != vec![(true, 1, 2), (true, 1, 2)] {
            return Err("unanswered preparation changed original live bindings".into());
        }
        let original = history(f, cid, FIRST, "overlap-connection-1")?
            .ok_or("first original record missing")?;
        if original.effects.len() != 1 || original.effects[0].2 != 1 {
            return Err("first original effect missing".into());
        }
        let identity = (original.record, original.effects[0].0.clone());
        let (signal, waited) = mpsc::channel();
        let _probe = f
            .server
            .store
            .observe_peer_read_wait_for_test(probe_request, signal)?;
        let (release, released) = mpsc::channel::<bool>();
        let (held, holding) = oneshot::channel();
        let store = f.server.store.clone();
        let room = cid.to_owned();
        let actor_identity = f.client.identity_id.clone();
        let writer = tokio::task::spawn_blocking(move || -> connect_protocol::Result<()> {
            let mut scopes = vec![AuthorityScope::conversation(&room, AuthorityMode::Write)?];
            if withdraw {
                scopes.push(AuthorityScope::identity(
                    &actor_identity,
                    AuthorityMode::Write,
                )?);
            }
            let guard = store
                .acquire_authority(&scopes, &AuthorityWait::new(Duration::from_millis(500)))?;
            let _ = held.send(());
            // Drop/disconnect also releases this SAME original holder on failure.
            let remaining = until
                .into_std()
                .saturating_duration_since(std::time::Instant::now());
            let released = released.recv_timeout(remaining);
            if matches!(released, Err(mpsc::RecvTimeoutError::Timeout)) {
                drop(guard);
                return Err(connect_protocol::Error::new(
                    "cancelled",
                    "Original test writer cutoff",
                ));
            }
            // Cleanup disconnect releases custody without another fixture
            // mutation. Only the proven actual wait authorizes this test cut.
            if withdraw && released == Ok(true) {
                store.withdraw_native_session_video_for_test(&guard, "overlap-connection-2")?;
            }
            drop(guard);
            Ok(())
        });
        owner = Some((release, writer));
        tokio::time::timeout_at(until, holding).await??;
        let _ = start_second.send(());
        // This observer belongs to the exact Store+SECOND wait and fires ONLY
        // on actual kernel Read WouldBlock, not send order or elapsed timing.
        let remaining = until
            .saturating_duration_since(tokio::time::Instant::now())
            .min(Duration::from_secs(1));
        let actual_wait =
            tokio::task::spawn_blocking(move || waited.recv_timeout(remaining)).await?;
        let was_blocked = actual_wait.is_ok();
        let no_attempt = counts(f, SECOND)? == (0, 0, 0);
        // Always release and join before testing these observations. No held
        // writer, missing answer or blocking job escapes a failing assertion.
        let (release, writer) = owner.take().ok_or("original writer missing")?;
        if was_blocked && no_attempt {
            let _ = release.send(true);
        }
        drop(release);
        let writer = writer.await?;
        writer?;
        if !was_blocked || !no_attempt {
            return Err("exact signed Read wait/no-effect rendezvous absent".into());
        }
        audio_progress(&audio, until).await?; // BOTH old Audio routes after writer, before answer.
        Ok(identity)
    }
    .await;
    if let Some((release, writer)) = owner.take() {
        drop(release);
        let _ = writer.await;
    }
    // Release original preparation even when admission/probe/withdrawal failed;
    // the two original clients will join before the test reports that failure.
    let _ = release_first.send(());
    result
}

async fn pump(
    source: &CanonicalClient,
    sh: u16,
    mut first: connect_client::canonical::NativeDatagrams,
    mut second: connect_client::canonical::NativeDatagrams,
    first_handle: u16,
    second_handle: u16,
    progress: Arc<[AtomicUsize; 2]>,
    until: tokio::time::Instant,
    stop: CancellationToken,
) -> TestResult<()> {
    use rvoip_uctp::substrate::{
        observe_native_media_datagram, pack_rtp_datagram, ObservedNativeMediaDatagram, RtpDatagram,
        RtpMediaPayload,
    };
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut seq = 42000u32;
    loop {
        tokio::select! {biased;
            _=stop.cancelled()=>return Ok(()),
            _=tokio::time::sleep_until(until)=>return Err("original media/control cutoff".into()),
            _=tick.tick()=>{
                let packet=RtpDatagram{flags:0,stream_local_id:sh,seq,rtp:RtpMediaPayload{
                    payload:(0..160).map(|n|if n%16<8{0x80}else{0x00}).collect::<Vec<u8>>().into(),
                    payload_type:0,sequence_number:seq as u16,timestamp:seq.wrapping_mul(160),ssrc:0x47360001}};
                source.send_native_media_datagram(&pack_rtp_datagram(&packet)?)?; seq+=1;
            },
            bytes=first.receive()=>{
                let bytes=bytes?;
                if let ObservedNativeMediaDatagram::Rtp(p)=observe_native_media_datagram(&bytes)? {
                    let handle=p.stream_local_id;let rtp=p.rtp.into_media_payload();
                    if rtp.payload_type==0 {
                        if handle!=first_handle {return Err("first original Audio handle changed".into());}
                        if rtp.payload.iter().any(|v|*v!=0xff&&*v!=0x7f) {progress[0].fetch_add(1,Ordering::AcqRel);}
                    }
                }
            },
            bytes=second.receive()=>{
                let bytes=bytes?;
                if let ObservedNativeMediaDatagram::Rtp(p)=observe_native_media_datagram(&bytes)? {
                    let handle=p.stream_local_id;let rtp=p.rtp.into_media_payload();
                    if rtp.payload_type==0 {
                        if handle!=second_handle {return Err("second original Audio handle changed".into());}
                        if rtp.payload.iter().any(|v|*v!=0xff&&*v!=0x7f) {progress[1].fetch_add(1,Ordering::AcqRel);}
                    }
                }
            },
        }
    }
}

async fn exercise(algorithm: &str, withdraw: bool) {
    let f = Fixture::with_native_stream_scope(algorithm, 12).await;
    ready_endpoint_with_capabilities(&f, vec!["audio".into(), "video".into(), "data".into()]).await;
    let mut target = CanonicalClient::open(f.client.clone()).await.unwrap();
    let cid = session_with_media(
        &f,
        &mut target,
        "voice+video",
        json!({"audio":["g.711-mu"],"video":["vp8"],"data":["reliable-ordered"]}),
    )
    .await;
    target
        .session_command(Request {
            operation_id: "overlap-data-upgrade".into(),
            conversation_id: cid.clone(),
            session_id: "native-session".into(),
            by_participant: None,
            expected_revision: Some(1),
            action: Action::Upgrade {
                media: vec!["data".into()],
            },
        })
        .await
        .unwrap();
    let source_config = party_with_capabilities(
        &f,
        &f.peer.identity_id,
        320,
        true,
        vec!["audio".into(), "video".into(), "data".into()],
    );
    let source = CanonicalClient::open(source_config.clone()).await.unwrap();
    let mut clients = vec![source, target];
    let mut credentials = vec![source_config.credential, f.client.credential.clone()];
    let mut second_config = None;
    for i in 2..6 {
        let identity = if i == 2 || i % 2 == 1 {
            &f.client.identity_id
        } else {
            &f.peer.identity_id
        };
        let config = party_with_capabilities(
            &f,
            identity,
            320 + i,
            true,
            vec!["audio".into(), "video".into(), "data".into()],
        );
        clients.push(CanonicalClient::open(config.clone()).await.unwrap());
        if i == 2 {
            second_config = Some(config.clone());
        }
        credentials.push(config.credential);
    }
    let mut answers = vec![];
    for i in 0..6 {
        let mut streams = vec![
            json!({"id":format!("overlap-audio-{i}"),"kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]}),
            json!({"id":format!("overlap-data-{i}"),"kind":"data","direction":"sendrecv","codec_preferences":["binary"]}),
        ];
        if i != 1 && i != 2 {
            streams.push(json!({"id":format!("overlap-video-{i}"),"kind":"video","direction":"sendonly","codec_preferences":["vp8"]}));
        }
        answers.push(
            negotiate_media(
                &mut clients[i],
                &cid,
                &format!("overlap-offer-{i}"),
                &format!("overlap-connection-{i}"),
                json!(streams),
                capabilities(),
            )
            .await,
        );
        wait_native_operation(&f, &credentials[i], &cid, &format!("overlap-offer-{i}")).await;
    }
    for i in [1usize, 2] {
        stream_control(
            &mut clients[i],
            &cid,
            &format!("overlap-connection-{i}"),
            &format!("overlap-audio-select-{i}"),
            1,
            "stream.subscribe",
            json!({"subscriptions":[{"strm_id":"overlap-audio-0"}]}),
        )
        .await;
    }
    wait_slots(&f, 6).await;
    let sh = local_handle(&answers[0], "overlap-audio-0");
    let th = local_handle(&answers[1], "overlap-audio-1");
    let second_handle = local_handle(&answers[2], "overlap-audio-2");
    assert_audio_reaches_other_participant(&mut clients, sh, &[th], 41800).await;
    let first_io = clients[1].claim_native_datagrams().unwrap();
    let second_io = clients[2].claim_native_datagrams().unwrap();
    let second_config = second_config.unwrap();
    let second_credential = f
        .server
        .store
        .authenticate(&second_config.credential)
        .unwrap()
        .credential_id;
    let probe_request = connect_store::PeerAuthorityRequest {
        credentials: vec![second_credential],
        device: connect_store::PeerDeviceClaim {
            id: second_config.device_id,
            kind: second_config.device_kind,
            platform: second_config.device_platform,
        },
        key_id: second_config.signing_key_id,
        algorithm: second_config.signing_algorithm,
        command: connect_store::PeerCommand::NativeStream(
            connect_protocol::native_streams::Intent {
                operation_id: SECOND.into(),
                conversation_id: cid.clone(),
                session_id: "native-session".into(),
                connection_id: "overlap-connection-2".into(),
                expected_revision: Some(2),
                by_participant: None,
                command: connect_protocol::native_streams::Command::Subscribe,
                payload: selection(),
            },
        ),
    };
    let until = tokio::time::Instant::now() + Duration::from_secs(8); // One original bound, never renewed.
    let (prepared, entered) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let (start, start_second) = oneshot::channel();
    let stop = CancellationToken::new();
    let progress = Arc::new([AtomicUsize::new(0), AtomicUsize::new(0)]);
    let ((first, second, coord), audio) = {
        let (source, targets) = clients.split_at_mut(1);
        let (first, rest) = targets.split_at_mut(1);
        let controls = async {
            let result = tokio::join!(
                original(
                    &mut first[0],
                    scope(&cid, 1, FIRST),
                    until,
                    Some((prepared, released)),
                    None
                ),
                original(
                    &mut rest[0],
                    scope(&cid, 2, SECOND),
                    until,
                    None,
                    Some(start_second)
                ),
                coordinate(
                    &f,
                    &cid,
                    until,
                    withdraw,
                    entered,
                    release,
                    start,
                    progress.clone(),
                    probe_request
                )
            );
            stop.cancel();
            result
        };
        tokio::join!(
            controls,
            pump(
                &source[0],
                sh,
                first_io,
                second_io,
                th,
                second_handle,
                progress.clone(),
                until,
                stop.clone()
            )
        )
    };
    let withdrawal_evidence = if withdraw {
        Some(withdrawn_history(&f, &cid, until).await)
    } else {
        None
    };
    let withdrawal_policy = if withdraw {
        Some(video_withdrawn_audio_current(&f))
    } else {
        None
    };
    let final_counts = (counts(&f, FIRST), counts(&f, SECOND));
    let final_revisions = live_revisions(&f);
    // Natural original owner cleanup precedes every overlap/result assertion.
    let mut shutdown = vec![];
    for client in clients {
        shutdown.push(client.shutdown().await);
    }
    let server = f.server.shutdown().await;
    assert!(
        coord.is_ok(),
        "{algorithm} real-overlap coordinator: {coord:?}; first={first:?}; second={second:?}"
    );
    assert!(audio.is_ok(), "{algorithm} old Audio pump: {audio:?}");
    assert!(
        shutdown.iter().all(|result| result.is_ok()),
        "all original signed peers joined"
    );
    assert!(server.is_ok(), "original server cleanup: {server:?}");
    let first_identity = coord.expect("original pre-Writer record/effect identity");
    let first = first.expect("first original signed exchange");
    let second = second.expect("second original signed exchange");
    if withdraw {
        assert_eq!(withdrawal_policy.unwrap().unwrap(),true,"actual current Video must be withdrawn while Audio remains accepted on BOTH original targets");
        assert_withdrawal(
            &first,
            &second,
            &first_identity,
            withdrawal_evidence
                .unwrap()
                .expect("same-original terminal withdrawal journal"),
        );
        assert_ne!(first.state.as_deref(), Some("completed"));
        assert_ne!(second.state.as_deref(), Some("completed"));
        assert_eq!(first.offers, 1);
        assert_eq!(
            second.offers, 0,
            "withdrawn current policy cannot prepare another offer"
        );
        assert_eq!(final_counts.0.unwrap(), (1, 1, 1));
        let second_counts = final_counts.1.unwrap();
        assert!(second_counts.0 <= 1 && second_counts.1 <= 1 && second_counts.2 <= 1);
        assert_eq!(
            final_revisions.unwrap(),
            vec![(true, 1, 2), (true, 1, 2)],
            "refused fresh current authority preserves original Audio/resources"
        );
    } else {
        assert_eq!(
            first.state.as_deref(),
            Some("completed"),
            "original first refusal: {:?}",
            first.problem
        );
        assert_eq!(
            second.state.as_deref(),
            Some("completed"),
            "original second refusal: {:?}",
            second.problem
        );
        assert_eq!(
            first.record.as_ref(),
            Some(&first_identity.0),
            "completed reply changed the pre-Writer original record"
        );
        assert_eq!((first.offers, second.offers), (1, 1));
        assert!(first.problem.is_none() && second.problem.is_none());
        assert_eq!(final_counts.0.unwrap(), (1, 1, 1));
        assert_eq!(final_counts.1.unwrap(), (1, 1, 1));
        assert_eq!(final_revisions.unwrap(), vec![(true, 1, 3), (true, 1, 3)]);
        for (out, connection) in [
            (&first, "overlap-connection-1"),
            (&second, "overlap-connection-2"),
        ] {
            let result = out
                .result
                .as_ref()
                .expect("actual completed selection result");
            assert_eq!(result["connection_id"], connection);
            assert_eq!(result["stream_revision"], 3);
            assert_eq!(result["resources_changed"], true);
        }
    }
}

#[path = "overlap/initial_offers.rs"]
mod initial_offers;
