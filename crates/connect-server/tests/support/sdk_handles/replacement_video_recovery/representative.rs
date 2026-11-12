//! Production regression of the actual matrix topology; the full-programme selector also runs the original 62-second workload.
//! Four physical/signing clients, two Sessions, six sources and four original
//! replacements, including return to the same original Device. This is a real codec/Store regression, not a matrix result.
use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tokio_util::sync::CancellationToken;

const SOURCE_FOR_RECEIVER: [usize; 6] = [3, 0, 1, 2, 5, 4];
const OWNER: [usize; 6] = [0, 1, 2, 3, 0, 1];
const SESSION: [usize; 6] = [0, 0, 0, 0, 1, 1];
const FRAMES: u32 = 62 * 30;
#[path = "representative_data.rs"]
mod data;
#[path = "representative_programme.rs"]
mod programme;
const AGE: Duration = Duration::from_millis(200);
type Work = tokio::task::JoinHandle<std::result::Result<(), String>>;

// Finite control markers only; never called from a frame/packet loop. UTC is
// sampled between the two monotonic readings, so the printed pair bounds its
// alignment uncertainty instead of inventing an atomic cross-clock instant.
fn phase_timing(
    event: &'static str,
    began: Instant,
    round: Option<usize>,
    bins: Option<(usize, usize)>,
) {
    let mono_before = Instant::now();
    let utc = chrono::Utc::now();
    let mono_after = Instant::now();
    let elapsed = |at: Instant| {
        if let Some(duration) = at.checked_duration_since(began) {
            duration.as_micros() as i64
        } else {
            -(began.duration_since(at).as_micros() as i64)
        }
    };
    let before_us = elapsed(mono_before);
    let after_us = elapsed(mono_after);
    let midpoint_us = before_us + (after_us - before_us) / 2;
    let began_utc = utc
        .checked_sub_signed(chrono::Duration::microseconds(midpoint_us))
        .map(|time| time.to_rfc3339_opts(chrono::SecondsFormat::Micros, true));
    eprintln!(
        "representative timing {}",
        json!({"event":event,"round":round,"utc":utc.to_rfc3339_opts(chrono::SecondsFormat::Micros,true),
            "elapsed_us_before":before_us,"elapsed_us_after":after_us,
            "clock_pair_window_us":after_us-before_us,"programme_began_utc_estimate":began_utc,
            "complete_bin_range":bins})
    );
}

fn panic_text(value: Box<dyn std::any::Any + Send>) -> String {
    value
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| value.downcast_ref::<&str>().map(|s| (*s).into()))
        .unwrap_or_else(|| "original codec/media task panicked".into())
}
fn marker(source: usize) -> u32 {
    40 + source as u32 * 25
}
fn short() -> RequestOptions {
    RequestOptions::new(Duration::from_secs(1)).unwrap()
}

const AUDIO_RESET_HISTORY: usize = 8;
#[derive(Clone, Copy, Debug)]
struct AudioResetEvent {
    previous: readiness_audio::RtpHeader,
    current: readiness_audio::RtpHeader,
    reasons: readiness_audio::ResetReasons,
    observed_at: Instant,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AudioResetReport {
    elapsed_us: Option<u128>,
    previous: readiness_audio::RtpHeader,
    current: readiness_audio::RtpHeader,
    reasons: readiness_audio::ResetReasons,
}
#[derive(Clone, Debug, Default)]
struct AudioContinuity {
    last: Option<readiness_audio::ContinuityObservation>,
    resets: u64,
    ssrc_changes: u64,
    sequence_discontinuities: u64,
    timestamp_discontinuities: u64,
    recent: [Option<AudioResetEvent>; AUDIO_RESET_HISTORY],
    next: usize,
    len: usize,
}
impl AudioContinuity {
    fn observe(&mut self, observation: readiness_audio::ContinuityObservation, now: Instant) {
        if self
            .last
            .is_some_and(|old| old.decode_serial == observation.decode_serial)
        {
            return;
        }
        self.last = Some(observation);
        if !observation.reset.any() {
            return;
        }
        let Some(previous) = observation.previous else {
            return;
        };
        self.resets = self.resets.saturating_add(1);
        self.ssrc_changes = self
            .ssrc_changes
            .saturating_add(u64::from(observation.reset.ssrc_changed));
        self.sequence_discontinuities = self
            .sequence_discontinuities
            .saturating_add(u64::from(observation.reset.sequence_discontinuity));
        self.timestamp_discontinuities = self
            .timestamp_discontinuities
            .saturating_add(u64::from(observation.reset.timestamp_discontinuity));
        self.recent[self.next] = Some(AudioResetEvent {
            previous,
            current: observation.current,
            reasons: observation.reset,
            observed_at: now,
        });
        self.next = (self.next + 1) % AUDIO_RESET_HISTORY;
        self.len = (self.len + 1).min(AUDIO_RESET_HISTORY);
    }
    fn reports(&self, began: Instant) -> [Option<AudioResetReport>; AUDIO_RESET_HISTORY] {
        let mut reports = [None; AUDIO_RESET_HISTORY];
        let first = (self.next + AUDIO_RESET_HISTORY - self.len) % AUDIO_RESET_HISTORY;
        for (offset, report) in reports.iter_mut().enumerate().take(self.len) {
            *report =
                self.recent[(first + offset) % AUDIO_RESET_HISTORY].map(|event| AudioResetReport {
                    elapsed_us: event
                        .observed_at
                        .checked_duration_since(began)
                        .map(|d| d.as_micros()),
                    previous: event.previous,
                    current: event.current,
                    reasons: event.reasons,
                });
        }
        reports
    }
}
// Finite control-boundary reports only. No packet logging, payload, new clock,
// authority lookup, polling owner or acceptance change is introduced.
fn audio_continuity_snapshot(
    stage: &'static str,
    round: usize,
    began: Instant,
    progress: &[Progress],
) {
    for (receiver, value) in progress.iter().enumerate() {
        let audio = &value.audio_continuity;
        eprintln!("representative audio continuity stage={stage} round={round} receiver={receiver} consecutive_samples={} last={:?} resets={} ssrc_changes={} sequence_discontinuities={} timestamp_discontinuities={} recent={:?}",
            value.audio_samples, audio.last, audio.resets, audio.ssrc_changes,
            audio.sequence_discontinuities, audio.timestamp_discontinuities, audio.reports(began));
    }
}

/// Bounded scalar observation. The full media stays with its original reader.
#[derive(Clone, Debug)]
struct Progress {
    encoded_frames: u64,
    encoded_bins: [u16; 62],
    published_frames: u64,
    published_bins: [u16; 62],
    published_video_packets: u64,
    published_audio_packets: u64,
    video_read_events: u64,
    video_packets: u64,
    assembled_frames: u64,
    assembled_keys: u64,
    decode_attempts: u64,
    audio_read_events: u64,
    audio_packets: u64,
    received_datagrams: u64,
    other_datagrams: u64,
    video_losses: u64,
    late_video_packets: u64,
    skipped_delta_frames: u64,
    recovered_keys: u64,
    feedback_requests: u64,
    publisher_feedback_received: u64,
    publisher_feedback_used: u64,
    video_waiting_key: bool,
    previous_decode: Option<(u32, connect_client::VideoSequenceRange)>,
    current_decode: Option<(u32, connect_client::VideoSequenceRange)>,
    assembly: connect_client::VideoAssemblyDiagnostics,
    public: bool,
    public_since: Option<Instant>,
    public_bins: [u16; 62],
    frames: u64,
    keys: u64,
    fragmented_keys: u64,
    audio_samples: u64,
    audio_peak_samples: u64,
    // Nonincreasing consecutive samples among actual new PCM decodes.
    audio_run_nonincreases: u64,
    audio_last_nonincrease: Option<Instant>,
    audio_continuity: AudioContinuity,
    last_video: Option<Instant>,
    last_audio: Option<Instant>,
    last_key: Option<Instant>,
    timestamp: Option<u32>,
    epoch: Option<sdk::VideoReceiveEpoch>,
    // Fixed programme bins; no per-packet diagnostic history is retained.
    bins: [u16; 62],
}
impl Default for Progress {
    fn default() -> Self {
        Self {
            encoded_frames: Default::default(),
            encoded_bins: [0; 62],
            published_frames: Default::default(),
            published_bins: [0; 62],
            published_video_packets: Default::default(),
            published_audio_packets: Default::default(),
            video_read_events: Default::default(),
            video_packets: Default::default(),
            assembled_frames: Default::default(),
            assembled_keys: Default::default(),
            decode_attempts: Default::default(),
            audio_read_events: Default::default(),
            audio_packets: Default::default(),
            received_datagrams: Default::default(),
            other_datagrams: Default::default(),
            video_losses: Default::default(),
            late_video_packets: Default::default(),
            skipped_delta_frames: Default::default(),
            recovered_keys: Default::default(),
            feedback_requests: Default::default(),
            publisher_feedback_received: Default::default(),
            publisher_feedback_used: Default::default(),
            video_waiting_key: Default::default(),
            previous_decode: Default::default(),
            current_decode: Default::default(),
            assembly: Default::default(),
            public: false,
            public_since: None,
            public_bins: [0; 62],
            frames: Default::default(),
            keys: Default::default(),
            fragmented_keys: Default::default(),
            audio_samples: Default::default(),
            audio_peak_samples: Default::default(),
            audio_run_nonincreases: Default::default(),
            audio_last_nonincrease: Default::default(),
            audio_continuity: Default::default(),
            last_video: Default::default(),
            last_audio: Default::default(),
            last_key: Default::default(),
            timestamp: Default::default(),
            epoch: Default::default(),
            bins: [0; 62],
        }
    }
}
type SharedProgress = Arc<Mutex<Progress>>;

// Keep the same hard old-view checks while retaining every bounded violation.
// One receiver's loss must not hide the next original preparation path.
fn old_playout_quality_failures(
    before: &[Progress],
    during: &[Progress],
    round: usize,
    move_started: Instant,
    first_bin: usize,
    last_bin: usize,
) -> Vec<String> {
    assert_eq!(before.len(), 6);
    assert_eq!(during.len(), 6);
    assert!(last_bin <= 62, "original programme measurement bound");
    let mut failures = Vec::new();
    if first_bin >= last_bin {
        failures.push(format!(
            "round={round} preparation has no complete measured second"
        ));
    }
    for (index, current) in during.iter().enumerate() {
        let previous = &before[index];
        if current.frames <= previous.frames {
            failures.push(format!(
                "round={round} receiver={index} decoded no newer frame before={} during={}",
                previous.frames, current.frames
            ));
        }
        if current.timestamp == previous.timestamp {
            failures.push(format!(
                "round={round} receiver={index} decoded timestamp did not advance value={:?}",
                current.timestamp
            ));
        }
        if !current.last_video.is_some_and(|at| at >= move_started) {
            failures.push(format!(
                "round={round} receiver={index} lacks newly decoded preparation video"
            ));
        }
        if !current.last_audio.is_some_and(|at| at >= move_started) {
            failures.push(format!(
                "round={round} receiver={index} lacks newly decoded preparation audio"
            ));
        }
        if current.audio_samples < 2880 {
            failures.push(format!(
                "round={round} receiver={index} consecutive decoded audio samples={} required=2880",
                current.audio_samples
            ));
        }
        for bin in first_bin..last_bin {
            if current.bins[bin] < 24 {
                failures.push(format!(
                    "round={round} receiver={index} source={} preparation second={bin} decoded={} required=24 publisher_encoded={} publisher_published={}",
                    SOURCE_FOR_RECEIVER[index], current.bins[bin],
                    during[SOURCE_FOR_RECEIVER[index]].encoded_bins[bin],
                    during[SOURCE_FOR_RECEIVER[index]].published_bins[bin]
                ));
            }
        }
    }
    failures
}

fn require_old_playout_quality(failures: &[String]) {
    assert!(
        failures.is_empty(),
        "old playout quality requirements failed: {failures:?}"
    );
}

#[test]
fn old_playout_quality_retains_multiple_receiver_bins_and_still_refuses() {
    let move_started = Instant::now();
    let before = vec![
        Progress {
            frames: 10,
            timestamp: Some(3000),
            ..Progress::default()
        };
        6
    ];
    let mut current = vec![
        Progress {
            frames: 11,
            timestamp: Some(6000),
            last_video: Some(move_started),
            last_audio: Some(move_started),
            audio_samples: 2880,
            bins: [30; 62],
            encoded_bins: [30; 62],
            published_bins: [30; 62],
            ..Progress::default()
        };
        6
    ];
    let passing = old_playout_quality_failures(&before, &current, 0, move_started, 2, 5);
    require_old_playout_quality(&passing);
    current[0].frames = 10;
    current[0].timestamp = Some(3000);
    current[0].last_video = None;
    current[0].last_audio = Some(move_started - Duration::from_millis(1));
    current[0].audio_samples = 960;
    current[0].bins[2] = 23;
    current[0].bins[3] = 0;
    current[4].bins[4] = 22;
    let failures = old_playout_quality_failures(&before, &current, 0, move_started, 2, 5);
    assert_eq!(failures.len(), 8);
    for expected in [
        "receiver=0 decoded no newer frame",
        "receiver=0 decoded timestamp did not advance",
        "receiver=0 lacks newly decoded preparation video",
        "receiver=0 lacks newly decoded preparation audio",
        "receiver=0 consecutive decoded audio samples=960 required=2880",
        "receiver=0 source=3 preparation second=2 decoded=23 required=24",
        "receiver=0 source=3 preparation second=3 decoded=0 required=24",
        "receiver=4 source=5 preparation second=4 decoded=22 required=24",
    ] {
        assert!(
            failures.iter().any(|failure| failure.contains(expected)),
            "{expected}: {failures:?}"
        );
    }
    assert!(std::panic::catch_unwind(|| require_old_playout_quality(&failures)).is_err());
}

// Application decoder history, matching the normal native media producer. The
// bounded assembler orders fragments, not complete dependency-bearing frames.
struct DecodeHistory {
    assembler: VideoFrameAssembler,
    ssrc: Option<u32>,
    timestamp: Option<u32>,
    last_sequence: Option<u16>,
    waiting_key: bool,
    need_key: bool,
    last_request: Option<Instant>,
}
impl Default for DecodeHistory {
    fn default() -> Self {
        Self {
            assembler: VideoFrameAssembler::default(),
            ssrc: None,
            timestamp: None,
            last_sequence: None,
            waiting_key: true,
            need_key: false,
            last_request: None,
        }
    }
}
impl DecodeHistory {
    fn loss(&mut self, progress: &SharedProgress) {
        self.waiting_key = true;
        self.need_key = self.ssrc.is_some();
        self.last_sequence = None;
        let mut measured = progress.lock().unwrap();
        measured.video_losses += 1;
        measured.video_waiting_key = true;
    }
    fn gap(&mut self, progress: &SharedProgress) {
        self.assembler = VideoFrameAssembler::default();
        self.loss(progress);
    }
    fn take_request(&mut self, now: Instant) -> bool {
        if !self.need_key
            || self.last_request.is_some_and(|last| {
                now.saturating_duration_since(last) < Duration::from_millis(100)
            })
        {
            return false;
        }
        self.last_request = Some(now);
        true
    }
    fn push(
        &mut self,
        packet: &NativeVideoPacket,
        progress: &SharedProgress,
    ) -> Option<NativeVideoFrame> {
        if self.ssrc != Some(packet.ssrc) {
            let request = self.last_request;
            *self = Self::default();
            self.last_request = request;
            self.ssrc = Some(packet.ssrc);
            progress.lock().unwrap().video_waiting_key = true;
        }
        if self.timestamp.is_some_and(|last| {
            let distance = packet.timestamp_rtp.wrapping_sub(last);
            distance == 0 || distance >= 0x80000000
        }) {
            progress.lock().unwrap().late_video_packets += 1;
            return None;
        }
        let before = self.assembler.diagnostics();
        let frame = self.assembler.push(packet).unwrap();
        let after = self.assembler.diagnostics();
        progress.lock().unwrap().assembly = after;
        if after.discarded_frames > before.discarded_frames
            || after.expired_frames > before.expired_frames
        {
            self.loss(progress);
        }
        let frame = frame?;
        let range = self
            .assembler
            .completed_sequence_range()
            .expect("actual completed RTP range");
        {
            let mut measured = progress.lock().unwrap();
            measured.assembled_frames += 1;
            measured.assembled_keys += u64::from(frame.keyframe);
        }
        if self
            .last_sequence
            .is_some_and(|last| last.wrapping_add(1) != range.first)
        {
            self.loss(progress);
        }
        self.last_sequence = Some(range.last);
        self.timestamp = Some(frame.timestamp_rtp);
        if frame.keyframe {
            self.assembler.retire_older_partials_after_keyframe();
            self.waiting_key = false;
            self.need_key = false;
        }
        if self.waiting_key {
            self.need_key = true;
            progress.lock().unwrap().skipped_delta_frames += 1;
            return None;
        }
        {
            let mut measured = progress.lock().unwrap();
            measured.decode_attempts += 1;
            measured.previous_decode = measured.current_decode;
            measured.current_decode = Some((frame.timestamp_rtp, range));
        }
        Some(frame)
    }
}

struct LiveDecoder {
    child: tokio::process::Child,
    input: tokio::process::ChildStdin,
    output: tokio::process::ChildStdout,
    source: usize,
    ordinal: u32,
    started: bool,
}
impl LiveDecoder {
    fn open(binary: &Path, source: usize) -> Self {
        let mut child = tokio::process::Command::new(binary)
            .arg("--decode-vp8")
            .env_clear()
            .env("LANG", "C")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        eprintln!(
            "representative original decoder source={source} pid={}",
            child.id().unwrap()
        );
        Self {
            input: child.stdin.take().unwrap(),
            output: child.stdout.take().unwrap(),
            child,
            source,
            ordinal: 0,
            started: false,
        }
    }
    async fn decode(
        &mut self,
        frame: &NativeVideoFrame,
        arrived: Instant,
        cutoff: Instant,
    ) -> bool {
        if !self.started && !frame.keyframe {
            return false;
        }
        let until = cutoff.min(arrived + AGE);
        assert!(
            Instant::now() < until,
            "original assembled frame aged out before actual decode"
        );
        self.ordinal += 1;
        let mut request = b"D8F1".to_vec();
        request.extend(self.ordinal.to_le_bytes());
        request.extend((frame.encoded.len() as u32).to_le_bytes());
        request.extend(u32::from(frame.keyframe).to_le_bytes());
        request.extend(&frame.encoded);
        let mut response = [0u8; 32];
        timeout_at(until, async {
            self.input.write_all(&request).await.unwrap();
            self.output.read_exact(&mut response).await.unwrap();
        })
        .await
        .expect("original live VP8 output exceeded original frame age");
        let word = |at| u32::from_le_bytes(response[at..at + 4].try_into().unwrap());
        assert_eq!(&response[..4], b"V8D1");
        assert_eq!(
            (word(4), word(8), word(12)),
            (self.ordinal, 1, u32::from(frame.keyframe))
        );
        assert_eq!((word(16), word(20), word(28)), (640, 360, 256));
        assert!(
            (i64::from(word(24)) - i64::from(marker(self.source)) * 256).abs() <= 3 * 256,
            "decoded pixels belong to a different original source"
        );
        self.started = true;
        true
    }
    async fn close(mut self) -> bool {
        let pid = self.child.id();
        let until = Instant::now() + Duration::from_secs(2);
        let result = timeout_at(until, async {
            let input = self.input.shutdown().await;
            drop(self.input);
            let mut trailing = [0u8; 1];
            let output = self.output.read(&mut trailing).await;
            let status = self.child.wait().await;
            (input, output, status)
        })
        .await;
        if let Ok((input, output, Ok(status))) = result {
            let clean = input.is_ok() && matches!(output, Ok(0)) && status.success();
            eprintln!("representative original decoder joined source={} pid={pid:?} outputs={} forced=false exit_code={:?} clean={clean}",self.source,self.ordinal,status.code());
            return clean;
        }
        if let Ok(Some(status)) = self.child.try_wait() {
            eprintln!("representative original decoder joined source={} pid={pid:?} outputs={} forced=false exit_code={:?} clean=false",self.source,self.ordinal,status.code());
            return false;
        }
        let killed = self.child.start_kill();
        let joined = self.child.wait().await;
        eprintln!("representative original decoder joined source={} pid={pid:?} forced=true kill={killed:?} wait={joined:?}",self.source);
        false
    }
}

/// Reuse the production fixture executable, not canned RTP. Sparse immutable
/// I420 files keep this twenty-six-second regression below 192MiB physical input.
/// Each source has changing textured pixels and a distinct 16x16 decoder mark.
async fn codec_inputs(directory: &Path) -> (std::path::PathBuf, Vec<std::path::PathBuf>) {
    use std::io::{Seek, SeekFrom, Write};
    let binary = directory.join("representative-live-vp8");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let built = tokio::process::Command::new("python3")
        .arg(root.join("scripts/video/build_live_vp8_encoder.py"))
        .arg("--output")
        .arg(&binary)
        .output()
        .await
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let mut paths = Vec::new();
    for source in 0..6 {
        let path = directory.join(format!("representative-source-{source}.i420"));
        init::private_write(&path, &[]).unwrap();
        let mut file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(u64::from(FRAMES) * 640 * 360 * 3 / 2).unwrap();
        for index in 0..FRAMES {
            let mut texture = vec![0u8; 640 * 64];
            let mut seed = 0x51f15eadu32 ^ (source as u32 * 7919) ^ index.wrapping_mul(104729);
            for pixel in &mut texture {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                *pixel = 32 + (seed % 192) as u8;
            }
            for y in 0..32 {
                texture[y * 640..y * 640 + 32].fill(marker(source) as u8);
            }
            file.seek(SeekFrom::Start(u64::from(index) * 640 * 360 * 3 / 2))
                .unwrap();
            file.write_all(&texture).unwrap();
        }
        file.sync_all().unwrap();
        paths.push(path);
    }
    (binary, paths)
}
fn encoder(binary: &Path, raw: &Path) -> Encoder {
    let mut child = tokio::process::Command::new(binary)
        .arg(raw)
        .arg(FRAMES.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    Encoder {
        executable: binary.into(),
        executable_sha256: file_sha256(binary),
        input: child.stdin.take().unwrap(),
        output: child.stdout.take().unwrap(),
        child,
    }
}

async fn finish_encoder(encoder: Encoder) -> bool {
    let Encoder {
        mut child,
        mut input,
        output,
        ..
    } = encoder;
    drop(output);
    let pid = child.id();
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        input.shutdown().await?;
        drop(input);
        Ok::<_, std::io::Error>(child.wait().await?.success())
    })
    .await;
    if let Ok(Ok(true)) = result {
        eprintln!("representative original encoder joined pid={pid:?} forced=false");
        return true;
    }
    let killed = child.start_kill();
    let joined = child.wait().await;
    eprintln!("representative original encoder joined pid={pid:?} forced=true kill={killed:?} wait={joined:?}");
    false
}
// One terminal refusal can be ordinary retirement only after the exact actual
// original commit is recorded. This never grants packet admission, pauses an
// old owner or accepts an Unsupported result merely because Ready was built.
#[derive(Clone)]
struct RetiredMove {
    record: String,
    reservation: String,
    candidate: String,
    runtime: String,
    generation: u64,
    observed_commit: Arc<AtomicBool>,
}
struct RetirementWitness {
    database: std::path::PathBuf,
    source: String,
    original: Mutex<Option<RetiredMove>>,
}
impl RetirementWitness {
    // Packet/timer loops consult only the already-observed historical fact.
    // They perform no database IO and cannot forecast a later commit.
    fn observed(&self) -> bool {
        self.original
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|original| original.observed_commit.load(Ordering::Acquire))
    }
    fn committed(&self) -> bool {
        let Some(original) = self.original.lock().unwrap().clone() else {
            return false;
        };
        if original.observed_commit.load(Ordering::Acquire) {
            return true;
        }
        let db = rusqlite::Connection::open_with_flags(
            &self.database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let committed = db.query_row("SELECT EXISTS(SELECT 1 FROM native_replacements n
            JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='commit' AND s.started=1
            JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=n.operation_record_id AND e.provider='native-replacement-commit' AND e.state='confirmed'
            JOIN connections old ON old.id=?1 AND old.state='retired' AND old.generation=?6
            JOIN connections target ON target.id=n.candidate_connection_id AND target.replaces=old.id AND target.state='ready' AND target.generation=?6
            JOIN connection_offers media ON media.connection_id=target.id AND media.runtime_connection_id=?5
            WHERE n.operation_record_id=?2 AND n.reservation_id=?3 AND n.candidate_connection_id=?4 AND n.candidate_runtime_id=?5 AND n.generation=?6)",
            rusqlite::params![self.source,original.record,original.reservation,original.candidate,original.runtime,original.generation],|row|row.get(0)).unwrap();
        if committed {
            // This exact confirmed commit remains historical fact when its
            // target subsequently moves again. It grants no media authority.
            original.observed_commit.store(true, Ordering::Release);
        }
        committed
    }
}
fn capture_retirement(
    f: &Fixture,
    command: &TransitionCommand,
    root: &connect_client::canonical::payloads::OperationUpdated,
    generation: u64,
    cutoff: Instant,
) -> RetiredMove {
    let db = rusqlite::Connection::open_with_flags(
        &f.config.database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let (record,reservation,candidate,runtime,actual_generation,expires_at,source):
        (String,String,String,String,u64,i64,String)=db.query_row(
        "SELECT n.operation_record_id,n.reservation_id,n.candidate_connection_id,n.candidate_runtime_id,n.generation,n.expires_at,o.connection_id FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id WHERE n.operation_record_id=?1 AND o.operation_id=?2",
        rusqlite::params![root.record_id.as_ref().unwrap().as_str(),command.operation_id],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).unwrap();
    assert_eq!(actual_generation, generation);
    assert_eq!(Some(&source), command.connection_id.as_ref());
    let wall = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    let remaining = Duration::from_secs(u64::try_from(expires_at).unwrap())
        .checked_sub(wall)
        .expect("original return preparation already expired");
    assert!(
        cutoff <= Instant::now() + remaining,
        "movement cannot renew original reservation"
    );
    RetiredMove {
        record,
        reservation,
        candidate,
        runtime,
        generation,
        observed_commit: Arc::new(AtomicBool::new(false)),
    }
}

fn retirement_control() -> (tempfile::TempDir, RetirementWitness, rusqlite::Connection) {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("retirement.sqlite3");
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection.execute_batch("
        CREATE TABLE native_replacements(operation_record_id TEXT, reservation_id TEXT,
            candidate_connection_id TEXT, candidate_runtime_id TEXT, generation INTEGER);
        CREATE TABLE native_replacement_steps(operation_record_id TEXT, phase TEXT,
            started INTEGER, effect_id TEXT);
        CREATE TABLE operation_effects(id TEXT, operation_record_id TEXT, provider TEXT, state TEXT);
        CREATE TABLE connections(id TEXT, state TEXT, generation INTEGER, replaces TEXT);
        CREATE TABLE connection_offers(connection_id TEXT, runtime_connection_id TEXT);
        INSERT INTO native_replacements VALUES('record','reservation','target','runtime',2);
        INSERT INTO native_replacement_steps VALUES('record','commit',1,'effect');
        INSERT INTO operation_effects VALUES('effect','record','native-replacement-commit','confirmed');
        INSERT INTO connections VALUES('source','retired',2,NULL),('target','ready',2,'source');
        INSERT INTO connection_offers VALUES('target','runtime');
    ").unwrap();
    let witness = RetirementWitness {
        database,
        source: "source".into(),
        original: Mutex::new(Some(RetiredMove {
            record: "record".into(),
            reservation: "reservation".into(),
            candidate: "target".into(),
            runtime: "runtime".into(),
            generation: 2,
            observed_commit: Arc::new(AtomicBool::new(false)),
        })),
    };
    (directory, witness, connection)
}

#[test]
fn candidate_retirement_poll_uses_only_observed_exact_commit_and_preserves_history() {
    let (_directory, witness, connection) = retirement_control();
    // Polling must not touch SQLite even when the return has been armed.
    let parked = witness.database.with_extension("parked");
    std::fs::rename(&witness.database, &parked).unwrap();
    assert!(!witness.observed());
    assert!(!witness.database.exists());
    std::fs::rename(&parked, &witness.database).unwrap();
    assert!(witness.committed());
    assert!(witness.observed());
    // A subsequent genuine return retires this target at generation three.
    // It cannot erase the original commit already observed for this source.
    connection
        .execute(
            "UPDATE connections SET state='retired',generation=3 WHERE id='target'",
            [],
        )
        .unwrap();
    std::fs::rename(&witness.database, &parked).unwrap();
    assert!(witness.observed());
    assert!(witness.committed());
    assert!(!witness.database.exists());
}

#[test]
fn original_readonly_deadline_requires_exact_confirmed_retirement() {
    let readonly_deadline = || {
        Err(sdk::Error {
            kind: sdk::ErrorKind::Deadline,
            certainty: sdk::Certainty::ReadOnly,
            operation: None,
            problem: None,
            observation: None,
        })
    };
    for invalid in [
        "UPDATE operation_effects SET state='unknown'",
        "UPDATE native_replacement_steps SET started=0",
        "UPDATE native_replacements SET reservation_id='other'",
        "UPDATE native_replacements SET candidate_runtime_id='other'",
        "UPDATE connections SET state='ready' WHERE id='source'",
        "UPDATE connections SET generation=1 WHERE id='source'",
        "UPDATE connection_offers SET runtime_connection_id='other'",
    ] {
        let (_directory, witness, connection) = retirement_control();
        connection.execute(invalid, []).unwrap();
        assert!(std::panic::catch_unwind(|| original_packet(
            readonly_deadline(),
            &witness,
            "video"
        ))
        .is_err());
        assert!(!witness.observed());
    }
    let (_directory, witness, _connection) = retirement_control();
    assert!(original_packet(readonly_deadline(), &witness, "video").is_none());
    // Mutating/uncertain timeouts and unrelated receive failures still fail.
    for (kind, certainty) in [
        (sdk::ErrorKind::Deadline, sdk::Certainty::Uncertain),
        (sdk::ErrorKind::Protocol, sdk::Certainty::ReadOnly),
    ] {
        let result = Err(sdk::Error {
            kind,
            certainty,
            operation: None,
            problem: None,
            observation: None,
        });
        assert!(std::panic::catch_unwind(|| original_packet(result, &witness, "video")).is_err());
    }
}

fn original_packet(
    result: sdk::Result<sdk::MediaPacket>,
    retirement: &RetirementWitness,
    medium: &'static str,
) -> Option<sdk::MediaPacket> {
    match result {
        Ok(packet) => Some(packet),
        Err(error)
            if (error.kind == sdk::ErrorKind::Unsupported
                || (error.kind == sdk::ErrorKind::Deadline
                    && error.certainty == sdk::Certainty::ReadOnly))
                && retirement.committed() =>
        {
            eprintln!("representative original receiver observed exact recorded source retirement source={}",retirement.source);
            None
        }
        Err(error) => panic!("original live {medium} receiver failed: {error:?}"),
    }
}
async fn send_original(
    stream: &sdk::Stream,
    packet: sdk::RtpPacket,
    retirement: &RetirementWitness,
    until: Instant,
) -> bool {
    match stream.send_rtp(packet, programme::request(until)).await {
        Ok(_) => true,
        Err(error) if error.kind == sdk::ErrorKind::Unsupported && retirement.committed() => {
            eprintln!("representative original publisher observed exact recorded source retirement source={}",retirement.source);
            false
        }
        Err(error) => panic!("original live publisher failed: {error:?}"),
    }
}

fn record_frame_bin(bins: &mut [u16; 62], at: Instant, began: Instant) {
    let bin = at.saturating_duration_since(began).as_secs() as usize;
    if let Some(count) = bins.get_mut(bin) {
        *count = count.saturating_add(1);
    }
}

#[test]
fn frame_input_bins_keep_original_second_boundaries_and_programme_limit() {
    let began = Instant::now();
    let mut bins = [0; 62];
    for elapsed in [
        Duration::ZERO,
        Duration::from_millis(999),
        Duration::from_secs(1),
        Duration::from_secs(61),
        Duration::from_secs(62),
    ] {
        record_frame_bin(&mut bins, began + elapsed, began);
    }
    assert_eq!(bins[0], 2);
    assert_eq!(bins[1], 1);
    assert_eq!(bins[61], 1);
    assert_eq!(bins.iter().sum::<u16>(), 4);
    bins[2] = u16::MAX;
    record_frame_bin(&mut bins, began + Duration::from_secs(2), began);
    assert_eq!(bins[2], u16::MAX);
}

fn observe_video(
    progress: &SharedProgress,
    frame: &NativeVideoFrame,
    epoch: sdk::VideoReceiveEpoch,
    arrived: Instant,
    began: Instant,
) {
    let mut state = progress.lock().unwrap();
    state.frames += 1;
    if frame.keyframe && state.video_waiting_key {
        state.recovered_keys += 1;
    }
    state.video_waiting_key = false;
    if frame.keyframe {
        state.keys += 1;
        state.last_key = Some(arrived);
        if frame.encoded.len() > 2000 {
            state.fragmented_keys += 1;
        }
    }
    state.last_video = Some(arrived);
    state.timestamp = Some(frame.timestamp_rtp);
    state.epoch = Some(epoch);
    record_frame_bin(&mut state.bins, arrived, began);
    if state.public && state.public_since.is_some_and(|at| arrived >= at) {
        record_frame_bin(&mut state.public_bins, arrived, began);
    }
}
fn observe_audio(progress: &SharedProgress, decoder: &readiness_audio::Decoder, decoded: bool) {
    if !decoded {
        return;
    }
    let mut state = progress.lock().unwrap();
    let samples = decoder.samples();
    let now = Instant::now();
    if state.audio_samples > 0 && samples <= state.audio_samples {
        state.audio_run_nonincreases += 1;
        state.audio_last_nonincrease = Some(now);
    }
    state.audio_samples = samples;
    state.audio_peak_samples = state.audio_peak_samples.max(samples);
    state.last_audio = Some(now);
    if let Some(observation) = decoder.continuity() {
        state.audio_continuity.observe(observation, now);
    }
}

#[test]
fn ignored_audio_packets_cannot_refresh_actual_decoded_playout() {
    let progress = Arc::new(Mutex::new(Progress::default()));
    let mut decoder = selected_opus();
    let bytes = decoder.encoded_silence().unwrap();
    for sequence in 0..3u16 {
        let decoded = decoder
            .decode(7, sequence, u32::from(sequence) * 960, &bytes)
            .unwrap();
        assert!(decoded);
        observe_audio(&progress, &decoder, decoded);
    }
    let accepted = decoder.continuity();
    let expired = Instant::now() - AGE;
    progress.lock().unwrap().last_audio = Some(expired);
    for sequence in [2u16, 1] {
        let decoded = decoder
            .decode(7, sequence, u32::from(sequence) * 960, &bytes)
            .unwrap();
        assert!(!decoded);
        observe_audio(&progress, &decoder, decoded);
        let observed = progress.lock().unwrap();
        assert_eq!(observed.audio_samples, 2880);
        assert_eq!(observed.last_audio, Some(expired));
        assert_eq!(observed.audio_run_nonincreases, 0);
        assert_eq!(decoder.continuity(), accepted);
        assert_eq!(observed.audio_continuity.last, accepted);
        assert_eq!(observed.audio_continuity.resets, 0);
    }
    assert!(decoder.decode(7, 3, 2880, &[]).is_err());
    assert_eq!(decoder.continuity(), accepted);
    assert_eq!(progress.lock().unwrap().last_audio, Some(expired));
    let decoded = decoder.decode(7, 3, 2880, &bytes).unwrap();
    assert!(decoded);
    observe_audio(&progress, &decoder, decoded);
    assert!(progress.lock().unwrap().last_audio.unwrap() > expired);
    let decoded = decoder.decode(7, 5, 4800, &bytes).unwrap();
    assert!(decoded);
    observe_audio(&progress, &decoder, decoded);
    let observed = progress.lock().unwrap();
    assert_eq!(observed.audio_samples, 960);
    assert_eq!(observed.audio_peak_samples, 3840);
    assert_eq!(observed.audio_run_nonincreases, 1);
    assert_eq!(observed.audio_continuity.resets, 1);
    let reset = observed.audio_continuity.last.unwrap().reset;
    assert!(!reset.ssrc_changed);
    assert!(reset.sequence_discontinuity && reset.timestamp_discontinuity);
    drop(observed);
    // A real codec error after the discontinuity predicate must not publish
    // a fabricated successful output or continuity observation.
    let mut g711 = readiness_audio::Decoder::selected(&json!({"name":"pcmu","params":{
        "sample_rate":8000,"channels":1,"payload_type":0}}))
    .unwrap();
    assert!(g711.decode(9, 1, 160, &[0xff; 160]).unwrap());
    let accepted = g711.continuity();
    assert!(g711.decode(9, 3, 480, &[0xff; 159]).is_err());
    assert_eq!(g711.continuity(), accepted);
}

#[test]
fn actual_audio_rtp_resets_report_exact_ssrc_sequence_and_timestamp_causes() {
    let cases = [
        (
            (8, 101, 10560),
            readiness_audio::ResetReasons {
                ssrc_changed: true,
                sequence_discontinuity: false,
                timestamp_discontinuity: false,
            },
        ),
        (
            (7, 102, 10560),
            readiness_audio::ResetReasons {
                ssrc_changed: false,
                sequence_discontinuity: true,
                timestamp_discontinuity: false,
            },
        ),
        (
            (7, 101, 11520),
            readiness_audio::ResetReasons {
                ssrc_changed: false,
                sequence_discontinuity: false,
                timestamp_discontinuity: true,
            },
        ),
        (
            (7, 102, 11520),
            readiness_audio::ResetReasons {
                ssrc_changed: false,
                sequence_discontinuity: true,
                timestamp_discontinuity: true,
            },
        ),
    ];
    for ((ssrc, sequence, timestamp), expected) in cases {
        let progress = Arc::new(Mutex::new(Progress::default()));
        let mut decoder = selected_opus();
        let bytes = decoder.encoded_silence().unwrap();
        let began = Instant::now();
        assert!(decoder.decode(7, 100, 9600, &bytes).unwrap());
        observe_audio(&progress, &decoder, true);
        assert!(decoder.decode(ssrc, sequence, timestamp, &bytes).unwrap());
        observe_audio(&progress, &decoder, true);
        assert_eq!(decoder.samples(), 960);
        assert!(!decoder.usable());
        let observed = progress.lock().unwrap();
        let continuity = &observed.audio_continuity;
        assert_eq!(continuity.resets, 1);
        assert_eq!(continuity.len, 1);
        let report = continuity.reports(began)[0].unwrap();
        assert_eq!(
            report.previous,
            readiness_audio::RtpHeader {
                ssrc: 7,
                sequence: 100,
                timestamp: 9600
            }
        );
        assert_eq!(
            report.current,
            readiness_audio::RtpHeader {
                ssrc,
                sequence,
                timestamp
            }
        );
        assert_eq!(report.reasons, expected);
        assert!(report.elapsed_us.is_some());
        assert_eq!(continuity.ssrc_changes, u64::from(expected.ssrc_changed));
        assert_eq!(
            continuity.sequence_discontinuities,
            u64::from(expected.sequence_discontinuity)
        );
        assert_eq!(
            continuity.timestamp_discontinuities,
            u64::from(expected.timestamp_discontinuity)
        );
    }
}

#[test]
fn actual_audio_rtp_continuity_wrap_keeps_required_pcm_run() {
    let progress = Arc::new(Mutex::new(Progress::default()));
    let mut decoder = selected_opus();
    let bytes = decoder.encoded_silence().unwrap();
    for (sequence, timestamp) in [(u16::MAX, u32::MAX - 959), (0, 0), (1, 960)] {
        let decoded = decoder.decode(7, sequence, timestamp, &bytes).unwrap();
        assert!(decoded);
        observe_audio(&progress, &decoder, decoded);
    }
    assert!(decoder.usable());
    assert_eq!(decoder.samples(), 2880);
    let observed = progress.lock().unwrap();
    assert_eq!(observed.audio_samples, 2880);
    assert_eq!(observed.audio_continuity.resets, 0);
    let last = observed.audio_continuity.last.unwrap();
    assert_eq!(
        last.previous,
        Some(readiness_audio::RtpHeader {
            ssrc: 7,
            sequence: 0,
            timestamp: 0
        })
    );
    assert_eq!(
        last.current,
        readiness_audio::RtpHeader {
            ssrc: 7,
            sequence: 1,
            timestamp: 960
        }
    );
    assert!(!last.reset.any());
}

#[test]
fn actual_audio_reset_history_is_fixed_and_counts_saturate_without_duplicate_credit() {
    let progress = Arc::new(Mutex::new(Progress::default()));
    let mut decoder = selected_opus();
    let bytes = decoder.encoded_silence().unwrap();
    let began = Instant::now();
    for index in 0..=12u16 {
        let decoded = decoder
            .decode(7 + u32::from(index), index, u32::from(index) * 960, &bytes)
            .unwrap();
        assert!(decoded);
        observe_audio(&progress, &decoder, decoded);
    }
    {
        let mut observed = progress.lock().unwrap();
        let continuity = &mut observed.audio_continuity;
        assert_eq!(continuity.resets, 12);
        assert_eq!(continuity.ssrc_changes, 12);
        assert_eq!(continuity.sequence_discontinuities, 0);
        assert_eq!(continuity.timestamp_discontinuities, 0);
        assert_eq!(continuity.len, AUDIO_RESET_HISTORY);
        let reports = continuity.reports(began);
        assert_eq!(reports[0].unwrap().current.ssrc, 12);
        assert_eq!(reports[7].unwrap().current.ssrc, 19);
        continuity.observe(
            decoder.continuity().unwrap(),
            Instant::now() + Duration::from_secs(1),
        );
        assert_eq!(continuity.resets, 12);
        assert_eq!(continuity.reports(began), reports);
        continuity.resets = u64::MAX;
        continuity.ssrc_changes = u64::MAX;
    }
    let decoded = decoder.decode(20, 13, 12480, &bytes).unwrap();
    assert!(decoded);
    observe_audio(&progress, &decoder, decoded);
    let observed = progress.lock().unwrap();
    let continuity = &observed.audio_continuity;
    assert_eq!(continuity.resets, u64::MAX);
    assert_eq!(continuity.ssrc_changes, u64::MAX);
    assert_eq!(continuity.len, AUDIO_RESET_HISTORY);
    assert_eq!(continuity.reports(began)[7].unwrap().current.ssrc, 20);
    assert_eq!(observed.audio_samples, 960);
}

fn selected_opus() -> readiness_audio::Decoder {
    readiness_audio::Decoder::selected(&json!({"name":"opus","params":{
        "sample_rate":48000,"channels":1,"payload_type":111}}))
    .unwrap()
}
fn spawn_publisher(
    connection: &sdk::Connection,
    source: usize,
    master: &programme::Source,
    progress: SharedProgress,
    stop: CancellationToken,
    retirement: Arc<RetirementWitness>,
    began: Instant,
    cutoff: Instant,
) -> Vec<Work> {
    let video = connection.stream_by_id(format!("representative-video-{source}").parse().unwrap());
    let voice = connection.stream_by_id(format!("representative-audio-{source}").parse().unwrap());
    let mut samples = master.subscribe();
    let requested = master.force_key.clone();
    vec![tokio::spawn(async move {
        AssertUnwindSafe(async {
            let mut sequence = 0u32;
            loop {
                if retirement.observed() { break; }
                tokio::select! {
                    biased;
                    _ = stop.cancelled() => break,
                    request = video.receive_video_feedback(short()) => match request {
                        Ok(request) => {
                            progress.lock().unwrap().publisher_feedback_received += 1;
                            if request.ssrc() == 0x74000000 + source as u32 && video.video_feedback_is_current(&request).unwrap() {
                                requested.store(true, Ordering::Release);
                                progress.lock().unwrap().publisher_feedback_used += 1;
                            }
                        }
                        Err(error) if error.kind == sdk::ErrorKind::Deadline && error.certainty == sdk::Certainty::ReadOnly => {}
                        Err(error) if error.kind == sdk::ErrorKind::Unsupported && retirement.committed() => break,
                        Err(error) => panic!("original publisher feedback failed: {error:?}"),
                    },
                    sample = samples.recv() => {
                        let sample = sample.expect("original programme master output must not overflow or disappear");
                        let until = cutoff.min(sample.captured + AGE);
                        assert!(Instant::now() < until, "actual original source sample age");
                        if sample.video {
                            let parts = sample.bytes.chunks(1000).collect::<Vec<_>>();
                            for (part, bytes) in parts.iter().enumerate() {
                                let mut body = vec![if part == 0 {0x10} else {0}];body.extend_from_slice(bytes);
                                let packet = programme::video_packet(sequence,sample.index * 3000,
                                    0x74000000 + source as u32,part + 1 == parts.len(),&body);
                                if !timeout_at(until,send_original(&video,packet,&retirement,until)).await.unwrap() {return;}
                                sequence = sequence.checked_add(1).expect("original native video datagram counter");
                                progress.lock().unwrap().published_video_packets += 1;
                            }
                            let mut measured = progress.lock().unwrap();
                            measured.published_frames += 1;
                            record_frame_bin(&mut measured.published_bins,Instant::now(),began);
                        } else {
                            let sequence = sample.index as u16;
                            let packet = sdk::RtpPacket::new(sample.index,rtp(111,sequence,sample.index*960,
                                0x61000000+source as u32,false,&sample.bytes)).unwrap();
                            if !timeout_at(until,send_original(&voice,packet,&retirement,until)).await.unwrap() {return;}
                            progress.lock().unwrap().published_audio_packets += 1;
                        }
                    }
                }
            }
        }).catch_unwind().await.map_err(panic_text)
    })]
}

fn spawn_original_receiver(
    connection: &sdk::Connection,
    index: usize,
    source: usize,
    binary: &Path,
    progress: SharedProgress,
    stop: CancellationToken,
    retirement: Arc<RetirementWitness>,
    began: Instant,
    cutoff: Instant,
) -> Work {
    spawn_original_receiver_with_read_cutoff(
        connection, index, source, binary, progress, stop, retirement, began, cutoff, None,
    )
}

// Only the isolated intentional-authority-exclusion fixture supplies a fixed
// original read cutoff. Normal/full-programme receivers keep short() exactly.
fn spawn_original_receiver_with_read_cutoff(
    connection: &sdk::Connection,
    index: usize,
    source: usize,
    binary: &Path,
    progress: SharedProgress,
    stop: CancellationToken,
    retirement: Arc<RetirementWitness>,
    began: Instant,
    cutoff: Instant,
    read_cutoff: Option<Instant>,
) -> Work {
    let connection = connection.clone();
    let mut decoder = LiveDecoder::open(binary, source);
    tokio::spawn(async move {
        let result=AssertUnwindSafe(async {
            let video_id:sdk::StreamId=format!("representative-video-{source}").parse().unwrap();
            let audio_id:sdk::StreamId=format!("representative-audio-{index}").parse().unwrap();
            let selected=connection.native_negotiation().unwrap().expect("original adopted native parameters");
            assert!(selected.streams.iter().any(|stream|stream.id==video_id&&stream.medium==Medium::Video
                &&stream.direction==Direction::Recvonly&&stream.stream_local_id.is_some_and(|id|id>0)),
                "selected remote video must already own its native receive binding");
            assert_eq!(selected.video_feedback.get(&video_id).expect("selected original remote video feedback").connection_generation,1);
            assert!(selected.streams.iter().any(|stream|stream.id==audio_id&&stream.medium==Medium::Audio
                &&stream.direction==Direction::Sendrecv&&stream.stream_local_id.is_some_and(|id|id>0)),
                "own duplex audio must keep its native receive binding");
            let video=connection.stream_by_id(video_id);
            let voice=connection.stream_by_id(audio_id);
            let mut continuity=DecodeHistory::default();
            let mut audio=selected_opus();let mut epoch=None;
            // During deliberate exclusion derive remaining time from the SAME
            // absolute cutoff. Once elapsed, keep normal short reads for owner
            // cleanup; the caller's fault/SDK/decoded assertions still fail at
            // the original cutoff and never gain a new acceptance window.
            let read_options=||read_cutoff
                .and_then(|until|until.checked_duration_since(Instant::now()))
                .filter(|remaining|!remaining.is_zero())
                .map_or_else(short,|remaining|RequestOptions::new(remaining).unwrap());
            loop {
                if retirement.observed() {break;}
                tokio::select! {
                    _=stop.cancelled()=>break,
                    packet=video.receive_packet(read_options())=>{
                        let Some(packet)=original_packet(packet,&retirement,"video") else {break};
                        progress.lock().unwrap().video_read_events+=1;
                        let arrived=Instant::now();
                        let frame=match packet {
                            sdk::MediaPacket::Rtp(packet)=>{
                                progress.lock().unwrap().video_packets+=1;
                                let history=packet.video_epoch().expect("original signed receiver epoch").clone();
                                if epoch.as_ref()!=Some(&history) {
                                    continuity=DecodeHistory::default();decoder.started=false;epoch=Some(history);
                                    progress.lock().unwrap().video_waiting_key=true;
                                }
                                let parsed=NativeVideoPacket::parse(packet.bytes().to_vec()).unwrap();
                                continuity.push(&parsed,&progress)
                            }
                            sdk::MediaPacket::Gap{..}=>{continuity.gap(&progress);None}
                            _=>None,
                        };
                        if continuity.take_request(Instant::now()) {
                            match video.request_keyframe(short()).await {
                                Ok(_) => progress.lock().unwrap().feedback_requests += 1,
                                Err(error) if error.kind == sdk::ErrorKind::Unsupported
                                    && retirement.committed() => break,
                                Err(error) => panic!("original current keyframe request failed: {error:?}"),
                            }
                        }
                        if let Some(frame)=frame {
                            if decoder.decode(&frame,arrived,cutoff).await {
                                observe_video(&progress,&frame,epoch.as_ref().unwrap().clone(),arrived,began);
                            }
                        }
                    },
                    packet=voice.receive_packet(read_options())=>{
                        let Some(packet)=original_packet(packet,&retirement,"audio") else {break};
                        progress.lock().unwrap().audio_read_events+=1;
                        let sdk::MediaPacket::Rtp(packet)=packet else {continue};
                        progress.lock().unwrap().audio_packets+=1;
                        let header=packet.header();assert_eq!(header.payload_type,111);
                        let decoded=audio.decode(header.ssrc,header.sequence_number,header.timestamp,packet.payload()).unwrap();
                        observe_audio(&progress,&audio,decoded);
                    }
                }
            }
        }).catch_unwind().await.map_err(panic_text);
        if result.is_err() {
            eprintln!("representative original receiver failed route={index} expected_source={source} progress={:?}",progress.lock().unwrap());
        }
        let joined = decoder.close().await;
        result.and(if joined {
            Ok(())
        } else {
            Err("original receiver decoder did not finish successfully".into())
        })
    })
}

struct Candidate {
    client: CanonicalClient,
    io: Option<NativeVideoDatagrams>,
    preparation: Option<NativeReplacementReadiness>,
    voice: u16,
    camera: u16,
    incoming: u16,
    force_key: Arc<AtomicBool>,
    retirement: Option<Arc<RetirementWitness>>,
}
async fn attach_candidate(
    target: &mut Candidate,
    command: &TransitionCommand,
    root: &connect_client::canonical::payloads::OperationUpdated,
    index: usize,
    phase: &str,
    source_generation: u64,
    database: &Path,
    cutoff: Instant,
) {
    timeout_at(cutoff,async {
        let client=&mut target.client;
        let offer=json!({"substrate":"quic","capabilities":{
            "audio_codecs":[{"name":"opus","params":{"sample_rate":48000,"channels":1}}],
            "video_codecs":[{"name":"vp8","params":{"sample_rate":90000,"channels":0}}],
            "data_protocols":["binary"],"max_streams_per_connection":8,"video_feedback":"rtcp-pli-epoch/1"},
            "streams_offered":[
                {"id":format!("representative-{phase}-data-{index}"),"kind":"data","direction":"sendrecv","codec_preferences":["binary"]},
                {"id":format!("representative-{phase}-audio-{index}"),"kind":"audio","direction":"sendrecv","codec_preferences":["opus"]},
                {"id":format!("representative-{phase}-video-{index}"),"kind":"video","direction":"sendonly","codec_preferences":["vp8"]},
                {"id":format!("representative-{phase}-incoming-{index}"),"kind":"video","direction":"recvonly","codec_preferences":["vp8"]}]});
        client.allocate_native_replacement_target(command,root,serde_json::from_value(offer.clone()).unwrap()).await.unwrap();
        let (_,scope)=client.begin_native_replacement_media(command,root,serde_json::from_value(offer).unwrap()).await.unwrap();
        assert_eq!(client.receive().await.unwrap().kind(),"connection.answer");
        let callback=client.receive().await.unwrap();assert_eq!(callback.kind(),"connection.offer");
        let bindings=callback.original()["payload"]["substrate_setup"]["streams"].as_array().unwrap();
        let handle=|name:&str|bindings.iter().find(|s|s["strm_id"]==name).unwrap()["stream_local_id"].as_u64().unwrap() as u16;
        target.voice=handle(&format!("representative-{phase}-audio-{index}"));
        target.camera=handle(&format!("representative-{phase}-video-{index}"));
        target.incoming=handle(&format!("representative-{phase}-incoming-{index}"));
        let answer=client.native_replacement_answer(&callback).unwrap();
        client.answer_connection_offer(&callback,answer,&scope).await.unwrap();
        let preparation=client.native_replacement_readiness().unwrap();
        assert_eq!((preparation.record_id(),preparation.source_generation(),preparation.target_generation()),
            (root.record_id.as_ref().unwrap().as_str(),source_generation,source_generation+1));
        assert_eq!(preparation.audio().stream_local_id,target.voice);
        assert_eq!(preparation.video().unwrap().stream_local_id,target.incoming);
        let mut io=client.claim_native_video_datagrams().unwrap();
        io.reset_video_receive_decoder(target.incoming).unwrap();
        target.retirement=Some(Arc::new(RetirementWitness{database:database.into(),
            source:preparation.connection_id().unwrap().into(),original:Mutex::new(None)}));
        target.preparation=Some(preparation);target.io=Some(io);
    }).await.expect("same original eight-second target negotiation deadline");
}

fn spawn_candidate(
    candidate: &mut Candidate,
    source: usize,
    inbound: usize,
    binary: &Path,
    master: &programme::Source,
    progress: SharedProgress,
    stop: CancellationToken,
    began: Instant,
    cutoff: Instant,
) -> Work {
    use rvoip_uctp::substrate::{
        observe_native_media_datagram, pack, MediaDatagram, ObservedNativeMediaDatagram,
    };
    let force_key = candidate.force_key.clone();
    let retirement = candidate.retirement.as_ref().unwrap().clone();
    let mut io = candidate.io.take().unwrap();
    let (voice, camera, incoming) = (candidate.voice, candidate.camera, candidate.incoming);
    let mut samples = master.subscribe();
    let requested = master.force_key.clone();
    let source_end = began + Duration::from_secs(programme::SECONDS as u64);
    let mut decoder = LiveDecoder::open(binary, inbound);
    tokio::spawn(async move {
        let result=AssertUnwindSafe(async {
            let mut audio=selected_opus();let mut continuity=DecodeHistory::default();
            let mut epoch=None;let mut sequence=0u32;
            'live: loop {
                // This original physical may stop only after its own exact
                // later replacement really committed. No pre-Ready forecast
                // or renewed deadline can retire a generation-two candidate.
                if retirement.observed() {break;}
                tokio::select! {
                    biased;
                    _=stop.cancelled()=>break,
                    sample=samples.recv()=>{
                        let sample=sample.expect("same original source master cannot overflow");
                        let until=cutoff.min(source_end).min(sample.captured+AGE);
                        assert!(Instant::now()<until,"actual candidate source sample age and original62s publication cutoff");
                        if force_key.swap(false,Ordering::AcqRel)
                            || io.take_keyframe_request(camera,0x74000000+source as u32).unwrap() {
                            requested.store(true,Ordering::Release);
                        }
                        if sample.video {
                            for (part,bytes) in sample.bytes.chunks(1000).enumerate() {
                                let mut body=vec![if part==0 {0x10} else {0}];body.extend_from_slice(bytes);
                                assert!(Instant::now()<until,"original source fragment publication cutoff");
                                let datagram=pack(&MediaDatagram{flags:0,stream_local_id:camera,seq:sequence,
                                    payload:rtp(96,sequence as u16,sample.index*3000,0x74000000+source as u32,
                                        (part+1)*1000>=sample.bytes.len(),&body).into()});
                                let submitted=tokio::select! { biased;
                                    _=stop.cancelled()=>break 'live,
                                    result=io.send_until(&datagram,until)=>result,
                                };
                                match submitted {
                                    Ok(())=>{},
                                    Err(connect_client::ClientError::Transport|connect_client::ClientError::MediaEnded)
                                        if retirement.committed()=>break 'live,
                                    Err(error)=>panic!("original candidate video submission failed before exact retirement: {error:?}"),
                                }
                                sequence=sequence.checked_add(1).expect("candidate native video datagram counter");
                                progress.lock().unwrap().published_video_packets+=1;
                            }
                            let mut measured=progress.lock().unwrap();
                            measured.published_frames+=1;
                            record_frame_bin(&mut measured.published_bins,Instant::now(),began);
                        } else {
                            let datagram=pack(&MediaDatagram{flags:0,stream_local_id:voice,seq:sample.index,
                                payload:rtp(111,sample.index as u16,sample.index*960,
                                    0x61000000+source as u32,false,&sample.bytes).into()});
                            let submitted=tokio::select! { biased;
                                _=stop.cancelled()=>break 'live,
                                result=io.send_until(&datagram,until)=>result,
                            };
                            match submitted {
                                Ok(())=>{},
                                Err(connect_client::ClientError::Transport|connect_client::ClientError::MediaEnded)
                                    if retirement.committed()=>break 'live,
                                Err(error)=>panic!("original candidate audio submission failed before exact retirement: {error:?}"),
                            }
                            progress.lock().unwrap().published_audio_packets+=1;
                        }
                    },
                    packet=io.receive()=>{
                        let packet=match packet {
                            Ok(packet)=>packet,
                            Err(connect_client::ClientError::MediaEnded|connect_client::ClientError::Transport)
                                if retirement.committed()=>break,
                            Err(error)=>panic!("original candidate media failed before exact retirement: {error:?}"),
                        };
                        progress.lock().unwrap().received_datagrams+=1;
                        if let Some(history)=packet.video_epoch {
                            {let mut measured=progress.lock().unwrap();measured.video_read_events+=1;measured.video_packets+=1;}
                            assert!(io.is_current_video_receive_epoch(incoming,&history).unwrap());
                            assert_eq!(u16::from_be_bytes(packet.bytes[2..4].try_into().unwrap()),incoming);
                            if epoch.as_ref()!=Some(&history) {
                                continuity=DecodeHistory::default();decoder.started=false;epoch=Some(history.clone());
                                progress.lock().unwrap().video_waiting_key=true;
                            }
                            let parsed=NativeVideoPacket::parse(packet.bytes[24..].to_vec()).unwrap();
                            if let Some(frame)=continuity.push(&parsed,&progress) {
                                if decoder.decode(&frame,packet.received_at,cutoff).await {
                                    observe_video(&progress,&frame,history,packet.received_at,began);
                                }
                            }
                        } else {
                            let ObservedNativeMediaDatagram::Rtp(packet)=observe_native_media_datagram(&packet.bytes).unwrap() else {progress.lock().unwrap().other_datagrams+=1;continue};
                            if packet.stream_local_id!=voice {progress.lock().unwrap().other_datagrams+=1;continue;}
                            {let mut measured=progress.lock().unwrap();measured.audio_read_events+=1;measured.audio_packets+=1;}
                            let packet=packet.rtp.into_packet();assert_eq!(packet.header.payload_type,111);
                            let decoded=audio.decode(packet.header.ssrc,packet.header.sequence_number,packet.header.timestamp,&packet.payload).unwrap();
                            observe_audio(&progress,&audio,decoded);
                        }
                    }
                }
                assert!(Instant::now()<cutoff,"original representative media programme deadline");
            }
        }).catch_unwind().await.map_err(panic_text);
        if result.is_err() {
            eprintln!(
                "representative candidate failed source={source} inbound={inbound} progress={:?}",
                progress.lock().unwrap()
            );
        }
        eprintln!(
            "representative candidate receiver notice observation source={source} inbound={inbound} connection={} incoming_handle={incoming} verified_receiver_notices={:?}",
            retirement.source,
            io.verified_video_receiver_notice_count_for_test()
        );
        let decoder_join = decoder.close().await;
        result.and(if decoder_join {
            Ok(())
        } else {
            Err("candidate decoder did not finish successfully".into())
        })
    })
}
fn fresh(progress: &Progress) -> bool {
    let now = Instant::now();
    progress.fragmented_keys > 0
        && !progress.video_waiting_key
        && progress.audio_samples >= 2880
        && [progress.last_video, progress.last_audio, progress.last_key]
            .iter()
            .all(|time| time.is_some_and(|at| now.saturating_duration_since(at) < AGE))
}
fn clone_progress(progress: &[SharedProgress]) -> Vec<Progress> {
    progress.iter().map(|p| p.lock().unwrap().clone()).collect()
}
fn original_binding_diagnostics(stage: &'static str, connection: &sdk::Connection, index: usize) {
    match connection.native_negotiation() {
        Ok(Some(selected)) => {
            let streams = selected
                .streams
                .iter()
                .take(8)
                .map(|stream| {
                    (
                        &stream.id,
                        &stream.medium,
                        &stream.direction,
                        stream.stream_local_id,
                    )
                })
                .collect::<Vec<_>>();
            let feedback = selected.video_feedback.keys().take(8).collect::<Vec<_>>();
            eprintln!("representative bindings stage={stage} route={index} expected_source={} connection={} revision={} source_clock={} streams={streams:?} omitted_streams={} feedback={feedback:?}",
                SOURCE_FOR_RECEIVER[index],connection.id().as_str(),selected.stream_revision.get(),selected.source_clock,selected.streams.len().saturating_sub(8));
        }
        Ok(None) => eprintln!(
            "representative bindings stage={stage} route={index} unavailable=not-installed"
        ),
        Err(error) => eprintln!(
            "representative bindings stage={stage} route={index} unavailable={:?}",
            error.kind
        ),
    }
    match connection.native_reconciliation() {
        Ok(Some(progress))=>eprintln!("representative reconciliation stage={stage} route={index} phase={:?} bindings_adopted={} expected_revision={} record={:?} subscription_record={:?} error={:?}",
            progress.phase,progress.bindings_adopted,progress.expected_revision,progress.record_id,progress.subscription_record_id,progress.error),
        Ok(None)=>eprintln!("representative reconciliation stage={stage} route={index} unavailable=not-installed"),
        Err(error)=>eprintln!("representative reconciliation stage={stage} route={index} unavailable={:?}",error.kind),
    }
}

async fn wait_original_playout(
    progress: &[SharedProgress],
    connections: &[sdk::Connection],
    until: Instant,
) {
    let result = timeout_at(until, async {
        while !progress.iter().all(|p| fresh(&p.lock().unwrap())) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    if result.is_err() {
        for (index, measured) in clone_progress(progress).iter().enumerate() {
            eprintln!("representative startup deadline route={index} expected_source={} progress={measured:?}",SOURCE_FOR_RECEIVER[index]);
            original_binding_diagnostics("startup-deadline", &connections[index], index);
        }
    }
    result.expect("all six original real decoders must become usable before replacement");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn signed_six_source_two_session_fragmented_vp8_handoffs_preserve_old_playout_and_decode_candidates(
) {
    representative(false, false).await;
}
#[cfg(feature = "test-support")]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn signed_parallel_original_allocations_release_authority_while_scene_coordinator_is_held() {
    representative(true, false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn signed_full_62_second_six_source_two_session_four_move_av_data_programme_joins_original_owners(
) {
    full_representative("EdDSA").await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn signed_es256_full_62_second_six_source_two_session_four_move_av_data_programme_joins_original_owners(
) {
    full_representative("ES256").await;
}
async fn full_representative(algorithm: &str) {
    // This outer fixture bound includes setup and ten seconds of idle. Every
    // programme/control/capture phase retains its separate original cutoff.
    let entered = Instant::now();
    representative_with_algorithm(false, true, algorithm).await;
    // Never drop the owning fixture future at an outer timeout: every actual
    // task/codec/client must join even if the final bounded verdict fails.
    assert!(
        entered.elapsed() < Duration::from_secs(180),
        "full fixture exceeded setup/programme/cleanup bound after owned joins"
    );
}
async fn representative(force_allocation_overlap: bool, full_programme: bool) {
    representative_with_algorithm(force_allocation_overlap, full_programme, "EdDSA").await;
}
async fn representative_with_algorithm(
    force_allocation_overlap: bool,
    full_programme: bool,
    algorithm: &str,
) {
    assert_eq!(std::env::consts::ARCH, "aarch64");
    assert!(
        matches!(std::env::consts::OS, "linux" | "macos"),
        "approved local ARM hosts"
    );
    // Match existing native fixture diagnostics without provisioning another
    // endpoint. This single selected integration opts in through RUST_LOG;
    // quantitative packet/scene counters already exist in the actual library.
    if std::env::var_os("RUST_LOG").is_some() {
        static LOGS: std::sync::Once = std::sync::Once::new();
        LOGS.call_once(|| {
            let _ = tracing_subscriber::fmt()
                .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap())
                .with_test_writer()
                .try_init();
        });
    }
    let mut f = Fixture::with_native_end_scope(algorithm, 16).await;
    assert_eq!(f.config.signing_algorithm, algorithm);
    assert_eq!(f.client.signing_algorithm, algorithm);
    assert_eq!(f.client.server_algorithm, algorithm);
    if full_programme {
        f.server.shutdown().await.unwrap();
        f.config.limits.message_bytes = 64 * 1024;
        f.server = RunningCanonicalServer::start(f.config.clone())
            .await
            .unwrap();
        f.client.endpoint = f.server.address;
    }
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
            super::super::super::super::canonical_native_connections::party_with_scope(
                &f,
                id,
                700 + n,
                vec!["audio".into(), "video".into(), "data".into()],
                participant_scope.clone(),
            )
        })
        .collect::<Vec<_>>();
    let replacement_config =
        super::super::super::super::canonical_native_connections::party_with_scope(
            &f,
            &identities[0],
            704,
            vec!["audio".into(), "video".into(), "data".into()],
            participant_scope.clone(),
        );
    // Every original and replacement peer uses the selected real key family.
    // This is setup-only: the programme/retirement clocks have not started.
    {
        let c = rusqlite::Connection::open_with_flags(
            &f.config.database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        for config in configs.iter().chain(std::iter::once(&replacement_config)) {
            assert_eq!(config.signing_algorithm, algorithm);
            assert_eq!(config.server_algorithm, algorithm);
            let enrolled: (String, String, String, u64) = c.query_row(
                "SELECT identity_id,device_id,algorithm,length(public_key) FROM peer_signing_keys WHERE key_id=?1 AND revoked=0",
                [&config.signing_key_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            ).unwrap();
            assert_eq!(
                enrolled,
                (
                    config.identity_id.clone(),
                    config.device_id.clone(),
                    algorithm.to_owned(),
                    if algorithm == "ES256" { 65 } else { 32 }
                )
            );
        }
    }
    let endpoints = [&configs[0].device_id, &replacement_config.device_id].map(|device| {
        rusqlite::Connection::open_with_flags(
            &f.config.database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap()
        .query_row(
            "SELECT id FROM endpoints WHERE device_id=?1",
            [device],
            |r| r.get::<_, String>(0),
        )
        .unwrap()
    });
    let mut clients = Vec::new();
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
    let mut sessions = Vec::new();
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
    let mut connections = Vec::new();
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
    for (index, connection) in connections.iter().enumerate() {
        let source = SOURCE_FOR_RECEIVER[index];
        let revision = connection
            .refresh(options())
            .await
            .unwrap()
            .stream_revision
            .unwrap();
        let mut selection_stage = "submit";
        let selection = async {
            let operation = connection
                .subscribe(
                    NonEmpty {
                        first: StreamSelector::Stream(
                            format!("representative-audio-{source}").parse().unwrap(),
                        ),
                        rest: vec![
                            StreamSelector::Stream(
                                format!("representative-video-{source}").parse().unwrap(),
                            ),
                            StreamSelector::Stream(
                                format!("representative-data-{source}").parse().unwrap(),
                            ),
                        ],
                    },
                    revision,
                    mutation(&format!("representative-select-{index}")),
                )
                .await?;
            selection_stage = "wait-original";
            operation.wait(options()).await?;
            Ok::<(), sdk::Error>(())
        }
        .await;
        if let Err(error) = selection {
            data::selection_failure(
                &f,
                connection,
                &format!("representative-select-{index}"),
                selection_stage,
                &error,
            );
            // A setup refusal must still close every exact already-offered
            // peer and join the host before propagating that first error.
            for client in &clients {
                let joined = client.close(options()).await;
                eprintln!("representative failed initial selection SDK join={joined:?}");
            }
            let joined = f.server.shutdown().await;
            eprintln!("representative failed initial selection server join={joined:?}");
            panic!(
                "original initial subscription failed: {error:?}; problem={:?}; observation={:?}",
                error.problem, error.observation
            );
        }

        original_binding_diagnostics("after-subscribe", connection, index);
    }
    let mut control_scope = Scope::collaboration(vec!["*".into()]);
    control_scope.operations.push("connection.replace".into());
    let credential = f
        .server
        .store
        .issue(
            &identities[0],
            control_scope,
            chrono::Utc::now().timestamp() + 600,
        )
        .unwrap();
    let mut http = canonical_http_config(&f);
    http.credential = credential.token;
    let mut controllers = Vec::new();
    for _ in 0..2 {
        controllers.push(CanonicalHttpClient::open(http.clone()).await.unwrap());
    }
    let progress = (0..6)
        .map(|_| Arc::new(Mutex::new(Progress::default())))
        .collect::<Vec<_>>();
    for measured in &progress {
        let mut observed = measured.lock().unwrap();
        observed.public = true;
        observed.public_since = Some(Instant::now());
    }
    let stop = CancellationToken::new();
    let retirements = (0..6)
        .map(|index| {
            Arc::new(RetirementWitness {
                database: f.config.database.clone(),
                source: connections[index].id().as_str().into(),
                original: Mutex::new(None),
            })
        })
        .collect::<Vec<_>>();
    let began = Instant::now() + Duration::from_millis(100);
    let programme_cutoff = began + Duration::from_secs(programme::SECONDS as u64);
    let read_cutoff = programme_cutoff + Duration::from_millis(200);
    let join_cutoff = programme_cutoff + Duration::from_secs(5);
    let mut masters = Vec::new();
    phase_timing("programme-anchor", began, None, None);
    let mut works = Vec::new();
    for index in 0..6 {
        let (master, owners) = programme::source(
            index,
            &binary,
            &raw[index],
            progress[index].clone(),
            stop.clone(),
            began,
            programme_cutoff,
        );
        works.extend(owners);
        masters.push(master);
        works.extend(spawn_publisher(
            &connections[index],
            index,
            &masters[index],
            progress[index].clone(),
            stop.clone(),
            retirements[index].clone(),
            began,
            programme_cutoff,
        ));
        works.push(spawn_original_receiver(
            &connections[index],
            index,
            SOURCE_FOR_RECEIVER[index],
            &binary,
            progress[index].clone(),
            stop.clone(),
            retirements[index].clone(),
            began,
            read_cutoff,
        ));
    }
    let mut targets = Vec::<Candidate>::new();
    let mut target_progress = Vec::<SharedProgress>::new();
    let mut quality_failures = Vec::new();
    let mut task_failures = Vec::new();
    let work=AssertUnwindSafe(async {
        wait_original_playout(&progress,&connections,began+Duration::from_secs(8)).await;
        // Warm up two genuine seconds before measuring the first preparation.
        tokio::time::sleep_until(began+Duration::from_secs(2)).await;
        eprintln!(
            "representative Store claim snapshot boundary=pre-preparation {}",
            f.server.store.packet_observer_claim_diagnostics()
        );
        let mut active_sources=connections.iter().map(|c|c.id().as_str().to_owned()).collect::<Vec<_>>();
        let mut active_progress=progress.clone();
        let mut previous_targets:[Option<usize>;2]=[None,None];
        let mut completed_originals=Vec::new();
        for round in 0..2usize {
            if full_programme {
                tokio::time::sleep_until(began+Duration::from_secs(programme::MOVES[round])).await;
            }
            let stream_phase=if round==0 {"target"} else {"return"};
            let source_generation=round as u64+1;
            let generation=source_generation+1;
            let before=clone_progress(&active_progress);
            let move_started=Instant::now();let move_cutoff=move_started+Duration::from_secs(8);
            assert!(move_cutoff<=programme_cutoff,"both rounds retain original movement budgets");
            phase_timing("round-start",began,Some(round),None);
            audio_continuity_snapshot("before",round,began,&before);
            let mut commands=Vec::new();
            for (sid,&index) in [0usize,4].iter().enumerate() {
                let revision=sessions[sid].refresh(short()).await.unwrap().topology_revision.get();
                let previous_publication=if round==0 {format!("representative-video-{index}")}
                    else {format!("representative-target-video-{index}")};
                let previous_receiver=if round==0 {format!("representative-video-{}",SOURCE_FOR_RECEIVER[index])}
                    else {format!("representative-target-incoming-{index}")};
                commands.push(TransitionCommand{operation_id:if round==0 {format!("representative-move-{index}")}else{format!("representative-return-move-{index}")},
                    conversation_id:room.id().as_str().into(),session_id:sessions[sid].id().as_str().into(),
                    connection_id:Some(active_sources[index].clone()),expected_revision:revision,
                    intent:TransitionIntent::Replace{endpoint_id:Some(endpoints[if round==0 {1}else{0}].clone()),candidate_connid:None,
                        retained_connid:connections[index+1].id().as_str().into(),video:Some(ReplacementVideo{
                            publications:vec![ReplacementVideoStream{previous:previous_publication,replacement:format!("representative-{stream_phase}-video-{index}")}],
                            receivers:vec![ReplacementVideoStream{previous:previous_receiver,replacement:format!("representative-{stream_phase}-incoming-{index}")}],
                        })},payload_extensions:Default::default()});
            }
            let [a,b]=controllers.as_mut_slice() else {unreachable!()};
            let (root0,root1)=tokio::join!(timeout_at(move_cutoff,a.transition(commands[0].clone())),
                timeout_at(move_cutoff,b.transition(commands[1].clone())));
            let roots=[root0.unwrap().unwrap(),root1.unwrap().unwrap()];
            let (prepared0,prepared1)=tokio::join!(timeout_at(move_cutoff,phase(&f,a,&commands[0],1)),
                timeout_at(move_cutoff,phase(&f,b,&commands[1],1)));
            prepared0.expect("original first prepare cutoff");prepared1.expect("original second prepare cutoff");
            // Exact source retirement is armed with this new original record,
            // but its predicate stays false until the genuine commit effect.
            for (sid,&index) in [0usize,4].iter().enumerate() {
                let witness=if let Some(previous)=previous_targets[sid] {
                    targets[previous].retirement.as_ref().unwrap().clone()
                } else {retirements[index].clone()};
                assert_eq!(witness.source,active_sources[index]);
                *witness.original.lock().unwrap()=Some(capture_retirement(&f,&commands[sid],&roots[sid],generation,move_cutoff));
            }
            let first=targets.len();
            let target_config=if round==0 {replacement_config.clone()} else {configs[0].clone()};
            assert_eq!(target_config.device_id,if round==0 {replacement_config.device_id.clone()}else{configs[0].device_id.clone()});
            // Return really reuses the original Device, credential and signed
            // identity. The four original public SDK controls remain alive.
            for _ in 0..2 {
                let client=timeout_at(move_cutoff,CanonicalClient::open(target_config.clone())).await.unwrap().unwrap();
                targets.push(Candidate{client,io:None,preparation:None,voice:0,camera:0,incoming:0,
                    force_key:Arc::new(AtomicBool::new(false)),retirement:None});
                target_progress.push(Arc::new(Mutex::new(Progress::default())));
            }
            let (_,fresh_targets)=targets.split_at_mut(first);let (left,right)=fresh_targets.split_at_mut(1);
            if force_allocation_overlap {
                #[cfg(not(feature = "test-support"))]
                panic!("forced actual coordinator overlap requires test-support");
                #[cfg(feature = "test-support")]
                {
                    // Both SAME signed requests must reach metadata-only wait
                    // while the actual coordinator is genuinely held. Initial
                    // application Writer aliases must drop or the second peer
                    // admission cannot reach this boundary at all.
                    let records=roots.iter().map(|r|r.record_id.as_ref().unwrap().as_str().to_owned()).collect::<Vec<_>>();
                    let original_rows=records.iter().map(|record| {
                        rusqlite::Connection::open(&f.config.database).unwrap().query_row(
                            "SELECT reservation_id,expires_at,candidate_connection_id,candidate_runtime_id,generation FROM native_replacements WHERE operation_record_id=?1",
                            [record],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,u64>(4)?))).unwrap()
                    }).collect::<Vec<_>>();
                    let probes=records.iter().map(|record|f.server.service.arm_canonical_preparation_registry_probe_for_test(
                        &format!("allocation-wait:{record}")).unwrap()).collect::<Vec<_>>();
                    let entered=CancellationToken::new();
                    let held=timeout_at(move_cutoff,f.server.service.with_canonical_replacement_coordinator_for_test(|| async {
                        phase_timing("coordinator-held",began,Some(round),None);
                        entered.cancel();
                        timeout_at(move_cutoff,async {
                            while !probes.iter().all(|probe|probe.load(Ordering::Acquire)==1) {
                                tokio::task::yield_now().await;
                            }
                        }).await.expect("both original signed requests must release application custody before coordinator wait");
                        phase_timing("coordinator-probe1-observed",began,Some(round),None);
                        let db=rusqlite::Connection::open(&f.config.database).unwrap();
                        for record in &records {
                            let begun:u64=db.query_row("SELECT count(*) FROM native_replacement_steps WHERE operation_record_id=?1 AND phase IN ('allocate','start') AND started=1",
                                [record],|r|r.get(0)).unwrap();
                            assert_eq!(begun,0,"metadata-only wait must begin no allocation/start effect");
                        }
                        phase_timing("coordinator-release-requested",began,Some(round),None);
                    }));
                    // Await the same original timeout/holder future. The helper
                    // drops its real role guard before this return is observed;
                    // no additional waiter, task, timeout or permission exists.
                    let held=async move {
                        let result=held.await;
                        if matches!(&result,Ok(Ok(_))) {
                            phase_timing("coordinator-holder-returned",began,Some(round),None);
                        }
                        result
                    };
                    let callers=async {
                        entered.cancelled().await;
                        tokio::join!(attach_candidate(&mut left[0],&commands[0],&roots[0],0,stream_phase,source_generation,&f.config.database,move_cutoff),
                            attach_candidate(&mut right[0],&commands[1],&roots[1],4,stream_phase,source_generation,&f.config.database,move_cutoff));
                    };
                    let (held,())=tokio::join!(held,callers);held.expect("original coordinator hold cutoff").unwrap();
                    assert!(probes.iter().all(|probe|probe.load(Ordering::Acquire)==2));
                    // This is an observation after the original callers join,
                    // not the unobserved instant of the production atomic store.
                    phase_timing("coordinator-probe2-observed-after-callers",began,Some(round),None);
                    for (record,original) in records.iter().zip(&original_rows) {
                        let current=rusqlite::Connection::open(&f.config.database).unwrap().query_row(
                            "SELECT reservation_id,expires_at,candidate_connection_id,candidate_runtime_id,generation FROM native_replacements WHERE operation_record_id=?1",
                            [record],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,u64>(4)?))).unwrap();
                        assert_eq!(&current,original,"coordinator contention cannot renew root, reservation, generation or expiry");
                    }
                }
            } else {
            tokio::join!(attach_candidate(&mut left[0],&commands[0],&roots[0],0,stream_phase,source_generation,&f.config.database,move_cutoff),
                attach_candidate(&mut right[0],&commands[1],&roots[1],4,stream_phase,source_generation,&f.config.database,move_cutoff));
            }
            for (sid,&index) in [0usize,4].iter().enumerate() {
                works.push(spawn_candidate(&mut targets[first+sid],index,SOURCE_FOR_RECEIVER[index],&binary,&masters[index],
                    target_progress[first+sid].clone(),stop.clone(),began,read_cutoff));
            }
            let held_until=Instant::now()+Duration::from_secs(2);
            assert!(held_until<move_cutoff);
            let ready=timeout_at(move_cutoff,async {
                loop {
                    if Instant::now()>=held_until && (first..first+2).all(|i|fresh(&target_progress[i].lock().unwrap())) {break;}
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }).await;
            if ready.is_err() {
                eprintln!(
                    "representative Store claim snapshot boundary=readiness-failure round={round} {}",
                    f.server.store.packet_observer_claim_diagnostics()
                );
                for sid in 0..2 {eprintln!("representative candidate readiness deadline round={round} route={} progress={:?}",[0,4][sid],target_progress[first+sid].lock().unwrap());}
            }
            let during=clone_progress(&active_progress);
            // Retain every old-view violation without suppressing later
            // observations. Actual candidate readiness and operation
            // preconditions remain immediate before each original Ready.
            let first_bin=move_started.saturating_duration_since(began).as_secs() as usize+1;
            let last_bin=Instant::now().saturating_duration_since(began).as_secs() as usize;
            phase_timing("quality-snapshot",began,Some(round),Some((first_bin,last_bin)));
            quality_failures.extend(old_playout_quality_failures(
                &before, &during, round, move_started, first_bin, last_bin,
            ));
            audio_continuity_snapshot("during",round,began,&during);
            ready.expect("both same original candidates require fresh real audio and fragmented decoded keys");
            for sid in 0..2 {
                let status=timeout_at(move_cutoff,controllers[sid].operation_get(&commands[sid].operation_id,scope(&commands[sid])))
                    .await.unwrap().unwrap();
                assert_eq!(status.record_id,roots[sid].record_id);
                assert_eq!(serde_json::to_value(&status).unwrap()["state"],"preparing");
                timeout_at(move_cutoff,async {
                    while !fresh(&target_progress[first+sid].lock().unwrap()) {tokio::time::sleep(Duration::from_millis(5)).await;}
                }).await.expect("original target observation still fresh immediately before Ready");
                let observed=target_progress[first+sid].lock().unwrap().clone();
                let ready=targets[first+sid].preparation.as_ref().unwrap().with_decoded(observed.audio_samples,
                    Some(NativeDecodedVideoReadiness{decoded_frames:observed.frames,decoded_keyframes:observed.keys})).unwrap();
                submit(&mut targets[first+sid].client,&ready,&mut controllers[sid],&commands[sid],&roots[sid],move_cutoff).await;
            }
            let mut committed_revisions=[0u64;2];
            for (sid,&index) in [0usize,4].iter().enumerate() {
                let done=timeout_at(move_cutoff,completed(&f,&mut controllers[sid],&commands[sid])).await.unwrap();
                assert_eq!(done.record_id,roots[sid].record_id);assert_eq!(done.operation_id,roots[sid].operation_id);
                let value=serde_json::to_value(done).unwrap();assert_eq!(value["state"],"completed");
                assert_eq!(value["result"]["generation"],generation);
                assert_eq!(value["result"]["replaces"],active_sources[index]);
                assert_eq!(value["result"]["device_id"],target_config.device_id);
                assert_eq!(value["result"]["endpoint_id"],endpoints[if round==0 {1}else{0}]);
                targets[first+sid].preparation.as_ref().unwrap().activation_from_completed(&value).unwrap();
                let committed_revision=value["result"]["readiness"]["stream_revision"].as_u64().unwrap();
                assert!(committed_revision>targets[first+sid].preparation.as_ref().unwrap().stream_revision(),
                    "same signed completed root readiness must advance its exact private preparation");
                committed_revisions[sid]=committed_revision;
                timeout_at(move_cutoff,async {
                    loop {
                        let (state,actual_generation):(String,u64)=rusqlite::Connection::open_with_flags(&f.config.database,
                            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap().query_row(
                            "SELECT state,generation FROM connections WHERE id=?1",[&active_sources[index]],
                            |row|Ok((row.get(0)?,row.get(1)?))).unwrap();
                        if (state.as_str(),actual_generation)==("retired",generation) {
                            let witness=if let Some(previous)=previous_targets[sid] {
                                targets[previous].retirement.as_ref().unwrap()
                            } else {&retirements[index]};
                            assert!(witness.committed(),"exact original commit must explain physical retirement");
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                }).await.expect("exact original source must retire within same original move deadline");
                active_sources[index]=targets[first+sid].preparation.as_ref().unwrap().connection_id().unwrap().into();
                active_progress[index].lock().unwrap().public=false;
                {
                    let mut observed=target_progress[first+sid].lock().unwrap();
                    observed.public=true;observed.public_since=Some(Instant::now());
                }
                completed_originals.push((sid,commands[sid].clone(),roots[sid].record_id.clone(),generation,value["result"].clone()));
            }
            let public_at_commit=clone_progress(&active_progress);
            let at_commit=clone_progress(&target_progress[first..first+2]);
            for target in &targets[first..first+2] {target.force_key.store(true,Ordering::Release);}
            let after_commit_until=move_cutoff.min(Instant::now()+Duration::from_secs(1));
            timeout_at(after_commit_until,async {
                loop {
                    let current=clone_progress(&active_progress);
                    if [1usize,5].iter().all(|&i|current[i].frames>public_at_commit[i].frames) {break;}
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }).await.expect("same retained receivers must decode fresh candidate uplink after each actual commit");
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert!(Instant::now()<move_cutoff,"postcommit continuity remains inside original operation budget");
            for (sid,&index) in [0usize,4].iter().enumerate() {
                let now=target_progress[first+sid].lock().unwrap().clone();
                assert!(now.frames>at_commit[sid].frames,"same actual decoder must continue through its commit");
                assert_eq!(now.epoch,at_commit[sid].epoch,"private decoder epoch changed at public activation");
                active_progress[index]=target_progress[first+sid].clone();previous_targets[sid]=Some(first+sid);
            }
            // Both Session public views and every unrelated original control
            // remain current after each round, including same-Device return.
            for (sid,session) in sessions.iter().enumerate() {
                let view=session.refresh(short()).await.unwrap();assert_eq!(view.state,"accepted");
                for client in &clients {client.conversation_by_id(room.id().clone()).refresh(short()).await.unwrap();}
                assert_eq!(session.id(),sessions[sid].id());
            }
            eprintln!("representative round={round} original_moves=2 generation={generation} old_preparation_fps_min=24 private_decoded_opus_min=2880 move_deadline_ms=8000 elapsed_ms={}",move_started.elapsed().as_millis());
            if full_programme {
                for (sid,&index) in [0usize,4].iter().enumerate() {
                    let candidate_id=targets[first+sid].preparation.as_ref().unwrap().connection_id().unwrap().to_owned();
                    data::exchange(&mut targets[first+sid].client,&commands[sid],candidate_id,committed_revisions[sid],
                        &connections[SOURCE_FOR_RECEIVER[index]],&connections[index+1],
                        &pids[OWNER[index]],&pids[OWNER[index+1]],index,stream_phase,
                        programme_cutoff.min(Instant::now()+Duration::from_secs(8))).await;
                }
            }

        }
        assert_eq!(completed_originals.len(),4);
        // Re-read every exact original root after both later operations. A
        // return cannot rewrite or replay a historical completed movement.
        for (sid,command,record,generation,original_result) in completed_originals {
            let status=controllers[sid].operation_get(&command.operation_id,scope(&command)).await.unwrap();
            assert_eq!(status.record_id,record);let value=serde_json::to_value(status).unwrap();
            assert_eq!(value["state"],"completed");assert_eq!(value["result"]["generation"],generation);
            assert_eq!(value["result"],original_result,"later return changed original completed result");
            let db=rusqlite::Connection::open_with_flags(&f.config.database,rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
            let (effects,confirmed,repeated):(u64,u64,u64)=db.query_row(
                "SELECT count(*),sum(state='confirmed'),sum(attempt>1 OR submission_generation>1) FROM operation_effects WHERE operation_record_id=?1",
                [record.as_ref().unwrap().as_str()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
            assert_eq!((effects,confirmed,repeated),(7,7,0),"each original movement has exactly one confirmed effect per phase");
        }
        if full_programme {
            tokio::time::sleep_until(programme_cutoff).await;
            assert!(Instant::now()>=programme_cutoff,"original 62-second programme must actually elapse");
            stop.cancel();
            assert!(Instant::now()<=read_cutoff,"read-stop issuance stays before original 62.2-second cutoff");
            // Join the actual original media/codec owners BEFORE any current
            // Connection end, exactly as stopAndJoin in the matrix workload.
            // No timeout drops a JoinHandle; a late physical join remains failed.
            while let Some(owner)=works.pop() {
                match owner.await { Ok(Ok(()))=>{},result=>task_failures.push(format!("{result:?}")) }
            }
            if Instant::now()>join_cutoff {task_failures.push("full programme owner join exceeded original67s cutoff".into());}
            quality_failures.extend(programme::quality(&progress,&target_progress));
            data::end_current(&connections,&mut targets,&previous_targets,&sessions,join_cutoff).await;
        }
        eprintln!("representative-six-source verified participants=4 sessions=2 sources=6 original_moves=4 generations=1,2,3 returned_original_device={} private_decoders=4 original_deadline_ms=8000 old_preparation_fps_min=24 genuine_opus_samples_min=2880 fragmented_vp8=true programme_bound_seconds=62",configs[0].device_id);
    }).catch_unwind().await;
    // Preserve original work failure, then join every exact original task and
    // decoder before propagating it. No forced process can qualify this test.
    stop.cancel();
    for task in works {
        // All work has its unchanged programme cutoff, and each codec owns a
        // bounded shutdown+wait. Never drop a live JoinHandle on a timeout.
        match task.await {
            Ok(Ok(())) => {}
            value => task_failures.push(format!("{value:?}")),
        }
    }
    if full_programme && Instant::now() > join_cutoff {
        task_failures
            .push("original programme owners did not join before original 67-second cutoff".into());
    }
    // Scalar final snapshots run only after every original codec/media task
    // joins. They neither retain media nor add work to a measured frame path.
    for (index, measured) in clone_progress(&progress).iter().enumerate() {
        eprintln!(
            "representative original final route={index} expected_source={} progress={measured:?}",
            SOURCE_FOR_RECEIVER[index]
        );
    }
    for (index, measured) in clone_progress(&target_progress).iter().enumerate() {
        eprintln!("representative candidate final index={index} progress={measured:?}");
    }
    // End both real Session owners using their current public topology. These
    // commands also exercise the shared original client after both handoffs.
    for (sid, session) in sessions.iter().enumerate() {
        if full_programme && work.is_ok() && Instant::now() >= join_cutoff {
            task_failures.push(
                "original Session control retirement deadline expired; no renewed success budget"
                    .into(),
            );
            break;
        }
        let end = AssertUnwindSafe(async {
            let before = session
                .refresh(if full_programme && work.is_ok() {
                    programme::request(join_cutoff)
                } else {
                    options()
                })
                .await?;
            if before.state == "ended" {
                return Ok::<_, sdk::Error>(());
            }
            session
                .end(
                    0,
                    "representative original cleanup".into(),
                    before.topology_revision,
                    if full_programme && work.is_ok() {
                        programme::mutation_at(
                            &format!("representative-session-end-{sid}"),
                            join_cutoff,
                        )
                    } else {
                        mutation(&format!("representative-session-end-{sid}"))
                    },
                )
                .await?
                .wait(if full_programme && work.is_ok() {
                    programme::request(join_cutoff)
                } else {
                    options()
                })
                .await?;
            Ok::<_, sdk::Error>(())
        })
        .catch_unwind()
        .await;
        match end {
            Ok(Ok(())) => {}
            Ok(Err(error)) => task_failures.push(format!("original Session cleanup: {error:?}")),
            Err(error) => task_failures.push(panic_text(error)),
        }
    }
    let drain_until = if full_programme && work.is_ok() {
        join_cutoff
    } else {
        Instant::now() + Duration::from_secs(5)
    };
    let drained = tokio::time::timeout_at(drain_until, async {
        loop {
            let snapshot = f.server.service.canonical_runtime_snapshot().unwrap();
            if snapshot.live_sessions == 0
                && snapshot.connections == 0
                && snapshot.replacement_preparations == 0
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    if drained.is_err() {
        task_failures.push("original Session runtime owners did not drain".into());
    }
    drop(masters);
    for target in targets {
        if let Err(error) = target.client.shutdown().await {
            task_failures.push(format!("original target shutdown: {error}"));
        }
    }
    for client in clients {
        let close_request = if full_programme && work.is_ok() && Instant::now() < join_cutoff {
            programme::request(join_cutoff)
        } else {
            // Failure cleanup still joins; it can never renew the success bound.
            options()
        };
        if let Err(error) = client.close(close_request).await {
            task_failures.push(format!("original SDK shutdown: {error}"));
        }
    }
    if full_programme {
        // This proof is AFTER every original Session/runtime/media/codec/raw
        // peer/SDK close. Ten seconds of idle cannot hide late physical joins.
        let all_original_joined = Instant::now();
        if all_original_joined > join_cutoff {
            task_failures.push(format!("all original physical/control/client owners joined at programme_ms={} beyond unchanged67000 cutoff",
                all_original_joined.saturating_duration_since(began).as_millis()));
        }
        let idle = AssertUnwindSafe(programme::idle(&f)).catch_unwind().await;
        if let Err(error) = idle {
            task_failures.push(panic_text(error));
        }
    }
    eprintln!(
        "representative Store claim snapshot boundary=terminal {}",
        f.server.store.packet_observer_claim_diagnostics()
    );
    let shutdown = f.server.shutdown().await;
    // Preserve secondary failures even when an original protocol/readiness
    // panic is resumed. No failed quality, child or shutdown is waived.
    eprintln!(
        "representative terminal quality_failures={quality_failures:?} task_failures={task_failures:?} server_shutdown={shutdown:?}"
    );
    if let Err(panic) = work {
        std::panic::resume_unwind(panic);
    }
    assert!(
        task_failures.is_empty(),
        "original media/codec owners did not join cleanly: {task_failures:?}"
    );
    shutdown.unwrap();
    require_old_playout_quality(&quality_failures);
}

// These controls exercise the same real SDK packet parser, bounded assembler
// and decoder gate used above. The separate original C-codec loss controls
// establish why an emitted delta across a missing reference is unsafe.
fn continuity_packet(sequence: u16, timestamp: u32, ssrc: u32, key: bool) -> NativeVideoPacket {
    let mut bytes = vec![0x80, 0x80 | 96];
    bytes.extend(sequence.to_be_bytes());
    bytes.extend(timestamp.to_be_bytes());
    bytes.extend(ssrc.to_be_bytes());
    bytes.extend([0x10, u8::from(!key), 0, 0]);
    NativeVideoPacket::parse(bytes).unwrap()
}

#[test]
fn representative_video_history_gates_missing_references_until_original_key() {
    let progress = Arc::new(Mutex::new(Progress::default()));
    let mut history = DecodeHistory::default();
    assert!(
        history
            .push(&continuity_packet(10, 100, 7, true), &progress)
            .unwrap()
            .keyframe
    );
    assert!(
        !history
            .push(&continuity_packet(11, 200, 7, false), &progress)
            .unwrap()
            .keyframe
    );
    // Complete frame 12 was lost, not merely delayed fragment reordering.
    assert!(history
        .push(&continuity_packet(13, 400, 7, false), &progress)
        .is_none());
    assert!(history
        .push(&continuity_packet(14, 500, 7, false), &progress)
        .is_none());
    let now = Instant::now();
    assert!(history.take_request(now));
    assert!(!history.take_request(now + Duration::from_millis(99)));
    assert!(history.take_request(now + Duration::from_millis(100)));
    // The same history/decoder owner resumes at the original sender's key.
    assert!(
        history
            .push(&continuity_packet(15, 600, 7, true), &progress)
            .unwrap()
            .keyframe
    );
    assert!(!history.take_request(now + Duration::from_millis(200)));
    assert!(history
        .push(&continuity_packet(16, 700, 7, false), &progress)
        .is_some());
    history.gap(&progress);
    assert!(history
        .push(&continuity_packet(17, 800, 7, false), &progress)
        .is_none());
    assert!(history
        .push(&continuity_packet(18, 900, 7, true), &progress)
        .is_some());
    let measured = progress.lock().unwrap();
    assert_eq!(measured.decode_attempts, 5);
    assert_eq!(measured.skipped_delta_frames, 3);
    assert_eq!(measured.video_losses, 2);
    assert_eq!(measured.current_decode.unwrap().1.first, 18);
}

#[test]
fn representative_video_history_preserves_wrapping_order_and_epoch_key_boundaries() {
    let progress = Arc::new(Mutex::new(Progress::default()));
    let mut history = DecodeHistory::default();
    assert!(history
        .push(
            &continuity_packet(u16::MAX, u32::MAX - 10, 7, true),
            &progress
        )
        .is_some());
    assert!(history
        .push(&continuity_packet(0, 10, 7, false), &progress)
        .is_some());
    assert!(history
        .push(
            &continuity_packet(u16::MAX, u32::MAX - 10, 7, true),
            &progress
        )
        .is_none());
    assert!(history
        .push(&continuity_packet(0, 10, 7, false), &progress)
        .is_none());
    // A new SSRC is never permission to feed a delta to the existing decoder.
    assert!(history
        .push(&continuity_packet(1, 20, 8, false), &progress)
        .is_none());
    assert!(history
        .push(&continuity_packet(2, 30, 8, true), &progress)
        .is_some());
    // Caller performs this explicit reset only at an authenticated epoch cut.
    history = DecodeHistory::default();
    assert!(history
        .push(&continuity_packet(3, 40, 8, false), &progress)
        .is_none());
    assert!(history
        .push(&continuity_packet(4, 50, 8, true), &progress)
        .is_some());
    assert!(history
        .push(&continuity_packet(5, 60, 8, false), &progress)
        .is_some());
    let measured = progress.lock().unwrap();
    assert_eq!(measured.late_video_packets, 2);
    assert_eq!(measured.skipped_delta_frames, 2);
    assert_eq!(measured.decode_attempts, 5);
}

#[test]
fn representative_recovered_key_keeps_the_next_contiguous_delta() {
    let progress = Arc::new(Mutex::new(Progress::default()));
    let mut history = DecodeHistory::default();
    let fragment = |sequence, timestamp, marker| {
        let mut bytes = continuity_packet(sequence, timestamp, 7, false).rtp;
        if marker {
            bytes[12] = 0; // Final fragment, with the start still in flight.
        } else {
            bytes[1] &= !0x80; // Start fragment whose marker will be lost.
        }
        NativeVideoPacket::parse(bytes).unwrap()
    };
    assert!(history
        .push(&continuity_packet(10, 100, 7, true), &progress)
        .is_some());
    for i in 0..8 {
        assert!(history
            .push(
                &fragment(11 + i * 2, 200 + u32::from(i) * 100, false),
                &progress
            )
            .is_none());
    }
    assert_eq!(history.assembler.diagnostics().pending_frames, 8);
    assert!(
        history
            .push(&continuity_packet(27, 1000, 7, true), &progress)
            .unwrap()
            .keyframe
    );
    let losses_at_key = progress.lock().unwrap().video_losses;
    let now = Instant::now();
    assert!(!history.take_request(now));

    // Receive the future frame's final fragment before the contiguous delta.
    // Before the fix the old fragments still fill the assembly queue, and an
    // unrelated eviction during this delta creates a second false key wait.
    assert!(history.push(&fragment(30, 1200, true), &progress).is_none());
    let next = history.push(&continuity_packet(28, 1100, 7, false), &progress);
    assert!(
        next.is_some_and(|frame| !frame.keyframe),
        "obsolete partial forced a key wait after exact recovery"
    );
    assert_eq!(progress.lock().unwrap().video_losses, losses_at_key);
    assert!(!history.take_request(now));
    assert!(
        history
            .push(&fragment(29, 1200, false), &progress)
            .is_some(),
        "newer reordered tail must survive the key boundary"
    );

    // A genuinely newer gap still cannot feed a delta to this decoder.
    assert!(history
        .push(&continuity_packet(32, 1400, 7, false), &progress)
        .is_none());
    assert!(history.take_request(now));
    assert_eq!(progress.lock().unwrap().video_losses, losses_at_key + 1);
}

#[cfg(feature = "test-support")]
#[path = "representative/initial_av.rs"]
mod initial_av;
