//! Actual fresh private SIP call beside the same Participant's public call.
//! Quarantine and full switch profiles keep their behavior assertions separate.
use super::*;
use rvoip_uctp::v2::payloads::OperationState;
#[path = "private_candidate/switched.rs"]
mod switched;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_private_sip_candidate_owns_fresh_muted_call_and_original_cleanup_eddsa() {
    exercise("EdDSA", false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_private_sip_candidate_owns_fresh_muted_call_and_original_cleanup_es256() {
    exercise("ES256", false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_private_sip_candidate_switch_preserves_original_call_and_public_sibling_eddsa()
{
    exercise("EdDSA", true).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_private_sip_candidate_switch_preserves_original_call_and_public_sibling_es256()
{
    exercise("ES256", true).await;
}

async fn observe_marked(clients: &mut [Peer], duration: Duration) -> Vec<usize> {
    let until = tokio::time::Instant::now() + duration;
    let mut counts = vec![0; clients.len()];
    loop {
        let reads = clients
            .iter_mut()
            .enumerate()
            .map(|(i, p)| {
                Box::pin(async move { (i, p.client.receive_native_rtp_datagram().await) })
            })
            .collect::<Vec<_>>();
        let Ok(((i, bytes), _, _)) = tokio::time::timeout_at(until, select_all(reads)).await else {
            break;
        };
        let packet = rvoip_uctp::substrate::unpack_rtp_datagram(&bytes.unwrap()).unwrap();
        assert_eq!(packet.stream_local_id, clients[i].handle);
        if let Some(path) = std::env::var_os("CONNECT_PRIVATE_SIP_AUDIO_DIAGNOSTIC") {
            use std::io::Write;
            let path = std::path::PathBuf::from(path);
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join(".local/gate-6");
            assert!(path.is_absolute() && path.starts_with(root));
            let row = json!({"receiver":i,"payload":&packet.rtp.payload,"observed_unix_micros":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros()});
            writeln!(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .unwrap(),
                "{row}"
            )
            .unwrap();
        }
        if packet
            .rtp
            .payload
            .iter()
            .any(|b| !matches!(*b, 0xff | 0x7f))
        {
            counts[i] += 1;
        }
    }
    counts
}

async fn marked(peer: &PeerSip, clients: &mut [Peer]) -> Vec<usize> {
    let (_, counts) = tokio::join!(
        peer.rtp(0),
        observe_marked(clients, Duration::from_millis(1000))
    );
    counts
}

async fn exercise(algorithm: &str, complete: bool) {
    let ConfiguredInitial { f, peer, .. } = if complete {
        configured_initial_human(algorithm).await
    } else {
        configured_initial_quarantined_ai(algorithm).await
    };
    let mut primary = CanonicalClient::open(f.client.clone()).await.unwrap();
    let mut input =
        connect_client::conversations::CreateRequest::persistent(vec![f.peer.identity_id.clone()]);
    input.tenant_id = Some("tnt_default".into());
    let room = primary
        .create_conversation("private-sip-room", input)
        .await
        .unwrap();
    let own = room
        .participants
        .iter()
        .find(|p| p.identity_id == f.client.identity_id)
        .unwrap()
        .participant_id
        .clone();
    let phone = room
        .participants
        .iter()
        .find(|p| p.identity_id == f.peer.identity_id)
        .unwrap()
        .participant_id
        .clone();
    primary
        .session_command(Request {
            operation_id: "public-phone-invite".into(),
            conversation_id: room.id.clone(),
            session_id: "native-session".into(),
            by_participant: Some(own.clone()),
            expected_revision: None,
            action: Action::Invite {
                participants: vec![phone.clone()],
                medium: "voice".into(),
                intent: "call".into(),
                capabilities: json!({"audio":["g.711-mu"]}),
            },
        })
        .await
        .unwrap();
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let until = tokio::time::Instant::now() + Duration::from_secs(8);
    let public = loop {
        let row:Option<(String,String,String)>=db.query_row("SELECT o.connection_id,n.runtime_connection_id,n.sip_call_id FROM initial_telephone_operations n JOIN operations o ON o.record_id=n.operation_record_id JOIN initial_telephone_answers a ON a.operation_record_id=n.operation_record_id WHERE n.parent_kind='invitation' AND a.connected_at IS NOT NULL AND a.retired_at IS NULL",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().unwrap();
        if let Some(row) = row {
            break row;
        }
        assert!(
            tokio::time::Instant::now() < until,
            "public baseline call did not become ready"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(peer.unique(), 1);
    let public_media = *peer.media_target.lock().unwrap();
    let public_dialog = peer.dialog.lock().unwrap().clone();
    ready_endpoint(&f).await;
    let answer = negotiate_named(
        &mut primary,
        &room.id,
        "private-primary-offer",
        "private-primary",
        "private-primary-audio",
    )
    .await;
    wait_native_operation(&f, &f.client.credential, &room.id, "private-primary-offer").await;
    // The first subscriber uses an actual live SIP source, not only its ready metadata.
    peer.rtp(0).await;
    stream_control(
        &mut primary,
        &room.id,
        "private-primary",
        "private-primary-public-phone",
        1,
        "stream.subscribe",
        json!({"subscriptions":[{"from_participant":phone.clone(),"kinds":["audio"]}]}),
    )
    .await;
    let mut held = enroll_kind_with_scope(
        &f,
        &mut primary,
        &room.id,
        "private-held",
        201,
        true,
        false,
        IdentityKind::Human,
        if complete {
            Some(super::super::received::privacy::signal_scope())
        } else {
            None
        },
    )
    .await;
    let held_revision = stream_revision(&f, &held.id);
    stream_control(
        &mut held.client,
        &room.id,
        &held.id,
        "held-public-phone",
        held_revision,
        "stream.subscribe",
        json!({"subscriptions":[{"from_participant":phone.clone(),"kinds":["audio"]}]}),
    )
    .await;
    let mut clients = vec![
        Peer {
            client: primary,
            config: Some(f.client.clone()),
            identity: f.client.identity_id.clone(),
            participant: own.clone(),
            id: "private-primary".into(),
            handle: handle(&answer),
            clock: tokio::time::Instant::now(),
            sequence: 1,
            live: true,
        },
        held,
    ];
    let baseline = marked(&peer, &mut clients).await;
    assert!(
        baseline.iter().all(|count| *count > 10),
        "public phone baseline missing: {baseline:?}"
    );
    // No private allocation or RTP input exists yet. Count already admitted
    // public media separately before observing the fresh private input below.
    let baseline_backlog = observe_marked(&mut clients, Duration::from_millis(250)).await;
    eprintln!("PRIVATE_SIP_PRE_ALLOCATION_PUBLIC_BACKLOG {baseline_backlog:?}");
    let key = f
        .server
        .store
        .issue(
            &f.client.identity_id,
            Scope::operator(),
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut cfg = canonical_http_config(&f);
    cfg.credential = key.token;
    let mut controller = CanonicalHttpClient::open(cfg).await.unwrap();
    let command = TransitionCommand {
        payload_extensions: Default::default(),
        operation_id: "private-sip-consult".into(),
        conversation_id: room.id.clone(),
        session_id: "native-session".into(),
        connection_id: Some("private-primary".into()),
        expected_revision: revision(&f),
        intent: TransitionIntent::Consult {
            retained_connid: clients[1].id.clone(),
            private_participants: vec![own, phone],
            hold_treatment: HoldTreatment::Silence,
        },
    };
    let accepted = controller.transition(command.clone()).await.unwrap();
    assert_eq!(accepted.state, OperationState::Accepted);
    let record = accepted.record_id.as_ref().unwrap().as_str().to_owned();
    let until = tokio::time::Instant::now() + Duration::from_secs(8);
    let (child, connection, core, call, details) = loop {
        let row:Option<(String,String,String,String,String)>=db.query_row("SELECT n.operation_record_id,o.connection_id,n.runtime_connection_id,n.sip_call_id,a.connected_json FROM initial_telephone_operations n JOIN operations o ON o.record_id=n.operation_record_id JOIN initial_telephone_answers a ON a.operation_record_id=n.operation_record_id WHERE n.parent_kind='consultation' AND n.parent_record_id=?1 AND a.connected_at IS NOT NULL AND a.retired_at IS NULL",[&record],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().unwrap();
        if let Some(row) = row {
            break row;
        }
        assert!(
            tokio::time::Instant::now() < until,
            "private call did not attest muted original Core admission"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_ne!(connection, public.0);
    assert_ne!(core, public.1);
    assert_ne!(call, public.2);
    assert_eq!(peer.unique(), 2);
    assert_eq!(peer.dialog.lock().unwrap().as_ref().unwrap().call_id, call);
    let original_private_media = *peer.media_target.lock().unwrap();
    eprintln!(
        "PRIVATE_SIP_ORIGINAL_MEDIA_MAP {}",
        json!({"public_media":public_media.map(|a|a.to_string()),"private_media":original_private_media.map(|a|a.to_string()),"different_media_ports":public_media!=original_private_media})
    );
    let details: Value = serde_json::from_str(&details).unwrap();
    assert_eq!(details["private_audio"], true);
    assert_eq!(details["stream_revision"], 2);
    assert_eq!(details["private_consultation_record_id"], record);
    let reserved: String = db
        .query_row(
            "SELECT connection_id FROM sip_consultation_reservations WHERE operation_record_id=?1",
            [&record],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(reserved, connection);
    assert_eq!(db.query_row("SELECT count(*) FROM native_offer_operations n JOIN operations o ON o.record_id=n.operation_record_id WHERE o.connection_id=?1",[&connection],|r|r.get::<_,u64>(0)).unwrap(),0,"SIP candidate borrowed a native signing/admission source");
    if complete {
        let observed = json!({"algorithm":algorithm,"conversation_id":room.id,"record":record,"child":child,"connection":connection,"core":core,"call":call,"public":public,"details":details,"baseline":baseline,"pre_allocation_public_backlog":baseline_backlog});
        switched::exercise(
            &f,
            &peer,
            &mut clients,
            &mut controller,
            &command,
            &key.id,
            observed,
            public_media,
            public_dialog,
        )
        .await;
        drop(db);
        for p in &mut clients {
            p.client.close();
        }
        f.server.shutdown().await.unwrap();
        return;
    }
    let kinds: (String, String) = db.query_row(
        "SELECT i.kind,p.kind FROM connections c JOIN identities i ON i.id=c.identity_id JOIN participants p ON p.participant_id=c.participant_id WHERE c.id=?1",
        [&connection], |r| Ok((r.get(0)?, r.get(1)?)),
    ).unwrap();
    assert_eq!(kinds, ("ai".into(), "ai".into()));
    assert_unclaimed_quarantine(&db, &record);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM operation_effects WHERE operation_record_id=?1",
            [&record],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0,
        "quarantine is not a private audience switch"
    );
    let replay = controller.transition(command.clone()).await.unwrap();
    assert_eq!(replay.record_id, accepted.record_id);
    let before_busy_byes = peer
        .methods
        .lock()
        .unwrap()
        .iter()
        .filter(|m| **m == Method::Bye)
        .count();
    let store = f.server.store.clone();
    let cid = room.id.clone();
    let writer = tokio::task::spawn_blocking(move || {
        store
            .acquire_authority(
                &[connect_store::AuthorityScope::conversation(
                    &cid,
                    connect_store::AuthorityMode::Write,
                )
                .unwrap()],
                &connect_store::AuthorityWait::new(Duration::from_secs(2)),
            )
            .unwrap()
    })
    .await
    .unwrap();
    // Two complete probe periods: packet/read guards must refuse, while the
    // already closed original quarantine keeps its exact physical call alive.
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(
        peer.methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| **m == Method::Bye)
            .count(),
        before_busy_byes,
        "temporary metadata contention retired the original quarantined call"
    );
    drop(writer);
    eprintln!(
        "PRIVATE_SIP_CURRENT_MEDIA_MAP {}",
        json!({"current_media":peer.media_target.lock().unwrap().map(|a|a.to_string()),"original_private_media":original_private_media.map(|a|a.to_string()),"current_dialog_is_original_private":peer.dialog.lock().unwrap().as_ref().is_some_and(|d|d.call_id==call)})
    );
    let no_input = std::env::var_os("CONNECT_PRIVATE_SIP_QUARANTINE_NO_INPUT_DIAGNOSTIC").is_some();
    eprintln!(
        "PRIVATE_SIP_QUARANTINE_DIAGNOSTIC {}",
        json!({"private_rtp_sent":!no_input,"observed_unix_micros":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros()})
    );
    let counts = if no_input {
        observe_marked(&mut clients, Duration::from_millis(1000)).await
    } else if let Ok(period) = std::env::var("CONNECT_PRIVATE_SIP_PRIVATE_PERIOD_DIAGNOSTIC") {
        let period: usize = period.parse().unwrap();
        eprintln!("PRIVATE_SIP_INPUT_PERIOD_DIAGNOSTIC {period}");
        let (_, counts) = tokio::join!(
            peer.rtp_pattern(0, period),
            observe_marked(&mut clients, Duration::from_millis(1000))
        );
        counts
    } else {
        marked(&peer, &mut clients).await
    };
    assert_unclaimed_quarantine(&db, &record);
    assert_eq!(
        counts,
        vec![0, 0],
        "fresh private input escaped quarantine through the public sibling or held receiver"
    );
    assert_eq!(peer.unique(), 2, "replay allocated another SIP call");
    let cancel=controller.request(payload("operation.cancel",json!({"operation_id":command.operation_id,"reason":"verified quarantined admission"})),CommandScope {conversation_id:Some(room.id.clone()),session_id:Some("native-session".into()),connection_id:Some("private-primary".into()),operation_id:Some("cancel-private-sip".into()),..Default::default()}).await.unwrap();
    cancel.ack().unwrap();
    let until = tokio::time::Instant::now() + Duration::from_secs(6);
    loop {
        let absent:bool=db.query_row("SELECT c.state='retired' AND a.retired_at IS NOT NULL FROM connections c JOIN operations o ON o.connection_id=c.id JOIN initial_telephone_answers a ON a.operation_record_id=o.record_id WHERE o.record_id=?1",[&child],|r|r.get(0)).unwrap();
        if absent {
            break;
        }
        assert!(
            tokio::time::Instant::now() < until,
            "original private call cleanup was not captured"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(peer.methods.lock().unwrap().contains(&Method::Bye));
    let original_live:bool=db.query_row("SELECT a.retired_at IS NULL AND c.state IN ('ready','active') FROM initial_telephone_operations n JOIN initial_telephone_answers a ON a.operation_record_id=n.operation_record_id JOIN operations o ON o.record_id=n.operation_record_id JOIN connections c ON c.id=o.connection_id WHERE o.connection_id=?1",[&public.0],|r|r.get(0)).unwrap();
    assert!(original_live);
    *peer.media_target.lock().unwrap() = public_media;
    let recovered = marked(&peer, &mut clients).await;
    assert!(
        recovered[0] > 10,
        "original public sibling did not survive private cleanup: {recovered:?}"
    );
    if let (false, Ok(path)) = (
        no_input,
        std::env::var("CONNECT_PRIVATE_SIP_CANDIDATE_CAPTURE"),
    ) {
        std::fs::write(format!("{path}.{algorithm}.json"),serde_json::to_vec_pretty(&json!({"algorithm":algorithm,"scope":"actual fresh SIP/Core quarantined admission and original cleanup; no private switch/DTMF/return qualification","parent":record,"child":child,"candidate_connection":connection,"core_connection":core,"sip_call_id":call,"original_public":public,"private_ready":details,"marked_public_baseline":baseline,"pre_allocation_public_backlog":baseline_backlog,"marked_quarantine":counts,"marked_public_after_cleanup":recovered,"distinct_sip_calls":peer.unique(),"original_public_live":original_live,"metadata_busy_preserved":true,"authority_writer_hold_ms":250,"identity_kind":kinds.0,"participant_kind":kinds.1,"unclaimed_before_and_after_input":true})).unwrap()).unwrap();
    }
    drop(db);
    for p in &mut clients {
        p.client.close();
    }
    f.server.shutdown().await.unwrap();
}

// State is part of the quarantine precondition, including after the packet
// observation. An audience that legitimately applied is a different profile.
fn assert_unclaimed_quarantine(db: &rusqlite::Connection, record: &str) {
    let state: (String, bool, u64, u64) = db.query_row(
        "SELECT state,cancel_requested,(SELECT count(*) FROM native_consultation_attempts WHERE operation_record_id=?1),(SELECT count(*) FROM operation_effects WHERE operation_record_id=?1) FROM operations WHERE record_id=?1",
        [record], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).unwrap();
    assert_eq!(
        state,
        ("accepted".into(), false, 0, 0),
        "quarantine requires an accepted, unclaimed, effect-free parent"
    );
}
