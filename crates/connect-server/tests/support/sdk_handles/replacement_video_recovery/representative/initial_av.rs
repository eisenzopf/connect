//! Initial ordinary AV selection under the exact prepared-subscription SQL boundaries.
//! Six signed ordinary connections/four identities/two Sessions; no handoff or matrix qualification.
use super::*;
use connect_store::{AuthorityMode, AuthorityScope, AuthorityWait};
use futures_util::FutureExt;
use std::panic::AssertUnwindSafe;
use tokio::sync::oneshot;
const OPERATION: &str = "representative-initial-av-select-3-with-contention";
const TARGET: usize = 3;
const OLD: [usize; 2] = [0, 2];
type TestResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn av_selection(index: usize) -> NonEmpty<StreamSelector> {
    let source = SOURCE_FOR_RECEIVER[index];
    NonEmpty {
        first: StreamSelector::Stream(format!("representative-audio-{source}").parse().unwrap()),
        rest: vec![StreamSelector::Stream(
            format!("representative-video-{source}").parse().unwrap(),
        )],
    }
}
async fn reached(
    probe: &std::sync::atomic::AtomicUsize,
    expected: usize,
    until: Instant,
) -> TestResult<()> {
    loop {
        let n = probe.load(Ordering::Acquire);
        if n == expected {
            return Ok(());
        }
        if n > expected || Instant::now() >= until {
            return Err(format!("original AV boundary expected={expected} actual={n}").into());
        }
        tokio::time::sleep_until(until.min(Instant::now() + Duration::from_millis(1))).await;
    }
}
async fn decoded(
    progress: &[SharedProgress],
    indices: &[usize],
    before: Option<&[Progress]>,
    until: Instant,
) {
    timeout_at(until, async {
        loop {
            let good = indices.iter().all(|&i| {
                let p = progress[i].lock().unwrap();
                fresh(&p)
                    && before.is_none_or(|b| {
                        p.frames > b[i].frames && p.audio_samples > b[i].audio_samples
                    })
            });
            if good {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("same original current Opus PCM and genuine fragmented VP8 decode must advance");
}
fn offers(f: &Fixture) -> TestResult<Vec<(String, String, String, u64, u64, String)>> {
    let sql = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut q=sql.prepare("SELECT o.operation_id,o.record_id,e.id,e.attempt,e.submission_generation,e.state FROM operations o JOIN operation_effects e ON e.operation_record_id=o.record_id WHERE o.command_type='connection.offer' AND o.operation_id LIKE 'representative-offer-%' ORDER BY o.operation_id")?;
    let rows: Vec<(String, String, String, u64, u64, String)> = q
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    assert_eq!(rows.len(), 6);
    for r in &rows {
        assert_eq!((r.3, r.4, r.5.as_str()), (1, 1, "confirmed"));
    }
    Ok(rows)
}
fn original_attempt(f: &Fixture) -> TestResult<(String, String, String, i64, u64, u64)> {
    let sql = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let row:(String,String,String,i64,u64,u64)=sql.query_row("SELECT o.record_id,n.attempt_id,n.effect_id,o.expires_at,e.attempt,e.submission_generation FROM operations o JOIN native_stream_operations n ON n.operation_record_id=o.record_id JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=o.record_id WHERE o.operation_id=?1 AND o.command_type='stream.subscribe' AND n.effect_started=1 AND e.provider='native-stream' AND e.state='submitting' AND e.observation_json IS NULL AND o.state='preparing' AND o.result_json IS NULL",[OPERATION],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
    assert_eq!((row.4, row.5), (1, 1));
    Ok(row)
}

/// The two real blockers always release and join. The future awaiting the SAME
/// SDK mutation is never dropped to cancel or recreate its prepared handle.
async fn faults(
    f: &Fixture,
    cid: &str,
    admission: Arc<std::sync::atomic::AtomicUsize>,
    scene: Arc<std::sync::atomic::AtomicUsize>,
    progress: &[SharedProgress],
    masters: &[programme::Source],
    until: Instant,
) -> TestResult<()> {
    let mut release = None;
    let mut worker = None;
    let result = AssertUnwindSafe(async {
        reached(&admission, 1, until).await?;
        let original = original_attempt(f)?;
        let before_offers = offers(f)?;
        let store = f.server.store.clone();
        let cid = cid.to_owned();
        let (entered_tx, entered_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        release = Some(release_tx);
        worker = Some(tokio::task::spawn_blocking(move || {
            let guard = store.acquire_authority(
                &[AuthorityScope::conversation(&cid, AuthorityMode::Write)?],
                &AuthorityWait::new(until.saturating_duration_since(Instant::now())),
            )?;
            let _ = entered_tx.send(());
            let _ = release_rx.blocking_recv();
            drop(guard);
            connect_protocol::Result::Ok(())
        }));
        timeout_at(until, entered_rx).await??;
        admission.store(2, Ordering::Release);
        reached(&admission, 3, until).await?;
        release
            .take()
            .unwrap()
            .send(())
            .map_err(|_| "original scoped AV Writer ended early")?;
        worker.take().unwrap().await??;
        // A real Writer may invalidate old packet epochs. Prove the SAME old
        // owners resume current real media, rather than reviving an old grant.
        let after_writer = clone_progress(progress);
        for &i in &OLD {
            masters[SOURCE_FOR_RECEIVER[i]]
                .force_key
                .store(true, Ordering::Release);
        }
        decoded(progress, &OLD, Some(&after_writer), until).await;
        reached(&scene, 1, until).await?;
        let store = f.server.store.clone();
        let (entered_tx, entered_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        release = Some(release_tx);
        worker = Some(tokio::task::spawn_blocking(move || {
            store.with_primary_sql_held_for_test(|| {
                let _ = entered_tx.send(Instant::now());
                let _ = release_rx.blocking_recv();
            })
        }));
        let sql_entered=timeout_at(until, entered_rx).await??;
        let before_sql = clone_progress(progress);
        // These are temporary metadata-only subscriptions to the SAME already
        // running codec masters. No source, encoder or packet owner is recreated.
        let mut samples=OLD.iter().map(|&i|(i,masters[SOURCE_FOR_RECEIVER[i]].subscribe())).collect::<Vec<_>>();
        let mut watermarks=Vec::new();
        for (index,reader) in &mut samples {
            loop {
                let sample=timeout_at(until,reader.recv()).await??;
                if sample.video && sample.captured>=sql_entered {
                    // The actual original publisher uses this exact formula;
                    // no handoff/RTP wrap occurs in this bounded initial case.
                    watermarks.push((*index,sample.index.checked_mul(3000).unwrap()));
                    break;
                }
            }
        }
        drop(samples);
        scene.store(2, Ordering::Release);
        reached(&scene, 3, until).await?;
        // The actual failed factory has returned and exact fresh Weak custody
        // is gone. Independent already-owned media must still decode while SQL
        // remains excluded; it supplies no replacement/current command grant.
        timeout_at(until,async {
            loop {
                let advanced=watermarks.iter().all(|&(i,timestamp)| {
                    let p=progress[i].lock().unwrap();
                    fresh(&p) && p.frames>before_sql[i].frames && p.audio_samples>before_sql[i].audio_samples
                        && p.timestamp.is_some_and(|observed|observed>=timestamp)
                });
                if advanced {break;}tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("original video captured AFTER actual SQL exclusion must decode while that SQL owner remains held");
        eprintln!("initial AV held-SQL decoded video capture watermarks={watermarks:?}; audio proof is PCM advance only, not source-quantum attribution");
        assert_eq!(original_attempt(f)?, original);
        assert_eq!(offers(f)?, before_offers);
        release
            .take()
            .unwrap()
            .send(())
            .map_err(|_| "original AV SQL owner ended early")?;
        worker.take().unwrap().await??;
        reached(&scene, 4, until).await?;
        assert_eq!(admission.load(Ordering::Acquire), 4);
        assert_eq!(offers(f)?, before_offers);
        assert!(Instant::now() < until);
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    })
    .catch_unwind()
    .await;
    drop(release.take());
    let joined: TestResult<()> = if let Some(worker) = worker.take() {
        match worker.await {
            Ok(r) => r.map_err(Into::into),
            Err(e) => Err(e.into()),
        }
    } else {
        Ok(())
    };
    match result {
        Err(p) => {
            if let Err(e) = joined {
                eprintln!("secondary original AV blocker join failure: {e}");
            }
            std::panic::resume_unwind(p)
        }
        Ok(Err(e)) => {
            if let Err(j) = joined {
                eprintln!("secondary original AV blocker join failure: {j}");
            }
            Err(e)
        }
        Ok(Ok(())) => {
            joined?;
            Ok(())
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn signed_initial_six_source_av_subscription_releases_both_busy_boundaries_without_replaying_original(
) {
    assert_eq!(std::env::consts::ARCH, "aarch64");
    if std::env::var_os("RUST_LOG").is_some() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap())
            .with_test_writer()
            .try_init();
    }
    let entered = Instant::now();
    let f = Fixture::with_native_end_scope("EdDSA", 16).await;
    let mut clients: Vec<Client> = Vec::new();
    let mut sessions: Vec<sdk::Session> = Vec::new();
    let mut connections: Vec<sdk::Connection> = Vec::new();
    let mut works: Vec<Work> = Vec::new();
    let mut masters: Vec<programme::Source> = Vec::new();
    let stop = CancellationToken::new();
    let progress = (0..6)
        .map(|_| Arc::new(Mutex::new(Progress::default())))
        .collect::<Vec<_>>();
    let work=AssertUnwindSafe(async {
    let (binary, raw) = codec_inputs(f.temp.path()).await;
    let admin = f.server.store.authenticate(&f.admin).unwrap().credential_id;
    let mut identities = vec![f.client.identity_id.clone(), f.peer.identity_id.clone()];
    for n in 2..4 {
        let ProvisioningOutcome::Identity { identity_id, .. } = f
            .server
            .store
            .execute_provisioning(
                &admin,
                ProvisioningCommand::Identity {
                    operation_id: format!("representative-identity-{n}"),
                    alias: format!("representative-{n}"),
                    kind: IdentityKind::Human,
                    display_name: None,
                },
            )
            .unwrap()
        else {
            panic!("original identity provision")
        };
        identities.push(identity_id);
    }
    let mut participant_scope = Scope::collaboration(vec!["*".into()]);
    participant_scope.operations.extend([
        "stream.subscribe".into(),
        "stream.unsubscribe".into(),
        "connection.end".into(),
        "connection.update".into(),
    ]);
    let configs = identities
        .iter()
        .enumerate()
        .map(|(n, id)| {
            crate::canonical_native_connections::party_with_scope(
                &f,
                id,
                700 + n,
                vec!["audio".into(), "video".into(), "data".into()],
                participant_scope.clone(),
            )
        })
        .collect::<Vec<_>>();
    for config in &configs {
        clients.push(
            Client::connect_configured(configured(config.clone()))
                .await
                .unwrap(),
        );
    }
    let room = clients[0]
        .create_conversation(
            identities[1..].iter().map(|s| s.parse().unwrap()).collect(),
            ConversationPolicy::Persistent,
            mutation("representative-room"),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    let view = room.refresh(options()).await.unwrap();
    assert_eq!(view.participants.len(), 4);
    let pids = identities
        .iter()
        .map(|identity| {
            view.participants
                .iter()
                .find(|p| p.identity_id.as_str() == identity)
                .unwrap()
                .id
                .clone()
        })
        .collect::<Vec<_>>();
    let caps = json!({"audio":["opus"],"video":["vp8"],"data":["reliable-ordered"]});
    for (sid, roles) in [vec![0usize, 1, 2, 3], vec![0, 1]].into_iter().enumerate() {
        let session = room
            .invite(
                format!("representative-session-{sid}").parse().unwrap(),
                roles[1..].iter().map(|&i| pids[i].clone()).collect(),
                sdk::SessionMedium::TextChat,
                "simultaneous real media integration".into(),
                caps.clone(),
                mutation(&format!("representative-invite-{sid}")),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap()
            .value;
        for &role in &roles[1..] {
            let peer = clients[role]
                .conversation_by_id(room.id().clone())
                .session_by_id(session.id().clone());
            let before = peer.refresh(options()).await.unwrap();
            peer.accept(
                caps.clone(),
                before.topology_revision,
                mutation(&format!("representative-accept-{sid}-{role}")),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        }
        let before = session.refresh(options()).await.unwrap();
        assert_eq!(before.media, vec![Medium::Data]);
        session
            .update(
                sdk::SessionUpdate::MediumUpgrade(NonEmpty {
                    first: Medium::Audio,
                    rest: vec![Medium::Video],
                }),
                before.topology_revision,
                mutation(&format!("representative-upgrade-{sid}")),
            )
            .await
            .unwrap()
            .wait(options())
            .await
            .unwrap();
        let after = session.refresh(options()).await.unwrap();
        assert_eq!(
            after.media,
            vec![Medium::Audio, Medium::Data, Medium::Video]
        );
        sessions.push(session);
    }
    assert_eq!(sessions[0].conversation_id(), sessions[1].conversation_id());
    for index in 0..6 {
        let session = clients[OWNER[index]]
            .conversation_by_id(room.id().clone())
            .session_by_id(sessions[SESSION[index]].id().clone());
        let streams = NonEmpty {
            first: sdk::OfferedStream {
                id: format!("representative-audio-{index}").parse().unwrap(),
                kind: Medium::Audio,
                direction: Direction::Sendrecv,
                codecs: single("opus".into()),
            },
            rest: vec![
                sdk::OfferedStream {
                    id: format!("representative-data-{index}").parse().unwrap(),
                    kind: Medium::Data,
                    direction: Direction::Sendrecv,
                    codecs: single("binary".into()),
                },
                sdk::OfferedStream {
                    id: format!("representative-video-{index}").parse().unwrap(),
                    kind: Medium::Video,
                    direction: Direction::Sendonly,
                    codecs: single("vp8".into()),
                },
                sdk::OfferedStream {
                    id: format!("representative-incoming-{index}").parse().unwrap(),
                    kind: Medium::Video,
                    direction: Direction::Recvonly,
                    codecs: single("vp8".into()),
                },
            ],
        };
        connections.push(
            session
                .offer_connection(
                    format!("representative-connection-{index}")
                        .parse()
                        .unwrap(),
                    streams,
                    mutation(&format!("representative-offer-{index}")),
                )
                .await
                .unwrap()
                .wait(options())
                .await
                .unwrap()
                .value,
        );
        original_binding_diagnostics("after-offer", &connections[index], index);
    }

        let retirements=(0..6).map(|i|Arc::new(RetirementWitness{database:f.config.database.clone(),source:connections[i].id().as_str().into(),original:Mutex::new(None)})).collect::<Vec<_>>();
        let began=Instant::now()+Duration::from_millis(100);let source_until=began+Duration::from_secs(62);
        for i in 0..6 {
            let(master,owners)=programme::source(i,&binary,&raw[i],progress[i].clone(),stop.clone(),began,source_until);
            works.extend(owners);works.extend(spawn_publisher(&connections[i],i,&master,progress[i].clone(),stop.clone(),retirements[i].clone(),began,source_until));masters.push(master);
        }
        // Pin the ONE original fault/read/command cap before baseline setup.
        // The deliberate CID Writer may withdraw authority for its bounded2s
        // admission wait; these SAME owners must survive and decode freshly
        // after release, with no new receiver or renewed read deadline.
        let until=Instant::now()+Duration::from_secs(8);
        for i in 0..TARGET {
            let revision=connections[i].refresh(programme::request(until)).await.unwrap().stream_revision.unwrap();
            connections[i].subscribe(av_selection(i),revision,programme::mutation_at(&format!("initial-av-baseline-select-{i}"),until)).await.unwrap().wait(programme::request(until)).await.unwrap();
            works.push(spawn_original_receiver_with_read_cutoff(&connections[i],i,SOURCE_FOR_RECEIVER[i],&binary,progress[i].clone(),stop.clone(),retirements[i].clone(),began,source_until,Some(until)));
            masters[SOURCE_FOR_RECEIVER[i]].force_key.store(true,Ordering::Release);
        }
        decoded(&progress,&OLD,None,until).await;
        let before=clone_progress(&progress);let original_offers=offers(&f).unwrap();
        let admission=f.server.service.arm_canonical_preparation_registry_probe_for_test(&format!("subscription-admission:{OPERATION}")).unwrap();
        let scene=f.server.service.arm_canonical_preparation_registry_probe_for_test(&format!("subscription-scene:{OPERATION}")).unwrap();
        let revision=connections[TARGET].refresh(programme::request(until)).await.unwrap().stream_revision.unwrap();
        let original=async {
            let mut options=mutation(OPERATION);options.request=programme::request(until);
            let operation=connections[TARGET].subscribe(av_selection(TARGET),revision,options).await.unwrap();
            let reference=operation.reference_json();
            let done=operation.wait(programme::request(until)).await.unwrap();
            assert_eq!(done.snapshot.state,sdk::OperationState::Completed);
            assert_eq!(done.value.connection.id(),connections[TARGET].id());
            assert_eq!(done.value.connection_generation,1);assert!(done.value.resources_changed);
            assert_eq!(done.value.stream_revision.get(),revision.get()+1);
            assert_eq!(operation.reference_json(),reference);
            done
        };
        let(control,held)=tokio::join!(AssertUnwindSafe(original).catch_unwind(),AssertUnwindSafe(faults(&f,room.id().as_str(),admission.clone(),scene.clone(),&progress,&masters,until)).catch_unwind());
        let done=match control {Ok(done)=>done,Err(p)=>std::panic::resume_unwind(p)};
        let held=match held {Ok(r)=>r,Err(p)=>std::panic::resume_unwind(p)};held.unwrap();
        assert_eq!(offers(&f).unwrap(),original_offers);
        works.push(spawn_original_receiver_with_read_cutoff(&connections[TARGET],TARGET,SOURCE_FOR_RECEIVER[TARGET],&binary,progress[TARGET].clone(),stop.clone(),retirements[TARGET].clone(),began,source_until,Some(until)));
        masters[SOURCE_FOR_RECEIVER[TARGET]].force_key.store(true,Ordering::Release);
        decoded(&progress,&[0,2,TARGET],Some(&before),until).await;
        let sql=rusqlite::Connection::open_with_flags(&f.config.database,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let row:(String,u64,String,u64,u64,u64,u64)=sql.query_row("SELECT o.record_id,COUNT(e.id),o.state,MAX(e.attempt),MAX(e.submission_generation),COUNT(e.observation_json),c.generation FROM operations o JOIN operation_effects e ON e.operation_record_id=o.record_id JOIN connections c ON c.id=o.connection_id WHERE o.operation_id=?1 AND o.command_type='stream.subscribe'",[OPERATION],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).unwrap();
        assert_eq!(done.snapshot.record_id.as_ref().unwrap().as_str(),row.0);
        assert_eq!((row.1,row.2.as_str(),row.3,row.4,row.5,row.6),(1,"completed",1,1,1,1));
        let replacements:u64=sql.query_row("SELECT COUNT(*) FROM native_replacements",[],|r|r.get(0)).unwrap();assert_eq!(replacements,0,"initial ordinary selection cannot be a replacement fixture");drop(sql);
        assert!(Instant::now()<until,"all actual initial AV fault/SDK/decode/receipt proofs must finish before the ONE original eight-second cutoff");
        eprintln!("initial AV original subscription: actual_sources=6 sessions=2 identities=4 original_mutation_submissions=1 effect_attempts=1 admission_probe={} scene_probe={} decoded_old_routes=0,2 decoded_target=3; no full62/matrix claim",admission.load(Ordering::Acquire),scene.load(Ordering::Acquire));
    }).catch_unwind().await;
    let cleanup = Instant::now();
    stop.cancel();
    let mut failures = Vec::new();
    for owner in works {
        match owner.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => failures.push(e),
            Err(e) => failures.push(e.to_string()),
        }
    }
    drop(masters);
    let mut cleanup_panics = Vec::new();
    for (i, session) in sessions.iter().enumerate() {
        let closed = AssertUnwindSafe(async {
            let view = session.refresh(options()).await?;
            if view.state != "ended" {
                session
                    .end(
                        0,
                        "initial AV fixture cleanup".into(),
                        view.topology_revision,
                        mutation(&format!("initial-av-session-end-{i}")),
                    )
                    .await?
                    .wait(options())
                    .await?;
            }
            Ok::<(), sdk::Error>(())
        })
        .catch_unwind()
        .await;
        match closed {
            Ok(Ok(())) => {}
            Ok(Err(e)) => failures.push(format!("original Session close: {e:?}")),
            Err(p) => {
                failures.push("original Session close panicked".into());
                cleanup_panics.push(p);
            }
        }
    }
    for client in clients {
        match AssertUnwindSafe(client.close(options()))
            .catch_unwind()
            .await
        {
            Ok(Ok(())) => {}
            Ok(Err(e)) => failures.push(format!("original SDK close: {e:?}")),
            Err(p) => {
                failures.push("original SDK close panicked".into());
                cleanup_panics.push(p);
            }
        }
    }
    let server_shutdown_returned_ok =
        match AssertUnwindSafe(f.server.shutdown()).catch_unwind().await {
            Ok(Ok(())) => true,
            Ok(Err(e)) => {
                failures.push(format!("original server shutdown: {e:?}"));
                false
            }
            Err(p) => {
                failures.push("original server shutdown panicked; successful join unproved".into());
                cleanup_panics.push(p);
                false
            }
        };
    if cleanup.elapsed() > Duration::from_secs(5) {
        failures.push("all original fixture owners exceeded unchanged5s cleanup".into());
    }
    eprintln!(
        "initial AV original cleanup failures={failures:?} server_shutdown_returned_ok={server_shutdown_returned_ok}"
    );
    if let Err(p) = work {
        std::panic::resume_unwind(p);
    }
    if let Some(p) = cleanup_panics.into_iter().next() {
        std::panic::resume_unwind(p);
    }
    assert!(failures.is_empty(), "{failures:?}");
    assert!(
        entered.elapsed() < Duration::from_secs(40),
        "bounded control completed late after all original joins"
    );
}
