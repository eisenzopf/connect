//! ONE original native frame through actual signed Root, Store and native IO.
//! A packet-boundary control, not global cadence, quality or privacy proof.
use super::*;
use rvoip_core::staged_streams::{
    SuccessorIngressObserverForTest, SuccessorIngressSnapshotForTest,
};
use rvoip_uctp::substrate::{pack_rtp_datagram, unpack_rtp_datagram, RtpDatagram, RtpMediaPayload};
#[tokio::test]
async fn actual_signed_native_reserved_prepared_original_is_first_fresh_selected_successor_output()
{
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
        ActivationAudit::SuccessorOriginal,
    )
    .await;
}
fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}
fn decode(bytes: &[u8]) -> Vec<i32> {
    bytes
        .iter()
        .map(|b| {
            let v = !*b;
            let n = (((i32::from(v & 15) << 3) + 132) << (i32::from(v & 112) >> 4)) - 132;
            if v & 128 != 0 {
                -n
            } else {
                n
            }
        })
        .collect()
}
fn pattern(seed: u32) -> Vec<u8> {
    (0..160u32)
        .map(|n| {
            if ((n.wrapping_mul(37) ^ seed ^ (n >> 2)).count_ones() & 1) == 0 {
                0x92
            } else {
                0x12
            }
        })
        .collect()
}
fn send_one(
    peer: &mut Peer,
    payload: Vec<u8>,
    ssrc: u32,
) -> std::result::Result<(u32, u32, u64), String> {
    peer.sequence = peer.sequence.wrapping_add(1);
    let timestamp = (peer.clock.elapsed().as_micros() * 8 / 1000) as u32;
    let hash = fingerprint(&payload);
    let bytes = pack_rtp_datagram(&RtpDatagram {
        flags: 0,
        stream_local_id: peer.handle,
        seq: peer.sequence,
        rtp: RtpMediaPayload {
            payload: payload.into(),
            payload_type: 0,
            sequence_number: peer.sequence as u16,
            timestamp,
            ssrc,
        },
    })
    .map_err(|e| format!("pack {e:?}"))?;
    peer.client
        .send_native_media_datagram(&bytes)
        .map_err(|e| format!("send {e:?}"))?;
    Ok((peer.sequence, timestamp, hash))
}
async fn snap(
    observer: &SuccessorIngressObserverForTest,
    until: tokio::time::Instant,
) -> std::result::Result<SuccessorIngressSnapshotForTest, String> {
    loop {
        match observer.snapshot() {
            Ok(v) => return Ok(v),
            Err(rvoip_core::staged_streams::StreamControlError::MetadataBusy)
                if tokio::time::Instant::now() < until =>
            {
                tokio::task::yield_now().await
            }
            Err(e) => return Err(format!("snapshot {e:?}")),
        }
    }
}
#[derive(Debug)]
struct NativeOutput {
    handle: u16,
    sequence: u32,
    timestamp: u32,
    ssrc: u32,
    payload_len: usize,
    payload_hash: u64,
}
// Every bounded native row is retained. The FIRST retained packet must qualify;
// an unknown retained tail is a failed/unqualified observation, never skipped.
// Other readers may still drain the existing private preparation/OLD close.
async fn first_retained(
    peers: &mut [Peer],
    expected_hash: Option<u64>,
    until: tokio::time::Instant,
    rows: &mut Vec<String>,
) -> std::result::Result<NativeOutput, String> {
    loop {
        let receives = peers
            .iter_mut()
            .enumerate()
            .filter(|(_, p)| p.live)
            .map(|(i, p)| {
                Box::pin(async move { (i, p.client.receive_native_media_datagram().await) })
            })
            .collect::<Vec<_>>();
        if receives.is_empty() {
            return Err("all native readers closed".into());
        }
        let ((i, result), _, _) =
            tokio::time::timeout_at(until, futures_util::future::select_all(receives))
                .await
                .map_err(|_| "original receive bound".to_owned())?;
        if rows.len() >= 128 {
            return Err("bounded native observation overflow".into());
        }
        let bytes = match result {
            Ok(v) => v,
            Err(e) if i == 1 => {
                peers[i].live = false;
                rows.push(format!("old reader closed {e:?}"));
                continue;
            }
            Err(e) => return Err(format!("native reader{i} {e:?}")),
        };
        let packet = unpack_rtp_datagram(&bytes).map_err(|e| format!("unpack {e:?}"))?;
        let hash = fingerprint(&packet.rtp.payload);
        rows.push(format!(
            "receiver={i} handle={} sequence={} timestamp={} ssrc={} pt={} len={} hash={hash}",
            packet.stream_local_id,
            packet.seq,
            packet.rtp.timestamp,
            packet.rtp.ssrc,
            packet.rtp.payload_type,
            packet.rtp.payload.len()
        ));
        if i != 0 {
            if expected_hash == Some(hash) {
                return Err(
                    "proof-associated successor output reached a different native reader".into(),
                );
            }
            continue;
        }
        if packet.stream_local_id != peers[0].handle || packet.rtp.payload_type != 0 {
            return Err("first retained packet changed actual native audio binding".into());
        }
        let pcm = decode(&packet.rtp.payload);
        if pcm.len() != 160 || !pcm.iter().any(|sample| *sample != 0) {
            return Err("first retained packet is not actual nonzero decoded PCMU160".into());
        }
        if expected_hash.is_some_and(|expected| expected != hash) {
            return Err("first retained post-cut packet is not the proof-associated original output (unclassified tail retained)".into());
        }
        return Ok(NativeOutput {
            handle: packet.stream_local_id,
            sequence: packet.seq,
            timestamp: packet.rtp.timestamp,
            ssrc: packet.rtp.ssrc,
            payload_len: packet.rtp.payload.len(),
            payload_hash: hash,
        });
    }
}
// Readiness's continuously published retained audio has a real public copy
// on OLD1 and a private copy on candidate2. Neither reader was drained by the
// private decoder's own receive loop. Classify/archive that history BEFORE
// sending either new mark; it can never earn fresh successor output credit.
async fn readiness_history(
    peers: &mut [Peer],
    observer: &SuccessorIngressObserverForTest,
    until: tokio::time::Instant,
    rows: &mut Vec<String>,
) -> std::result::Result<(), String> {
    let before = snap(observer, until).await?;
    if before.phase != Some("armed")
        || before.captures != 0
        || before.releases != 0
        || before.queued != 0
        || before.packet_epoch_current_at_admission
        || !before.outputs.is_empty()
        || before.missed != 0
    {
        return Err("readiness history is not before the original unclaimed reservation".into());
    }
    // Same packet bound and measured reader-first quiet convention already
    // used by final_control::observer's signed-readiness-backlog phase.
    let start = tokio::time::Instant::now();
    let minimum_tail = start + Duration::from_millis(300);
    let mut last_packet = start;
    if minimum_tail >= until {
        return Err("original cutoff cannot contain readiness history drain".into());
    }
    loop {
        if rows.len() >= 2048 {
            return Err("bounded readiness history overflow".into());
        }
        let now = tokio::time::Instant::now();
        if now >= until {
            return Err("readiness history did not quiesce within original cutoff".into());
        }
        let quiet_eligible =
            now >= minimum_tail && now.duration_since(last_packet) >= Duration::from_millis(200);
        // Reader FIRST: queued data cannot lose to the quiet decision. No new
        // media is sent here and the original marked-input clock has not begun.
        let event = {
            let receiving = peers
                .iter_mut()
                .enumerate()
                .filter(|(_, p)| p.live)
                .map(|(i, p)| {
                    Box::pin(async move { (i, p.client.receive_native_media_datagram().await) })
                })
                .collect::<Vec<_>>();
            if receiving.is_empty() {
                return Err("all history native readers closed".into());
            }
            tokio::select! { biased;
                ((i,result),_,_) = futures_util::future::select_all(receiving) => Some((i,result)),
                _ = tokio::time::sleep_until(until.min(now+Duration::from_millis(1))) => None,
            }
        };
        let Some((i, result)) = event else {
            if quiet_eligible {
                let after = snap(observer, until).await?;
                if after.phase != Some("armed")
                    || after.captures != 0
                    || after.releases != 0
                    || after.queued != 0
                    || after.packet_epoch_current_at_admission
                    || !after.outputs.is_empty()
                    || after.missed != 0
                {
                    return Err(
                        "history drain changed original reservation or gained output authority"
                            .into(),
                    );
                }
                return Ok(());
            }
            continue;
        };
        let bytes = result.map_err(|e| format!("readiness history native reader{i} {e:?}"))?;
        let packet =
            unpack_rtp_datagram(&bytes).map_err(|e| format!("readiness history unpack {e:?}"))?;
        let hash = fingerprint(&packet.rtp.payload);
        rows.push(format!("history_receiver={i} handle={} sequence={} timestamp={} ssrc={} pt={} len={} hash={hash}",packet.stream_local_id,packet.seq,packet.rtp.timestamp,packet.rtp.ssrc,packet.rtp.payload_type,packet.rtp.payload.len()));
        // Retained0 has no incoming readiness source. It must remain unknown/
        // refused here rather than being silently treated as an OLD tail.
        if !matches!(i, 1 | 2)
            || packet.stream_local_id != peers[i].handle
            || packet.rtp.payload_type != 0
            || packet.rtp.payload.len() != 160
        {
            return Err(
                "readiness history contradicts actual old/private native audience or binding"
                    .into(),
            );
        }
        if decode(&packet.rtp.payload).len() != 160 {
            return Err("readiness history PCMU shape".into());
        }
        last_packet = tokio::time::Instant::now();
    }
}
pub(super) struct Evidence {
    snapshot: SuccessorIngressSnapshotForTest,
    raw: Vec<String>,
    readiness_history: Vec<String>,
    source_sequence: u32,
    source_timestamp: u32,
    source_hash: u64,
    received: NativeOutput,
    old: NativeOutput,
    candidate: String,
    retained: String,
    record: String,
    generations: (u64, u64),
    source_handle: u16,
    retained_handle: u16,
}
impl Evidence {
    pub(super) fn validate(self) {
        eprintln!("ACTUAL_SIGNED_NATIVE_SUCCESSOR record={} snapshot={:?} old_wire={:?} successor_wire={:?} readiness_history={:?} raw={:?}",self.record,self.snapshot,self.old,self.received,self.readiness_history,self.raw);
        let s = self.snapshot;
        assert_eq!(s.missed, 0);
        assert_eq!(s.captures, 1);
        assert_eq!(s.releases, 1);
        assert_eq!(s.queued, 1);
        assert!(s.packet_epoch_current_at_admission && s.original_admitted_age_us < 200_000);
        let wire = s.wire.unwrap();
        assert!(wire.original_owner_exact);
        assert_eq!(wire.connection, self.candidate);
        assert_eq!(wire.stream_local_id, self.source_handle);
        assert_eq!(
            (wire.sequence, wire.timestamp, wire.ssrc),
            (self.source_sequence, self.source_timestamp, 0x4736_8f01)
        );
        assert_eq!(wire.payload_fingerprint, self.source_hash);
        assert_eq!(wire.payload_len, 160);
        assert_eq!(
            (wire.connection_generation, wire.stream_generation),
            self.generations
        );
        assert_eq!(s.outputs.len(), 1);
        let first = &s.outputs[0];
        assert!(first.selected_exact_target);
        assert_eq!(first.contribution_count, 1);
        assert_eq!(first.target, self.retained);
        assert_eq!(first.stream_local_id, self.retained_handle);
        assert_eq!(first.payload_len, 160);
        assert_eq!(first.payload_fingerprint, self.received.payload_hash);
        assert_eq!(self.received.handle, self.retained_handle);
        assert_eq!(self.old.handle, self.retained_handle);
        assert_eq!(self.received.payload_len, 160);
        let sequence_step = self.received.sequence.wrapping_sub(self.old.sequence);
        assert!(sequence_step > 0 && sequence_step < 1 << 31);
        // Native send_counted translates the Core timestamp before packing and
        // may retire SSRC when one-source clock origin changes at handoff.
        // Compare wire clocks only within one observed SSRC; archive both clock
        // namespaces rather than inventing Core==wire or stable-SSRC evidence.
        if self.received.ssrc == self.old.ssrc {
            let step = self.received.timestamp.wrapping_sub(self.old.timestamp);
            assert!(step > 0 && step < 1 << 31);
        }
    }
}
pub(super) async fn verify(
    f: &Fixture,
    key: &str,
    command: &TransitionCommand,
    candidate: &str,
    peers: &mut [Peer],
    custody: &connect_service::canonical_media_projection_fixture::PreparationCustodyProbe,
) -> std::result::Result<Evidence, String> {
    let result = inner(f, key, command, candidate, peers, custody).await;
    custody.release(); // errors never strand the actual original Root rendezvous
    result
}
async fn inner(
    f: &Fixture,
    key: &str,
    command: &TransitionCommand,
    candidate: &str,
    peers: &mut [Peer],
    custody: &connect_service::canonical_media_projection_fixture::PreparationCustodyProbe,
) -> std::result::Result<Evidence, String> {
    tokio::time::timeout(Duration::from_secs(5), custody.entered())
        .await
        .map_err(|_| "actual ManagedFinalPrepared not entered".to_owned())?;
    let until = custody
        .original_cutoff()
        .map_err(|e| format!("original cutoff {e:?}"))?;
    let original = status(f, key, &command.operation_id).await;
    let actual_private = f
        .server
        .service
        .canonical_replacement_audio_observer_for_test(&original.record_id)
        .map_err(|e| format!("private original {e:?}"))?
        .snapshot()
        .map_err(|e| format!("private metadata {e:?}"))?;
    let actual_binding = actual_private
        .audio
        .as_ref()
        .ok_or("actual private native binding absent")?
        .target
        .clone();
    let readiness = peers[2]
        .client
        .native_replacement_readiness()
        .map_err(|e| format!("signed readiness {e:?}"))?;
    if readiness.record_id() != original.record_id
        || readiness.operation_id() != Some(command.operation_id.as_str())
        || readiness.audio().stream_local_id != peers[2].handle
        || readiness.audio().payload_type != 0
    {
        return Err("different original signed readiness/native role".into());
    }
    if original.state != OperationState::Preparing {
        return Err("original signed Root not preparing".into());
    }
    let observer = f
        .server
        .service
        .canonical_replacement_successor_observer_for_test(&original.record_id)
        .map_err(|e| format!("actual reservation {e:?}"))?;
    if snap(&observer, until).await?.phase != Some("armed") {
        return Err(
            "slot is not original unclaimed ARMED (late private tail cannot substitute)".into(),
        );
    }
    let mut readiness_history_rows = Vec::new();
    readiness_history(peers, &observer, until, &mut readiness_history_rows)
        .await
        .map_err(|e| format!("{e}; readiness_history={readiness_history_rows:?}"))?;
    let mut raw = Vec::new();
    // The classified readiness history is now quiet BEFORE either new mark. Only this OLD peer is sent
    // here; require its first actual retained nonzero output before the mark.
    send_one(&mut peers[1], pattern(0x17), 0x4736_7f01)?;
    let old = first_retained(peers, None, until, &mut raw)
        .await
        .map_err(|e| format!("{e}; raw={raw:?}"))?;
    // No more candidate packets are sent after this ONE mark until output.
    let payload = pattern(0x52);
    let (sequence, timestamp, source_hash) = send_one(&mut peers[2], payload, 0x4736_8f01)?;
    loop {
        let s = snap(&observer, until).await?;
        if s.phase == Some("held") && s.captures == 1 {
            if s.queued != 0 || s.packet_epoch_current_at_admission || !s.outputs.is_empty() {
                return Err("unreleased original reached packet authority/output".into());
            }
            let wire = s
                .wire
                .as_ref()
                .ok_or("actual native original identity absent")?;
            if wire.connection != candidate
                || wire.stream_local_id != peers[2].handle
                || wire.sequence != sequence
                || wire.timestamp != timestamp
                || wire.payload_fingerprint != source_hash
                || !wire.original_owner_exact
            {
                return Err("wrong original claimed".into());
            }
            break;
        }
        if tokio::time::Instant::now() >= until {
            return Err("original never captured within its unchanged age".into());
        }
        tokio::task::yield_now().await;
    }
    custody.release();
    let final_snapshot = loop {
        let s = snap(&observer, until).await?;
        if !s.outputs.is_empty() && s.queued == 1 {
            break s;
        }
        if tokio::time::Instant::now() >= until {
            return Err("actual guarded writer completion not observed".into());
        }
        tokio::task::yield_now().await;
    };
    let expected = final_snapshot.outputs.first().ok_or("no selected output")?;
    if expected.payload_fingerprint == old.payload_hash {
        return Err("new original output is not distinguishable from observed OLD baseline".into());
    }
    if !expected.selected_exact_target
        || expected.contribution_count != 1
        || expected.stream_local_id != peers[0].handle
        || expected.payload_len != 160
    {
        return Err("successful output is not one selected original contribution".into());
    }
    // Real Service mixes at48kHz. Match the successful actual output payload,
    // not unchanged source PCM through the real8->48->8 resampling path.
    let received = first_retained(peers, Some(expected.payload_fingerprint), until, &mut raw)
        .await
        .map_err(|e| format!("{e}; raw={raw:?}"))?;
    let done = loop {
        if tokio::time::Instant::now() >= until {
            return Err("same original Root completion exceeded its original cutoff".into());
        }
        let current = tokio::time::timeout_at(until, status(f, key, &command.operation_id))
            .await
            .map_err(|_| "same original Root status read reached original cutoff".to_owned())?;
        // Even Completed returned at/after the original cutoff cannot qualify.
        if tokio::time::Instant::now() >= until {
            return Err(format!(
                "same original Root did not complete within cutoff: {current:?}"
            ));
        }
        if current.state == OperationState::Completed {
            break current;
        }
        // SAME existing wait_state cadence: returned read owns no authority.
        // A yield alone immediately re-admits another metadata reader and can
        // starve the historical capture writer this fixture is waiting for.
        tokio::time::sleep_until(
            until.min(tokio::time::Instant::now() + Duration::from_millis(20)),
        )
        .await;
    };
    if done.record_id != original.record_id {
        return Err("new original operation substituted".into());
    }
    if done
        .effects
        .iter()
        .filter(|effect| {
            effect.provider == "native-replacement-commit" && effect.state == EffectState::Confirmed
        })
        .count()
        != 1
    {
        return Err("exact original commit effect missing/repeated".into());
    }
    let retained = expected.target.clone();
    let retained_expected:String=rusqlite::Connection::open(&f.config.database).map_err(|e|e.to_string())?.query_row("SELECT m.runtime_connection_id FROM connection_offers m JOIN connections c ON c.id=m.connection_id JOIN native_connection_lifetimes life ON life.connection_id=c.id AND life.generation=c.generation AND life.runtime_epoch=c.observed_runtime_id AND life.retired=0 WHERE c.id='codec-source' AND c.generation=1 AND c.observed_runtime_id=?1 AND c.state IN ('ready','active')",[f.server.store.runtime_epoch()],|row|row.get(0)).map_err(|e|e.to_string())?;
    if retained != retained_expected {
        return Err("output did not use original selected retained Core route".into());
    }
    Ok(Evidence {
        snapshot: final_snapshot,
        raw,
        readiness_history: readiness_history_rows,
        source_sequence: sequence,
        source_timestamp: timestamp,
        source_hash,
        received,
        old,
        candidate: candidate.into(),
        retained: retained_expected,
        record: original.record_id,
        generations: (
            actual_binding.connection_generation,
            actual_binding.generation,
        ),
        source_handle: peers[2].handle,
        retained_handle: peers[0].handle,
    })
}

#[path = "successor_first/negatives.rs"]
pub(super) mod negatives;
