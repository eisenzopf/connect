//! Original source masters outlive every generation; only physical writers move.
use super::*;
use tokio::sync::broadcast;

pub(super) const SECONDS: usize = 62;
pub(super) const MOVES: [u64; 2] = [15, 40];

#[derive(Clone)]
pub(super) struct Sample {
    pub index: u32,
    pub captured: Instant,
    pub video: bool,
    pub bytes: Arc<[u8]>,
}
#[derive(Clone)]
pub(super) struct Source {
    samples: broadcast::Sender<Sample>,
    pub force_key: Arc<AtomicBool>,
}
impl Source {
    pub fn subscribe(&self) -> broadcast::Receiver<Sample> {
        self.samples.subscribe()
    }
}

// One codec owner per original source, never reconstructed during a move. The
// bounded channel carries the actual encoded sample and original capture clock;
// neither a new encoder nor an elapsed-time estimate recreates missing input.
pub(super) fn source(
    source: usize,
    binary: &Path,
    raw: &Path,
    progress: SharedProgress,
    stop: CancellationToken,
    began: Instant,
    until: Instant,
) -> (Source, Vec<Work>) {
    progress.lock().unwrap().video_diagnostics.began = Some(began);
    let (samples, _) = broadcast::channel(16);
    let force_key = Arc::new(AtomicBool::new(false));
    let mut encoder = encoder(binary, raw);
    let video_samples = samples.clone();
    let video_stop = stop.clone();
    let requested = force_key.clone();
    let video = tokio::spawn(async move {
        let result = AssertUnwindSafe(async {
            let mut tick = tokio::time::interval_at(began, Duration::from_nanos(1_000_000_000 / 30));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            for index in 0..FRAMES {
                tokio::select! { biased; _ = video_stop.cancelled() => break, _ = tick.tick() => {} }
                if Instant::now() >= until { break; }
                let captured = Instant::now();
                let periodic_key = index % 30 == 0;
                // Preserve original short circuit: a periodic key never consumes the flag.
                let requested_key = !periodic_key && requested.swap(false, Ordering::AcqRel);
                let key = periodic_key || requested_key;
                let bytes = timeout_at(until.min(captured + AGE), encoder.frame(index, key)).await
                    .expect("original source encoding must retain its 200ms capture age");
                if key { assert!(bytes.len() > 2000, "actual fragmented original VP8 key"); }
                {
                    let mut measured = progress.lock().unwrap();
                    measured.encoded_frames += 1;
                    if requested_key {
                        measured.video_diagnostics.record("requested-key-encoded",Some((0x74000000+source as u32,index,index*3000)),Some(captured));
                    } else if periodic_key && requested.load(Ordering::Acquire) {
                        measured.video_diagnostics.record("periodic-key-request-still-pending",Some((0x74000000+source as u32,index,index*3000)),Some(captured));
                    }
                    record_frame_bin(&mut measured.encoded_bins, captured, began);
                }
                // No subscriber is not a fabricated delivery. The original
                // encoder continues even between physical-generation owners.
                let _ = video_samples.send(Sample { index, captured, video: true, bytes: bytes.into() });
            }
        }).catch_unwind().await.map_err(panic_text);
        let clean = finish_encoder(encoder).await;
        result.and(if clean {
            Ok(())
        } else {
            Err("original source encoder required forced cleanup".into())
        })
    });
    let audio_samples = samples.clone();
    let audio = tokio::spawn(async move {
        AssertUnwindSafe(async {
            let mut encoder =
                opus::Encoder::new(48000, opus::Channels::Mono, opus::Application::Voip).unwrap();
            let mut tick = tokio::time::interval_at(began, Duration::from_millis(20));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            for index in 0..(SECONDS as u32 * 50) {
                tokio::select! { biased; _ = stop.cancelled() => break, _ = tick.tick() => {} }
                if Instant::now() >= until {
                    break;
                }
                let captured = Instant::now();
                let pcm = (0..960)
                    .map(|n| {
                        ((index as f64 * 960.0 + n as f64)
                            * (330.0 + source as f64 * 110.0)
                            * std::f64::consts::TAU
                            / 48000.0)
                            .sin()
                            * 10000.0
                    })
                    .map(|v| v as i16)
                    .collect::<Vec<_>>();
                let mut bytes = [0u8; 1276];
                let count = encoder.encode(&pcm, &mut bytes).unwrap();
                let _ = audio_samples.send(Sample {
                    index,
                    captured,
                    video: false,
                    bytes: Arc::from(&bytes[..count]),
                });
            }
        })
        .catch_unwind()
        .await
        .map_err(panic_text)
    });
    (Source { samples, force_key }, vec![video, audio])
}

pub(super) fn request(until: Instant) -> RequestOptions {
    let remaining = until
        .checked_duration_since(Instant::now())
        .expect("original absolute phase cutoff");
    RequestOptions::new(remaining).unwrap()
}
pub(super) fn mutation_at(id: &str, until: Instant) -> MutationOptions {
    let mut options = mutation(id);
    options.request = request(until);
    options
}

/// Sample only actual lifecycle counts after every original client/codec join.
/// This is a one-cycle focused lifecycle assertion, not the three-cycle matrix
/// OS/thread/FD/RSS or per-owner pool qualification.
pub(super) async fn idle(f: &Fixture) {
    let began = Instant::now();
    let until = began + Duration::from_secs(10);
    loop {
        let now = Instant::now();
        let actual = f.server.service.canonical_runtime_snapshot().unwrap();
        assert_eq!(
            (
                actual.physical_peers,
                actual.peer_session_views,
                actual.live_sessions,
                actual.connections,
                actual.media_readers,
                actual.role_projections,
                actual.replacement_preparations
            ),
            (0, 0, 0, 0, 0, 0, 0),
            "actual focused lifecycle owner remains during original ten-second idle: {actual:?}"
        );
        if now >= until {
            break;
        }
        tokio::time::sleep_until(until.min(now + Duration::from_millis(50))).await;
    }
    eprintln!("representative single-cycle idle observed_ms={} strong_runtime_owners=0 os_resource_qualification=false", began.elapsed().as_millis());
}

// Only the actual publicly adopted generation contributes. Private decoder
// frames cannot compensate for a stalled old public receiver. This focused
// oracle gates clean retained routes; moving-route per-bin metrics remain
// diagnostic pending the matrix's independently validated handoff-hole lane.
// It does not invent exclusion windows or replace source-identity/skew evidence.
pub(super) fn quality(originals: &[SharedProgress], candidates: &[SharedProgress]) -> Vec<String> {
    assert_eq!(originals.len(), 6);
    assert_eq!(candidates.len(), 4);
    let original = clone_progress(originals);
    let candidate = clone_progress(candidates);
    let mut failures = Vec::new();
    for route in 0..6 {
        let mut bins = original[route].public_bins;
        let generations: &[usize] = match route {
            0 => &[0, 2],
            4 => &[1, 3],
            _ => &[],
        };
        for &generation in generations {
            for (bin, count) in bins.iter_mut().zip(candidate[generation].public_bins) {
                *bin = bin.saturating_add(count);
            }
        }
        let frames: u64 = bins[2..SECONDS].iter().map(|&v| u64::from(v)).sum();
        if frames < 1440 {
            failures.push(format!(
                "full programme public route={route} measured decoded frames={frames} minimum=1440"
            ));
        }
        for (second, &count) in bins.iter().enumerate().skip(2) {
            if !matches!(route, 0 | 1 | 4 | 5) && count < 24 {
                failures.push(format!("full programme public route={route} second={second} decoded changing frames={count} minimum=24"));
            }
        }
        eprintln!("representative original full public observation route={route} measured_start_ms=2000 measured_end_ms=62000 changing_decoded_frames={frames} bins={:?}",&bins[2..]);
    }
    failures
}

#[test]
fn full_programme_clock_and_movements_match_the_declared_matrix_contract() {
    let contract: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../contracts/gate6-transport-matrix.json"
    )))
    .unwrap();
    assert_eq!(
        contract["workload"]["movement"]["programme_ms"],
        SECONDS as u64 * 1000
    );
    assert_eq!(
        contract["workload"]["movement"]["start_offsets_ms"],
        json!([MOVES[0] * 1000, MOVES[1] * 1000])
    );
    assert_eq!(contract["workload"]["movement"]["total_moves"], 4);
    assert_eq!(FRAMES, SECONDS as u32 * 30);
}

#[test]
fn full_public_quality_never_credits_private_decode_or_invents_moving_bin_exclusions() {
    let originals = (0..6)
        .map(|_| {
            let mut measured = Progress::default();
            measured.public_bins = [30; 62];
            Arc::new(Mutex::new(measured))
        })
        .collect::<Vec<_>>();
    let candidates = (0..4)
        .map(|_| {
            let mut measured = Progress::default();
            measured.bins = [30; 62];
            Arc::new(Mutex::new(measured))
        })
        .collect::<Vec<_>>();
    originals[0].lock().unwrap().public_bins = [0; 62];
    let failures = quality(&originals, &candidates);
    assert_eq!(
        failures.len(),
        1,
        "private-only frame bins never qualify public moving route"
    );
    assert!(failures[0].contains("route=0"));
    candidates[0].lock().unwrap().public_bins = [30; 62];
    assert!(quality(&originals, &candidates).is_empty());
    originals[2].lock().unwrap().public_bins[25] = 23;
    let failures = quality(&originals, &candidates);
    assert_eq!(failures.len(), 1);
    assert!(
        failures[0].contains("route=2 second=25"),
        "clean stable sender/receiver bin is never excused as a move"
    );
    // Moving-bin admission remains the full matrix's actual-hole oracle; it is
    // not silently replaced with this fixture's stricter or guessed exclusion.
    candidates[0].lock().unwrap().public_bins[25] = 23;
    assert_eq!(quality(&originals, &candidates).len(), 1);
}

pub(super) fn video_packet(
    sequence: u32,
    timestamp: u32,
    ssrc: u32,
    marker: bool,
    payload: &[u8],
) -> sdk::RtpPacket {
    sdk::RtpPacket::new(
        sequence,
        rtp(96, sequence as u16, timestamp, ssrc, marker, payload),
    )
    .unwrap()
}

#[test]
fn actual_native_packet_counter_continues_when_the_inner_rtp_sequence_wraps() {
    let a = video_packet(65535, 3000, 0x74000000, true, &[0x10, 0, 0, 0]);
    let b = video_packet(65536, 6000, 0x74000000, true, &[0x10, 1, 0, 0]);
    assert_eq!(a.datagram_sequence, 65535);
    assert_eq!(b.datagram_sequence, 65536);
    assert_eq!(a.header().sequence_number, 65535);
    assert_eq!(b.header().sequence_number, 0);
    assert_eq!(b.header().timestamp, 6000);
}
