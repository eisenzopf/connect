//! Actual original private egress, followed by cancellation and joined owners.
//! No acknowledgement, synthetic clock, readiness timeout, PLC or inferred PCM.
use super::*;
use std::sync::Arc;

#[derive(Clone, Copy)]
pub(crate) enum Case {
    G711Emission,
    G711Contiguous,
    OpusEmission,
    OpusContiguous,
}
impl Case {
    fn codec(self) -> &'static str {
        match self {
            Self::G711Emission | Self::G711Contiguous => "pcmu",
            _ => "opus",
        }
    }
    fn rate(self) -> u32 {
        if self.codec() == "opus" {
            48000
        } else {
            8000
        }
    }
    fn contiguous(self) -> bool {
        matches!(self, Self::G711Contiguous | Self::OpusContiguous)
    }
    pub(crate) fn configure_offer(self, offer: &mut Value) {
        offer["capabilities"]["audio_codecs"] = json!([{"name":self.codec(),
            "params":{"sample_rate":self.rate(),"channels":1}}]);
        offer["streams_offered"][0]["codec_preferences"] = json!([self.codec()]);
    }
}
struct Observed {
    rows: Vec<Value>,
    maximum_contiguous_samples: u64,
    drained: u64,
}
async fn observe(client: &mut CanonicalClient, prepared: &mut Prepared) -> Observed {
    assert!(prepared.readiness.video().is_none());
    let drained = timeout_at(prepared.cutoff, async {
        for count in 0..4096u64 {
            match tokio::time::timeout(
                Duration::from_millis(1),
                client.receive_native_media_datagram(),
            )
            .await
            {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => panic!("cadence setup reader failed before observation: {error}"),
                Err(_) => return count,
            }
        }
        panic!("cadence setup queue never reached an actual empty read boundary");
    })
    .await
    .expect("cadence setup exhausted the original preparation cutoff");
    let began = Instant::now();
    let quantum = u64::from(prepared.audio.rate() / 50);
    let mut rows = Vec::with_capacity(6);
    let mut previous: Option<(u32, u16, u32)> = None;
    let mut maximum_contiguous_samples = 0;
    timeout_at(prepared.cutoff, async {
        while rows.len() < 6 {
            let bytes = client
                .receive_native_media_datagram()
                .await
                .expect("cadence original reader failed while observing actual source");
            let ObservedNativeMediaDatagram::Rtp(packet) = observe_native_media_datagram(&bytes)
                .expect("cadence received malformed original native packet")
            else {
                continue;
            };
            assert_eq!(
                packet.stream_local_id,
                prepared.readiness.audio().stream_local_id,
                "cadence received a foreign stream before readiness"
            );
            let p = packet.rtp.into_packet();
            assert_eq!(
                p.header.payload_type,
                prepared.readiness.audio().payload_type,
                "cadence payload type changed its signed codec"
            );
            let delta = previous.map(|(ssrc, sequence, timestamp)| {
                assert_eq!(
                    p.header.ssrc, ssrc,
                    "cadence original preparation SSRC changed"
                );
                assert_eq!(
                    p.header.sequence_number,
                    sequence.wrapping_add(1),
                    "cadence setup lost or reordered an original packet"
                );
                p.header.timestamp.wrapping_sub(timestamp)
            });
            prepared
                .audio
                .decode(
                    p.header.ssrc,
                    p.header.sequence_number,
                    p.header.timestamp,
                    &p.payload,
                )
                .expect("cadence packet did not actually decode the negotiated 20ms PCM");
            assert!(prepared.audio.samples() >= quantum);
            maximum_contiguous_samples = maximum_contiguous_samples.max(prepared.audio.samples());
            rows.push(
                json!({"ssrc":p.header.ssrc,"sequence":p.header.sequence_number,
                "timestamp_rtp":p.header.timestamp,"delta_rtp":delta,
                "decoded_samples":quantum,"contiguous_decoded_samples":prepared.audio.samples(),
                "received_at_us":began.elapsed().as_micros()}),
            );
            previous = Some((p.header.ssrc, p.header.sequence_number, p.header.timestamp));
        }
    })
    .await
    .expect("cadence original source did not emit six decoded packets before its original cutoff");
    Observed {
        rows,
        maximum_contiguous_samples,
        drained,
    }
}
pub(crate) async fn observe_and_join(
    f: Fixture,
    mut target: CanonicalClient,
    mut prepared: Prepared,
    mut peers: Vec<Peer>,
    mut controller: CanonicalHttpClient,
    command: TransitionCommand,
    key: connect_store::IssuedCredential,
    case: Case,
) {
    assert_eq!(peers.len(), 2);
    assert_eq!(prepared.audio.rate(), case.rate());
    let cutoff = prepared.cutoff;
    let initial = timeout_at(cutoff, status(&f, &key.id, &command.operation_id))
        .await
        .unwrap();
    assert_eq!(initial.state, OperationState::Preparing);
    assert!(initial.result.is_none());
    assert_eq!(initial.effects.len(), 5);
    assert!(initial
        .effects
        .iter()
        .all(|e| e.attempt == 1 && e.state == EffectState::Confirmed));
    let service = Arc::downgrade(&f.server.service);
    let session = {
        let current = f
            .server
            .service
            .canonical_session_metadata_for_test("native-session")
            .unwrap();
        Arc::downgrade(&current)
    };
    assert!(service.strong_count() > 0 && session.strong_count() > 0);
    let observation = observe(&mut target, &mut prepared).await;
    drop(prepared); // Actual libopus/G711 decoder is not retained in the report.
    let after = timeout_at(cutoff, status(&f, &key.id, &command.operation_id))
        .await
        .unwrap();
    assert_eq!(after.state, OperationState::Preparing);
    assert_eq!(after.effects, initial.effects);
    assert!(
        after.result.is_none(),
        "observation alone must not acknowledge readiness"
    );
    timeout_at(cutoff, controller.request_value("operation.cancel",
        json!({"operation_id":command.operation_id,"reason":"original cadence observation complete"}),
        CommandScope {conversation_id:Some(command.conversation_id.clone()),
            session_id:Some(command.session_id.clone()),connection_id:command.connection_id.clone(),
            operation_id:Some("cancel-original-preparation-cadence".into()),..Default::default()}))
        .await.expect("cadence cleanup original cutoff elapsed before cancellation").unwrap().ack().unwrap();
    timeout_at(
        cutoff,
        stopped(
            &f,
            &key.id,
            &command.operation_id,
            OperationState::Cancelled,
        ),
    )
    .await
    .expect("cadence cleanup original movement did not retire before cutoff");
    timeout_at(
        cutoff,
        probe(
            &mut peers,
            0,
            &[1],
            "cadence-cancel-retains-original-remote-marked-pcm",
        ),
    )
    .await
    .expect("cadence cleanup lost the original live route");
    timeout_at(
        cutoff,
        probe(
            &mut peers,
            1,
            &[0],
            "cadence-cancel-retains-original-source-marked-pcm",
        ),
    )
    .await
    .expect("cadence cleanup lost the original reverse live route");
    let mut clients = peers
        .into_iter()
        .map(|peer| peer.client)
        .collect::<Vec<_>>();
    clients.push(target);
    let joins = timeout_at(
        cutoff,
        futures_util::future::join_all(clients.into_iter().map(|client| client.shutdown())),
    )
    .await
    .expect("cadence cleanup original physical client owners did not join");
    timeout_at(cutoff, wait_slots(&f, 0))
        .await
        .expect("cadence cleanup retained original physical slots");
    assert_eq!(
        f.server
            .service
            .canonical_runtime_snapshot()
            .unwrap()
            .replacement_preparations,
        0
    );
    let movement = timeout_at(
        cutoff,
        super::super::super::sdk_movements::before_shutdown(&f),
    )
    .await
    .expect("cadence cleanup original controller did not drain");
    drop(controller);
    let server = timeout_at(cutoff, f.server.shutdown())
        .await
        .expect("cadence cleanup original server did not join");
    super::super::super::sdk_movements::shutdown_joined(movement);
    assert_eq!(joins.len(), 3);
    assert!(
        joins.iter().all(|r| r.is_ok()),
        "cadence original client shutdown reported a failure: {joins:?}"
    );
    server.expect("cadence original server shutdown reported a failure");
    assert!(
        service.upgrade().is_none(),
        "cadence joined server still retains its original Service"
    );
    assert!(
        session.upgrade().is_none(),
        "cadence joined server still retains its original Session"
    );
    let required = u64::from(case.rate()) * 60 / 1000;
    let contiguous = observation.maximum_contiguous_samples >= required;
    let witness = json!({"codec":case.codec(),"clock_rate_hz":case.rate(),"packets_decoded":6,
        "rows":observation.rows,"drained_before_decoder":observation.drained,
        "maximum_contiguous_samples":observation.maximum_contiguous_samples,"required_contiguous_samples":required,
        "original_root_id":initial.record_id,"original_expires_at":initial.expires_at,
        "state_before_ready":"preparing","ready_submitted":false,"old_marked_routes_live_before_and_after":true,
        "original_clients_joined":3,"original_server_joined":true,"original_movement_retired":true,
        "original_service_weak_expired":true,"original_session_weak_expired":true,
        "all_joins_before_original_cutoff":Instant::now()<cutoff,"contiguous":contiguous,
        "verdict":if case.contiguous(){"contiguous"}else{"emission-control"}});
    assert!(
        Instant::now() < cutoff,
        "cadence original cutoff elapsed during original joins"
    );
    eprintln!("native-preparation-cadence-original-joined {witness}");
    if case.contiguous() {
        assert!(contiguous, "actual original preparation lacks three contiguous decoded 20ms frames after original joins");
    }
}
async fn exercise(case: Case) {
    super::super::exercise_replacement_with_av_control(
        true,
        true,
        false,
        false,
        false,
        None,
        false,
        Some((false, false, false)),
        false,
        false,
        super::super::ActivationAudit::PreparationCadence(case),
    )
    .await;
}
#[tokio::test]
async fn original_g711_preparation_emits_six_decoded_frames_and_joins() {
    exercise(Case::G711Emission).await;
}
#[tokio::test]
async fn original_g711_preparation_has_three_contiguous_decoded_frames_before_adoption() {
    exercise(Case::G711Contiguous).await;
}
#[tokio::test]
async fn original_opus_preparation_emits_six_decoded_frames_and_joins() {
    exercise(Case::OpusEmission).await;
}
#[tokio::test]
async fn original_opus_preparation_has_three_contiguous_decoded_frames_before_adoption() {
    exercise(Case::OpusContiguous).await;
}
