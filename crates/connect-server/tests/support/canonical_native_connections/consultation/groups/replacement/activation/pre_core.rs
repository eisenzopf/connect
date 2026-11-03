//! Actual signed native preparation, one injected pre-Core refusal, and duplex
//! preservation. This is not a full media-quality or transport-cell rehearsal.
use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[tokio::test]
async fn production_pre_core_adoption_refusal_keeps_original_duplex_and_never_replays_movement() {
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
        ActivationAudit::RefuseBeforeCore,
    )
    .await;
}

pub(in super::super) fn original_bindings(
    f: &Fixture,
) -> Vec<(String, String, u64, Option<String>)> {
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let mut query = c.prepare("SELECT id,state,generation,observed_runtime_id FROM connections WHERE id IN ('codec-source','codec-target') ORDER BY id").unwrap();
    let values = query
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(values.len(), 2);
    values
}

pub(in super::super) async fn send_until_refusal(peer: &mut Peer, armed: &AtomicBool) {
    use rvoip_uctp::substrate::{pack_rtp_datagram, RtpDatagram, RtpMediaPayload};
    let mut sent = 0;
    for _ in 0..12 {
        if !armed.load(Ordering::Acquire) {
            break;
        }
        peer.sequence += 1;
        let packet = pack_rtp_datagram(&RtpDatagram {
            flags: 0,
            stream_local_id: peer.handle,
            seq: peer.sequence,
            rtp: RtpMediaPayload {
                payload: vec![0xff; 160].into(),
                payload_type: 0,
                sequence_number: peer.sequence as u16,
                timestamp: (peer.clock.elapsed().as_micros() * 8 / 1000) as u32,
                ssrc: 0x4736_8002,
            },
        })
        .unwrap();
        match peer.client.send_native_media_datagram(&packet) {
            Ok(()) => sent += 1,
            Err(error) => {
                assert!(
                    !armed.load(Ordering::Acquire),
                    "target closed before the intended refusal: {error}"
                );
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(sent > 0, "actual native readiness input was never sent");
}

pub(in super::super) async fn verify(
    f: &Fixture,
    credential: &str,
    command: &TransitionCommand,
    candidate: &str,
    controller: &mut CanonicalHttpClient,
    peers: &mut [Peer],
    armed: Arc<AtomicBool>,
    before: Vec<(String, String, u64, Option<String>)>,
) {
    let until = tokio::time::Instant::now() + Duration::from_secs(10);
    let stopped = loop {
        let record = status(f, credential, &command.operation_id).await;
        if record.state == OperationState::Failed
            && f.server
                .service
                .canonical_runtime_snapshot()
                .unwrap()
                .replacement_preparations
                == 0
        {
            break record;
        }
        assert!(
            tokio::time::Instant::now() < until,
            "pre-Core refusal did not settle: {record:?}"
        );
        assert_ne!(
            record.state,
            OperationState::Completed,
            "pre-Core refusal must not commit movement"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(
        !armed.load(Ordering::Acquire),
        "the exact injected failure was not reached"
    );
    assert!(stopped.result.is_none());
    assert_eq!(
        stopped.effects.len(),
        13,
        "seven original phases and six preparation compensations"
    );
    let movement = stopped
        .effects
        .iter()
        .filter(|e| e.provider == "native-replacement-commit")
        .collect::<Vec<_>>();
    assert_eq!(movement.len(), 1);
    assert_eq!(movement[0].state, EffectState::Failed);
    assert!(movement[0].compensates_effect_id.is_none());
    assert!(stopped
        .effects
        .iter()
        .all(|e| e.attempt == 1 && e.submission_generation == 1));
    assert_eq!(
        stopped
            .effects
            .iter()
            .filter(|e| e.state == EffectState::Unknown)
            .count(),
        0
    );
    assert_eq!(
        stopped
            .effects
            .iter()
            .filter(|e| e.compensates_effect_id.is_some())
            .count(),
        6
    );
    assert!(stopped
        .effects
        .iter()
        .filter(|e| e.provider != "native-replacement-commit")
        .all(|e| e.state == EffectState::Confirmed));
    super::unpublished(f, candidate, &stopped.record_id);
    let c = rusqlite::Connection::open(&f.config.database).unwrap();
    let observation:String=c.query_row("SELECT observation_json FROM native_replacement_steps WHERE operation_record_id=?1 AND phase='commit'",[&stopped.record_id],|r|r.get(0)).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&observation).unwrap(),
        json!({"submission":"not-started"})
    );
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM connections WHERE id=?1",
            [candidate],
            |r| r.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    drop(c);
    wait_slots(f, 2).await;
    assert_eq!(original_bindings(f), before);
    peers[2].live = false;
    probe(
        peers,
        0,
        &[1],
        "pre-Core-refusal-retains-original-remote-to-source",
    )
    .await;
    probe(
        peers,
        1,
        &[0],
        "pre-Core-refusal-retains-original-source-to-remote",
    )
    .await;
    controller.transition(command.clone()).await.unwrap();
    let repeated = status(f, credential, &command.operation_id).await;
    assert_eq!(repeated.record_id, stopped.record_id);
    assert_eq!(repeated.state, stopped.state);
    assert_eq!(repeated.revision, stopped.revision);
    assert_eq!(repeated.effects, stopped.effects);
    assert_eq!(repeated.result, stopped.result);
    assert_eq!(original_bindings(f), before);
    super::unpublished(f, candidate, &repeated.record_id);
    wait_slots(f, 2).await;
    eprintln!("pre-Core native refusal: exact injection consumed, no movement/Ready, one failed commit attempt, exact candidate retired, original duplex positive in both directions, unchanged original replay");
}
