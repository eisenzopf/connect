//! Bounded real lifecycle/owner census, not a media quality or RSS qualification.
use super::*;
use futures_util::FutureExt;
use std::{panic::AssertUnwindSafe, sync::Weak};
use tracing::{instrument::WithSubscriber, Instrument};
#[path = "../../../../../connect-service/tests/support/heap_log.rs"]
mod heap_log;
fn config(control: CanonicalClientConfig) -> sdk::ClientConfig {
    sdk::ClientConfig {
        control,
        tenant_id: "tnt_default".parse().unwrap(),
        native_video_feedback: true,
        application: None,
    }
}
fn one<T>(first: T) -> NonEmpty<T> {
    NonEmpty {
        first,
        rest: vec![],
    }
}
fn offer(id: &str) -> NonEmpty<sdk::OfferedStream> {
    one(sdk::OfferedStream {
        id: id.parse().unwrap(),
        kind: sdk::Medium::Audio,
        direction: sdk::Direction::Sendrecv,
        codecs: one("opus".into()),
    })
}
async fn pair(
    room: &sdk::Conversation,
    other: &Client,
    recipient: sdk::ParticipantId,
    prefix: &str,
) -> (sdk::Session, sdk::Connection, sdk::Connection) {
    let caps = json!({"audio":["opus"]});
    let session = room
        .invite(
            format!("{prefix}-session").parse().unwrap(),
            vec![recipient],
            sdk::SessionMedium::Voice,
            "bounded heap owner census".into(),
            caps.clone(),
            mutation(&format!("{prefix}-invite")),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    let recipient = other
        .conversation_by_id(room.id().clone())
        .session_by_id(session.id().clone());
    recipient
        .accept(
            caps,
            sdk::TopologyRevision::try_from(0).unwrap(),
            mutation(&format!("{prefix}-accept")),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap();
    let a_id = format!("{prefix}-a");
    let b_id = format!("{prefix}-b");
    let a_stream = format!("{prefix}-a-voice");
    let b_stream = format!("{prefix}-b-voice");
    let a = session
        .offer_connection(
            a_id.parse().unwrap(),
            offer(&a_stream),
            mutation(&format!("{prefix}-a-offer")),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    let b = recipient
        .offer_connection(
            b_id.parse().unwrap(),
            offer(&b_stream),
            mutation(&format!("{prefix}-b-offer")),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap()
        .value;
    a.subscribe(
        one(sdk::StreamSelector::Stream(b_stream.parse().unwrap())),
        sdk::StreamRevision::try_from(1).unwrap(),
        mutation(&format!("{prefix}-a-select")),
    )
    .await
    .unwrap()
    .wait(options())
    .await
    .unwrap();
    b.subscribe(
        one(sdk::StreamSelector::Stream(a_stream.parse().unwrap())),
        sdk::StreamRevision::try_from(1).unwrap(),
        mutation(&format!("{prefix}-b-select")),
    )
    .await
    .unwrap()
    .wait(options())
    .await
    .unwrap();
    // Actual libopus encode and real normal native packet/media paths. This is
    // initialization evidence only, not the matrix's tone/quality acceptance.
    let a_audio = a.stream_by_id(format!("{prefix}-a-voice").parse().unwrap());
    let b_audio = b.stream_by_id(format!("{prefix}-b-voice").parse().unwrap());
    tokio::join!(super::sound(&a_audio, true), super::sound(&b_audio, true));
    let a_receive = a.stream_by_id(a_stream.parse().unwrap());
    let b_receive = b.stream_by_id(b_stream.parse().unwrap());
    let (a_observed, b_observed) = tokio::join!(
        a_receive.receive_packet(options()),
        b_receive.receive_packet(options())
    );
    assert!(matches!(a_observed.unwrap(), sdk::MediaPacket::Rtp(_)));
    assert!(matches!(b_observed.unwrap(), sdk::MediaPacket::Rtp(_)));
    (session, a, b)
}
async fn end(session: &sdk::Session, a: &sdk::Connection, b: &sdk::Connection, prefix: &str) {
    a.end(
        0,
        "normal bounded cleanup".into(),
        None,
        mutation(&format!("{prefix}-a-end")),
    )
    .await
    .unwrap()
    .wait(options())
    .await
    .unwrap();
    b.end(
        0,
        "normal bounded cleanup".into(),
        None,
        mutation(&format!("{prefix}-b-end")),
    )
    .await
    .unwrap()
    .wait(options())
    .await
    .unwrap();
    let revision = session.refresh(options()).await.unwrap().topology_revision;
    session
        .end(
            0,
            "normal bounded cleanup".into(),
            revision,
            mutation(&format!("{prefix}-session-end")),
        )
        .await
        .unwrap()
        .wait(options())
        .await
        .unwrap();
}
fn weak(
    f: &Fixture,
    session: &sdk::Session,
    connection: &sdk::Connection,
) -> Vec<(&'static str, Weak<dyn std::any::Any + Send + Sync>)> {
    let database = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let runtime: String = database
        .query_row(
            "SELECT runtime_connection_id FROM connection_offers WHERE connection_id=?1",
            [connection.id().as_str()],
            |row| row.get(0),
        )
        .unwrap();
    f.server
        .service
        .canonical_staged_media_weak_owners_for_test(session.id().as_str(), &runtime)
        .unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_real_session_ends_on_one_host_restore_live_owner_census_and_keep_unrelated_session_live(
) {
    let f = Fixture::with_native_end_scope("EdDSA", 12).await;
    let log = f.temp.path().join("heap-boundaries.jsonl");
    let capture = heap_log::Capture::new(&log);
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_env_filter("connect_service::runtime_retention=debug")
        .with_writer(capture.clone())
        .finish();
    let mut clients = Vec::new();
    let observed=AssertUnwindSafe(async {
        super::super::super::canonical_native_connections::ready_endpoint_with_capabilities(&f,vec!["audio".into()]).await;
        let mut scope=Scope::collaboration(vec!["*".into()]);
        scope.operations.extend(["stream.subscribe".into(),"connection.end".into()]);
        let recipient=super::super::super::canonical_native_connections::party_with_scope(&f,&f.peer.identity_id,991,vec!["audio".into()],scope);
        clients.push(Client::connect_configured(config(f.client.clone())).await.unwrap());
        clients.push(Client::connect_configured(config(recipient)).await.unwrap());
        let room=clients[0].create_conversation(vec![f.peer.identity_id.parse().unwrap()],ConversationPolicy::Persistent,
            mutation("heap-lifecycle-room")).await.unwrap().wait(options()).await.unwrap().value;
        let recipient=room.refresh(options()).await.unwrap().participants.into_iter()
            .find(|peer|peer.identity_id.as_str()==f.peer.identity_id).unwrap().id;
        let (live,live_a,live_b)=pair(&room,&clients[1],recipient.clone(),"heap-unrelated").await;
        let original_live=weak(&f,&live,&live_a);
        assert!(!original_live.is_empty());
        let credential=f.server.store.authenticate(&f.admin).unwrap().credential_id;
        let pid=std::process::id();
        f.server.service.execute(credential.clone(),Operation::Health{})
            .instrument(tracing::info_span!(target:"connect_service::runtime_retention","connect-memory-baseline"))
            .await.unwrap();
        for cycle in 1..=3 {
            let prefix=format!("heap-cycle-{cycle}");
            let (session,a,b)=pair(&room,&clients[1],recipient.clone(),&prefix).await;
            let mut original=weak(&f,&session,&a);original.extend(weak(&f,&session,&b));
            assert!(original.iter().all(|(_,owner)|owner.strong_count()>0));
            end(&session,&a,&b,&prefix).await;
            assert!(original.iter().all(|(_,owner)|owner.strong_count()==0),"ended Session retained actual media owners before host shutdown");
            assert!(original_live.iter().all(|(_,owner)|owner.strong_count()>0),"unrelated live Session owner was retired");
            assert_ne!(live.refresh(options()).await.unwrap().state,"ended");
            assert_eq!(std::process::id(),pid);
            drop((session,a,b,original));
            f.server.service.execute(credential.clone(),Operation::Health{})
                .instrument(tracing::info_span!(target:"connect_service::runtime_retention","connect-memory-idle-final",cycle))
                .await.unwrap();
        }
        end(&live,&live_a,&live_b,"heap-unrelated").await;
        assert!(original_live.iter().all(|(_,owner)|owner.strong_count()==0));
    }.with_subscriber(subscriber)).catch_unwind().await;
    // Original clients and host always close naturally before failure/assertion
    // propagation. Do not parse/retain heap JSON during the measured cycles.
    let mut closed = Vec::new();
    for client in clients {
        closed.push(client.close(options()).await);
    }
    let shutdown = f.server.shutdown().await;
    assert!(capture.within_bound());
    drop(capture);
    for close in closed {
        close.unwrap();
    }
    shutdown.unwrap();
    observed.unwrap();
    let rows = heap_log::rows(&log);
    let samples = rows
        .iter()
        .filter(|row| !row["heap_boundary"].is_null())
        .collect::<Vec<_>>();
    assert_eq!(samples.len(), 4);
    let baseline = &samples[0]["heap_boundary"];
    assert_eq!(baseline["boundary"], "connect-memory-baseline");
    for sample in &samples[1..] {
        let sample = &sample["heap_boundary"];
        assert_eq!(sample["boundary"], "connect-memory-idle-final");
        assert_eq!(sample["pid"], baseline["pid"]);
        assert!(sample["raw_c_heap"].is_null());
        for kind in [
            "core.managed_session_runtime",
            "core.media_scene",
            "core.audio_mixer",
            "core.stream_controller",
            "core.media_port",
            "core.audio_codec_pair",
            "native.routed_stream",
            "native.packetizer",
        ] {
            let count = |row: &Value| {
                row["objects"]["kinds"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|entry| entry["kind"] == kind)
                    .and_then(|entry| entry["live"].as_u64())
                    .expect("required live owner kind was not observed")
            };
            assert!(
                count(baseline) > 0,
                "unrelated original owner baseline absent"
            );
            assert_eq!(
                count(sample),
                count(baseline),
                "ended Session changed original live owner count: {kind}"
            );
        }
    }
    // Emit only scalar accounting after all measured work and original owners
    // have joined. The fixture's temporary log is removed on normal return.
    // The regression runner retains this bounded stdout under --nocapture.
    let accounting = samples
        .iter()
        .enumerate()
        .map(|(index, row)| {
            json!({"boundary_index": index, "heap": row["heap_boundary"],
                "store": row["registries"]["packet_observer_claims"]["memory_footprint"]})
        })
        .collect::<Vec<_>>();
    println!(
        "{}",
        json!({"format": "connect/heap-lifecycle-regression-observation/1",
            "optimized": !cfg!(debug_assertions), "cycles": 3,
            "original_owners_joined": true, "session_owner_release_assertions_passed": true,
            "accounting": accounting})
    );
}
