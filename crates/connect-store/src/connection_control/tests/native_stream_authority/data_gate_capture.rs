//! Store custody with controlled Audio+Data readiness, not a Core gate/codec or
//! decoded-media claim. The original same-connection PCM epoch must survive a
//! captured Data-only gate's first durable recording and its exact replay.
use super::*;
use crate::{NativeMediaRead, NativeMediaSnapshot};
use connect_protocol::{
    native_gates::GateStream, native_streams::CodecBinding, transitions::OperationState,
};
use std::{
    panic::{catch_unwind, resume_unwind, AssertUnwindSafe},
    sync::Arc,
    time::Instant,
};

type Database = Vec<(String, Vec<Vec<rusqlite::types::Value>>)>;

// Exact typed values stay in memory; no row contents or identifiers are logged.
fn database(store: &Store) -> Database {
    let c = store.connection().unwrap();
    let mut names = c.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap();
    let names = names
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    names
        .into_iter()
        .map(|name| {
            let quoted = name.replace('"', "\"\"");
            let mut statement = c.prepare(&format!("SELECT * FROM \"{quoted}\"")).unwrap();
            let columns = statement.column_count();
            let mut rows = statement
                .query_map([], |r| {
                    (0..columns)
                        .map(|i| r.get(i))
                        .collect::<rusqlite::Result<Vec<rusqlite::types::Value>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            rows.sort_by_key(|r| format!("{r:?}"));
            (name, rows)
        })
        .collect()
}

// AV metadata is independently compared even when the Data stream revision
// advances. The immutable offer, actual connection incarnation and audio
// resource record must retain their exact original values.
fn audio_metadata(store: &Store) -> (Vec<rusqlite::types::Value>, String) {
    let c = store.connection().unwrap();
    let mut statement = c.prepare("SELECT c.id,c.session_id,c.participant_id,c.membership_epoch,c.identity_id,c.endpoint_id,c.generation,c.state,c.revision,c.observed_runtime_id,o.offer_json FROM connections c JOIN connection_offers o ON o.connection_id=c.id WHERE c.id='connection-native'").unwrap();
    let n = statement.column_count();
    let connection = statement
        .query_row([], |r| {
            (0..n)
                .map(|i| r.get(i))
                .collect::<rusqlite::Result<Vec<rusqlite::types::Value>>>()
        })
        .unwrap();
    let audio = c.query_row("SELECT json_group_array(json(j.value)) FROM native_stream_snapshots s,json_each(s.projection_json,'$.stream_metadata') j WHERE s.connection_id='connection-native' AND json_extract(j.value,'$.medium')='audio'", [], |r|r.get::<_,String>(0)).unwrap();
    assert_ne!(
        audio, "[]",
        "real stored audio description must exist before the Data gate"
    );
    (connection, audio)
}

fn binding(data: bool) -> CodecBinding {
    CodecBinding {
        stream_id: if data { "application" } else { "audio" }.into(),
        medium: if data { "data" } else { "audio" }.into(),
        direction: if data { "sendonly" } else { "sendrecv" }.into(),
        codec: if data {
            json!({"name":"json","params":{}})
        } else {
            json!({"name":"opus","params":{"sample_rate":48000,"channels":1}})
        },
        stream_local_id: if data { 8 } else { 9 },
        stream_generation: 1,
        connection_generation: 1,
    }
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    DataGate,
    AudioGate,
    Codecs,
    Subscribe,
}

fn prepared(
    kind: Kind,
) -> (
    Fixture,
    Arc<crate::NativeMediaOrigin>,
    crate::NativeStreamCommitted,
    crate::PeerAuthorityGuard,
) {
    let (f, mut request, _, origin) =
        parent_permissions_origin_filtered_media(true, true, false, true, true, &[], &[], true);
    // Like the parent's ready fixture, these are controlled Store metadata,
    // established under the real broad Writer before any retained media IO.
    // Production replace/save builds the exact Audio+Data projection, not a
    // hand-written permit or an invented media/current-authority grant.
    {
        let writer = f.controlled_conversation_writer(0);
        let c = f.store.connection_with_authority(&writer).unwrap();
        crate::native_stream_projection::complete(
            &c,
            origin.connection_id(),
            1,
            1,
            1,
            Some(&[binding(false), binding(true)]),
        )
        .unwrap();
    }
    let PeerCommand::NativeStream(wire) = &mut request.command else {
        panic!()
    };
    wire.operation_id = "captured-media-boundary".into();
    wire.command = if matches!(kind, Kind::Subscribe) {
        Command::Subscribe
    } else {
        Command::Update
    };
    wire.payload = match kind {
        Kind::DataGate => json!({"action":"mute","streams":["application"]}),
        Kind::AudioGate => json!({"action":"mute","streams":["audio"]}),
        Kind::Codecs => {
            json!({"action":"codec-renegotiate","streams":["audio"],"codec_preferences":["opus"]})
        }
        Kind::Subscribe => {
            json!({"subscriptions":[{"from_participant":f.pids[1],"kinds":["audio"]}]})
        }
    };
    let proof = admit(&f, &request).unwrap();
    f.store
        .accept_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap();
    let (lease, started) = f
        .store
        .begin_native_stream_with_peer(&f.keys[0], &proof)
        .unwrap()
        .unwrap();
    assert_eq!(started.state, OperationState::Preparing);
    assert_eq!(started.effects.len(), 1);
    let receipt = match kind {
        Kind::DataGate | Kind::AudioGate => lease
            .gates_committed(
                2,
                vec![GateStream {
                    binding: binding(matches!(kind, Kind::DataGate)),
                    state: "active".into(),
                    source_muted: true,
                    held_direction: None,
                }],
            )
            .unwrap(),
        Kind::Codecs => lease.codecs_committed(2, vec![binding(false)]).unwrap(),
        Kind::Subscribe => lease.committed(2, true, None).unwrap(),
    };
    // Keep this actual original command proof through first completion, just
    // as Service retains the already-executed command until reply admission.
    assert!(proof.current());
    (f, origin, receipt, proof)
}

fn current(snapshot: &NativeMediaSnapshot) -> NativeMediaRead {
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        match snapshot.admit() {
            Ok(io) => return io,
            Err(e) if e.code == "authority-busy" && Instant::now() < until => {
                std::thread::yield_now()
            }
            Err(e) => panic!("controlled PCM setup refused: {}", e.code),
        }
    }
}

// All live IO and original command readers drain and the watcher joins before
// any intended causal assertion. A work panic is restored after cleanup.
fn exercise(kind: Kind) {
    let (f, origin, receipt, proof) = prepared(kind);
    let store = Arc::new(f.store);
    let weak = Arc::downgrade(&store);
    let source = origin.clone();
    let snapshot = store
        .register_native_media_snapshot_for(&origin, None, move || {
            weak.upgrade()
                .ok_or_else(crate::Error::forbidden)?
                .try_native_media_read(&source, None)
        })
        .unwrap();
    let mut watcher = store.start_media_authorization_watcher().unwrap();
    let work = catch_unwind(AssertUnwindSafe(|| {
        let io = current(&snapshot);
        assert!(io.current());
        assert_eq!(io.target().connection.connection_id, origin.connection_id());
        assert!(io.target().offer["streams_offered"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["kind"] == "audio"));
        let stamp = io.snapshot_stamp().unwrap();
        let epoch = io.packet_epoch().unwrap();
        assert!(epoch.current());
        let before = database(&store);
        let original_audio = audio_metadata(&store);
        let until = Instant::now() + Duration::from_secs(2);
        let mut early_busy = 0usize;
        let first = loop {
            assert!(
                Instant::now() < until,
                "original outcome metadata cutoff expired"
            );
            let result = store.complete_native_stream(&receipt);
            if result
                .as_ref()
                .err()
                .is_some_and(|e| e.code == "authority-busy")
                && epoch.current()
                && io.current()
                && proof.current()
                && database(&store) == before
            {
                // A primary-SQL or local metadata clash before any writer is
                // not the old-condition oracle. Never restore a cut epoch.
                let original = snapshot.admit_original(stamp);
                if original.as_ref().is_ok_and(|v| v.current()) && Instant::now() < until {
                    early_busy += 1;
                    drop(original);
                    std::thread::sleep(
                        Duration::from_millis(1)
                            .min(until.saturating_duration_since(Instant::now())),
                    );
                    continue;
                }
            }
            break result;
        };
        let original = snapshot.admit_original(stamp);
        let original_epoch_current = epoch.current();
        let survived =
            original_epoch_current && io.current() && original.as_ref().is_ok_and(|v| v.current());
        let audio_unchanged =
            !matches!(kind, Kind::DataGate) || audio_metadata(&store) == original_audio;
        let no_change_if_refused = first.is_ok() || database(&store) == before;
        let within = Instant::now() < until;
        let replay = first.as_ref().ok().map(|done| {
            let again = store.complete_native_stream(&receipt);
            let same = again.as_ref().is_ok_and(|v| v == done);
            (
                same,
                epoch.current() && io.current() && original.as_ref().is_ok_and(|v| v.current()),
            )
        });
        let code = first.as_ref().err().map(|e| e.code.clone());
        // All original epoch samples precede drop and watcher shutdown.
        drop(original);
        drop(io);
        (
            first,
            survived,
            no_change_if_refused,
            within,
            replay,
            code,
            early_busy,
            original_epoch_current,
            audio_unchanged,
        )
    }));
    drop(proof);
    let cleanup = watcher.shutdown();
    let (
        first,
        survived,
        no_change_if_refused,
        within,
        replay,
        code,
        early_busy,
        original_epoch_current,
        audio_unchanged,
    ) = match work {
        Ok(v) => v,
        Err(panic) => {
            let _ = cleanup;
            resume_unwind(panic)
        }
    };
    cleanup.unwrap();
    // Same captured receipt after every original reader drained proves the
    // fixture's completion is usable, even on the deliberately failing old path.
    let recovered = store.complete_native_stream(&receipt).unwrap();
    assert_eq!(recovered.state, OperationState::Completed);
    assert_eq!(recovered.effects.len(), 1);
    assert_eq!(store.complete_native_stream(&receipt).unwrap(), recovered);
    assert!(
        within && no_change_if_refused && audio_unchanged,
        "pre-oracle cutoff/durable qualifiers failed"
    );
    eprintln!("captured-gate-boundary kind={kind:?} first_code={code:?} original_epoch_current={survived} durable_refusal_unchanged={no_change_if_refused} early_metadata_busy={early_busy} same_receipt_recovered=true all_original_readers_drained=true watcher_joined=true");
    if matches!(kind, Kind::DataGate) {
        if !original_epoch_current {
            assert_eq!(
                code.as_deref(),
                Some("authority-busy"),
                "old-condition qualification requires actual writer refusal"
            );
        }
        assert!(original_epoch_current, "captured Data-only gate revoked original same-connection PCM epoch before durable recording");
        assert!(survived);
        assert_eq!(first.unwrap(), recovered);
        assert_eq!(replay, Some((true, true)));
        let result = recovered.result.unwrap();
        assert_eq!(
            result["gate_streams"],
            serde_json::to_value(vec![GateStream {
                binding: binding(true),
                state: "active".into(),
                source_muted: true,
                held_direction: None,
            }])
            .unwrap()
        );
    } else {
        assert_eq!(code.as_deref(), Some("authority-busy"));
        assert!(
            !original_epoch_current,
            "an actual AV mutation must fence its original media epoch"
        );
    }
}

#[test]
fn captured_data_only_gate_records_under_original_command_and_same_connection_pcm_readers() {
    exercise(Kind::DataGate);
}

#[test]
fn captured_audio_gate_codec_and_subscription_still_fence_original_media_epochs() {
    for kind in [Kind::AudioGate, Kind::Codecs, Kind::Subscribe] {
        exercise(kind);
    }
}

mod permit;
mod public_controls;
