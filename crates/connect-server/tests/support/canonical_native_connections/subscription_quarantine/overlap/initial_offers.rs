//! Distinct initial-offer 503 site: six real signed original offers progress
//! together, all stop at their own original resource callback, then activate.
use super::*;
use futures_util::future::join_all;
use std::collections::BTreeSet;
use tokio::sync::mpsc as async_mpsc;

#[tokio::test]
async fn signed_six_concurrent_eddsa_initial_offers_keep_original_preparation_and_activation() {
    run("EdDSA").await;
}
#[tokio::test]
async fn signed_six_concurrent_es256_initial_offers_keep_original_preparation_and_activation() {
    run("ES256").await;
}

enum Progress {
    Prepared(usize),
    Refused(usize),
}
fn operation(index: usize) -> String {
    format!("initial-overlap-offer-{index}")
}
fn connection(index: usize) -> String {
    format!("initial-overlap-connection-{index}")
}
fn streams(index: usize) -> Value {
    json!([{"id":format!("initial-overlap-audio-{index}"),"kind":"audio","direction":"sendrecv","codec_preferences":["g.711-mu"]},
        {"id":format!("initial-overlap-data-{index}"),"kind":"data","direction":"sendrecv","codec_preferences":["binary"]},
        {"id":format!("initial-overlap-video-{index}"),"kind":"video","direction":"sendonly","codec_preferences":["vp8"]}])
}
fn exact_scope(frame: &Envelope, cid: &str, index: usize) -> bool {
    frame.scope(rvoip_uctp::v2::Scope::Conversation) == Some(cid)
        && frame.scope(rvoip_uctp::v2::Scope::Session) == Some("native-session")
        && frame.scope(rvoip_uctp::v2::Scope::Connection) == Some(connection(index).as_str())
        && frame.scope(rvoip_uctp::v2::Scope::Operation) == Some(operation(index).as_str())
}
async fn offer(
    client: &mut CanonicalClient,
    cid: &str,
    index: usize,
    until: tokio::time::Instant,
    progress: async_mpsc::UnboundedSender<Progress>,
    release: CancellationToken,
) -> TestResult<Outcome> {
    let result:TestResult<Outcome>=async {
        let op=operation(index);let conn=connection(index);
        let scope=CommandScope {conversation_id:Some(cid.into()),session_id:Some("native-session".into()),connection_id:Some(conn.clone()),operation_id:Some(op.clone()),expected_revision:Some(0)};
        // Exactly ONE mutation per peer; status below uses only the same ID.
        let request=tokio::time::timeout_at(until,client.send_value("connection.offer",
            json!({"substrate":"quic","capabilities":capabilities(),"streams_offered":streams(index)}),scope.clone())).await??;
        let initial=tokio::time::timeout_at(until,client.receive_control()).await??;
        if initial.kind()=="error" {return Ok(Outcome {state:None,problem:Some(initial.original()["payload"].clone()),record:None,offers:0,result:None});}
        if initial.kind()!="connection.answer" || initial.scope(rvoip_uctp::v2::Scope::Reply)!=Some(request.as_str()) || !exact_scope(&initial,cid,index) {
            return Err("initial original answer correlation/scope changed".into());
        }
        let expected=streams(index).as_array().ok_or("original stream intent")?.iter().map(|s|s["id"].as_str().map(str::to_owned).ok_or("original stream ID")).collect::<Result<BTreeSet<_>,_>>()?;
        let actual=initial.original()["payload"]["streams_answered"].as_array().ok_or("initial selected stream set")?.iter().map(|s|s["id"].as_str().map(str::to_owned).ok_or("selected stream ID")).collect::<Result<BTreeSet<_>,_>>()?;
        if expected!=actual || actual.len()!=3 {return Err("initial offer selected another stream set".into());}
        let callback=tokio::time::timeout_at(until,client.receive_control()).await??;
        if callback.kind()=="error" {return Ok(Outcome {state:None,problem:Some(callback.original()["payload"].clone()),record:None,offers:0,result:None});}
        if callback.kind()!="connection.offer" || !exact_scope(&callback,cid,index) {return Err("missing exact original resource callback".into());}
        let _=progress.send(Progress::Prepared(index));
        // Real adapter parameter preparation stays owned by this exact command.
        // The coordinator releases every waiter even if a sibling refuses.
        tokio::select!{biased;_=release.cancelled()=>{},_=tokio::time::sleep_until(until)=>return Err("original initial-offer cutoff".into())}
        let p=&callback.original()["payload"];
        let prepared=p["substrate_setup"]["streams"].as_array().ok_or("initial original prepared streams")?;
        let selected=prepared.iter().map(|s|json!({"id":s["strm_id"],"kind":s["kind"],
            "direction":match s["direction"].as_str(){Some("sendonly")=>"recvonly",Some("recvonly")=>"sendonly",_=>"sendrecv"},"codec":s["codec"]})).collect::<Vec<_>>();
        let answer=serde_json::from_value(json!({"by_participant":p["by_participant"],"substrate":"quic","capabilities":capabilities(),"streams_answered":selected,"substrate_setup":p["substrate_setup"]}))?;
        tokio::time::timeout_at(until,client.answer_connection_offer(&callback,answer,&scope)).await??;
        let mut record=None;let mut revision=0;
        loop {
            if tokio::time::Instant::now()>=until {return Err("same original offer observation cutoff".into());}
            let batch=tokio::time::timeout_at(until,client.request_value("operation.get",json!({"operation_id":op}),CommandScope {
                conversation_id:Some(cid.into()),session_id:Some("native-session".into()),connection_id:Some(conn.clone()),..Default::default()})).await??;
            for frame in batch.frames.iter().filter(|frame|frame.scope(rvoip_uctp::v2::Scope::Reply)==Some(batch.request_id.as_str())) {
                if !resource_scope(frame,&scope,true) {return Err("same-original offer read response changed resource scope".into());}
            }
            // Same actual closed refusal is retained for origin diagnostics.
            if let Some(error)=batch.frames.iter().find(|f|f.kind()=="error") {
                return Ok(Outcome {state:None,problem:Some(error.original()["payload"].clone()),record,offers:1,result:None});
            }
            batch.ack()?;
            let updates=batch.frames.iter().filter(|f|f.kind()=="operation.updated"&&f.scope(rvoip_uctp::v2::Scope::Reply)==Some(batch.request_id.as_str())).collect::<Vec<_>>();
            if updates.len()!=1 {return Err("missing unique same-original offer observation".into());}
            let payload=&updates[0].original()["payload"];
            let next=payload["revision"].as_u64().ok_or("original offer revision")?;
            let id=payload["record_id"].as_str().filter(|id|!id.is_empty()).ok_or("original offer record")?;
            if payload["operation_id"]!=op || payload["command_type"]!="connection.offer" || payload["expected_revision"]!=0 || next<revision || record.as_deref().is_some_and(|old|old!=id) {
                return Err("original offer record/intent/precondition changed".into());
            }
            record=Some(id.to_owned());revision=next;
            let state=payload["state"].as_str().ok_or("original offer state")?;
            match state {
                "completed"|"failed"|"cancelled"|"interrupted"|"unknown"=>return Ok(Outcome {state:Some(state.into()),problem:payload.get("problem").cloned(),record,offers:1,result:payload.get("result").cloned()}),
                "accepted"|"preparing"|"ready"|"committing"=>{},
                _=>return Err("unsupported original offer state".into()),
            }
            tokio::time::sleep_until((tokio::time::Instant::now()+Duration::from_millis(20)).min(until)).await;
        }
    }.await;
    if !matches!(&result,Ok(out) if out.state.as_deref()==Some("completed")) {
        let _ = progress.send(Progress::Refused(index));
    }
    result
}

async fn all_prepared(
    f: &Fixture,
    cid: &str,
    until: tokio::time::Instant,
    mut progress: async_mpsc::UnboundedReceiver<Progress>,
    release: CancellationToken,
) -> TestResult<()> {
    let result: TestResult<()> = async {
        let mut ready = BTreeSet::new();
        while ready.len() < 6 {
            match tokio::time::timeout_at(until, progress.recv()).await? {
                Some(Progress::Prepared(index)) if index < 6 => {
                    if !ready.insert(index) {
                        return Err("duplicate original preparation".into());
                    }
                }
                Some(Progress::Refused(index)) => {
                    return Err(format!(
                        "original initial offer {index} refused before all six prepared"
                    )
                    .into())
                }
                _ => return Err("original initial offer owner ended before preparation".into()),
            }
        }
        for i in 0..6 {
            if counts(f, &operation(i))? != (1, 1, 1) {
                return Err("initial original offer/effect not unique".into());
            }
        }
        let store = f.server.store.clone();
        let cid = cid.to_owned();
        // A Writer must progress while ALL six actual parameter callbacks wait.
        // Join this exact blocking owner before releasing any original answer.
        tokio::task::spawn_blocking(move || -> connect_protocol::Result<()> {
            let held = store.acquire_authority(
                &[AuthorityScope::conversation(&cid, AuthorityMode::Write)?],
                &AuthorityWait::new(Duration::from_millis(500)),
            )?;
            drop(held);
            Ok(())
        })
        .await??;
        Ok(())
    }
    .await;
    release.cancel(); // Also releases all original waiters on refusal/error.
    result
}

async fn run(algorithm: &str) {
    let f = Fixture::with_native_stream_scope(algorithm, 12).await;
    ready_endpoint_with_capabilities(&f, vec!["audio".into(), "video".into(), "data".into()]).await;
    let mut first = CanonicalClient::open(f.client.clone()).await.unwrap();
    let cid = session_with_media(
        &f,
        &mut first,
        "voice+video",
        json!({"audio":["g.711-mu"],"video":["vp8"],"data":["reliable-ordered"]}),
    )
    .await;
    first
        .session_command(Request {
            operation_id: "initial-overlap-data-upgrade".into(),
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
    let mut clients = vec![first];
    for index in 1..6 {
        let identity = if index % 2 == 0 {
            &f.client.identity_id
        } else {
            &f.peer.identity_id
        };
        let config = party_with_capabilities(
            &f,
            identity,
            350 + index,
            true,
            vec!["audio".into(), "video".into(), "data".into()],
        );
        clients.push(CanonicalClient::open(config).await.unwrap());
    }
    let until = tokio::time::Instant::now() + Duration::from_secs(8);
    let (events, observed) = async_mpsc::unbounded_channel();
    let release = CancellationToken::new();
    let offers =
        join_all(clients.iter_mut().enumerate().map(|(index, client)| {
            offer(client, &cid, index, until, events.clone(), release.clone())
        }));
    drop(events);
    let (outcomes, coordinator) =
        tokio::join!(offers, all_prepared(&f, &cid, until, observed, release));
    let recorded = (0..6)
        .map(|i| counts(&f, &operation(i)))
        .collect::<Vec<_>>();
    let mut cleanup = vec![];
    for client in clients {
        cleanup.push(client.shutdown().await);
    }
    let server = f.server.shutdown().await;
    // No original callback, client, blocking task or server is detached by a
    // failed progress assertion. No original wire command is ever resubmitted.
    assert!(
        coordinator.is_ok(),
        "{algorithm} initial-offer overlap: {coordinator:?}; actual outcomes={outcomes:?}; cleanup={cleanup:?}; server={server:?}"
    );
    for (index, (outcome, recorded)) in outcomes.into_iter().zip(recorded).enumerate() {
        let outcome = outcome.expect("same original signed offer exchange");
        assert_eq!(
            outcome.state.as_deref(),
            Some("completed"),
            "{algorithm} initial offer {index} closed refusal: {:?}",
            outcome.problem
        );
        assert_eq!(outcome.offers, 1);
        assert_eq!(recorded.unwrap(), (1, 1, 1));
        assert!(outcome.problem.is_none());
        assert!(outcome.record.is_some());
        // connection.offer completes with readiness details. Its exact
        // connection identity is carried by the original wire resource scopes,
        // checked on the answer, callback and every same-ID Get response above.
        // stream.subscribe has a different result schema with connection_id.
        let result = outcome.result.as_ref().expect("completed offer readiness");
        let ready = result["streams"]
            .as_array()
            .expect("completed offer streams");
        let actual = ready
            .iter()
            .map(|id| id.as_str().expect("completed offer stream ID"))
            .collect::<BTreeSet<_>>();
        let offered = streams(index);
        let expected = offered
            .as_array()
            .unwrap()
            .iter()
            .map(|stream| stream["id"].as_str().unwrap())
            .collect::<BTreeSet<_>>();
        assert_eq!(ready.len(), 3, "exact original completed stream count");
        assert_eq!(actual, expected, "exact original completed stream IDs");
        assert!(result["stream_revision"].as_u64().is_some_and(|r| r > 0));
        assert!(result["participant_revision"]
            .as_u64()
            .is_some_and(|r| r > 0));
    }
    assert!(
        cleanup.iter().all(|r| r.is_ok()),
        "original six client cleanup"
    );
    assert!(
        server.is_ok(),
        "original initial-offer server cleanup: {server:?}"
    );
}
