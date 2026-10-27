# Native audio/video participant

`connect-video` uses the public Rust `ConnectClient` over authenticated UCTP/QUIC.
It publishes separate PCMU audio and VP8 video streams in an accepted durable
Session. The server forwards encoded VP8; FFmpeg capture/encoding and ffplay
rendering belong to this example. This work is under Gate 5 qualification;
source compilation is not a release or decoded-route acceptance claim.

Build from the prepared workspace:

```sh
bash scripts/prepare-dependencies.sh
cargo build --locked -p connect-native-video --bin connect-video
```

The local qualification tooling selects FFmpeg 9.0.2 with `libvpx`. Install FFmpeg and
ffplay explicitly; the Connect server needs neither. Start a disposable local
server using the versioned [setup instructions](../../docs/SETUP.md). Supply two
ordinary, revocable client configurations; task tracking is optional.

Prepare an accepted audio/video Session through the Python native helper. Its
private intent file retains mutation IDs through an uncertain acknowledgment:

```sh
PYTHONPATH=sdk/python python3 examples/native-video/prepare-session.py \
  --human .local/demo/human.toml --peer .local/demo/agent.toml \
  --intent .local/demo/video-intent.json --connect target/debug/connect
```

Use the returned conversation, session and opposite identity IDs to start both
participants in separate terminals. Each output path must be new:

```sh
target/debug/connect-video --config .local/demo/human.toml \
  --conversation CONVERSATION --session SESSION --peer AGENT_ID \
  --native-peer --input test --audio-input tone:440 --seconds 60 \
  --output .local/demo/human-received.ivf

target/debug/connect-video --config .local/demo/agent.toml \
  --conversation CONVERSATION --session SESSION --peer HUMAN_ID \
  --native-peer --input test --audio-input tone:660 --seconds 60 \
  --output .local/demo/agent-received.ivf
```

`--input test` generates moving video. `--input camera:0` explicitly selects a
macOS AVFoundation camera. `--audio-input microphone:0` explicitly selects its
microphone. Device permission refusal or a missing encoder ends the attempt;
the example does not change device permissions. Add `--preview` to render the
incoming video with ffplay, or independently inspect a retained output:

```sh
ffplay .local/demo/human-received.ivf
ffmpeg -i .local/demo/human-received.ivf -f framemd5 -
```

Keep private media under ignored `.local/`. Generated-camera results do not
qualify human hardware. Audio capture/encoding is independent of video; the
Rust SDK offers camera and microphone mute separately. The command owns and
stops its codec, capture, renderer and media work on interruption or failure.
Slow preview consumes a bounded two-frame queue and cannot back up live media.
The optional ffplay preview renders video only; the example does not implement
live audio playback. `--audio-output PATH` retains received PCMU for independent
decoding, and `--report PATH` writes a bounded numeric arrival/emission journal.
Both paths must be new and stay under private `.local/` storage. `--ffmpeg PATH`
selects the executable used for both video encoding and audio capture.
The report separates the public SDK's client queue/buffer snapshot from the
example's assembler and playout measurements. SDK channel high water is sampled
at send/receive and pump polls; exact incoming enqueue peaks and opaque Quinn
internal queued age/evictions are not observed.

Automated checks use `--input test-pulse` and `--audio-input pulse:FREQUENCY`
to generate synchronized white video and identifiable audio gain markers.
The generated fixture uses `connect-av-cadence/1`: a bounded, nonrepeating
cadence with 0.5/0.9/1.3-second intervals and 100 ms pulses. Observers independently
calibrate each decoded track's marker ordinals and pair only equal ordinals.
Missing or ambiguous calibration fails; nearest identical periodic pulses cannot
establish this proof. This changes fixture coding, not the codec or skew limits.
`--fixture-marker ID` (0–3) adds four large gray cells identifying the sender;
this option is restricted to generated `test`/`test-pulse` input. Independent
observers reject swapped or looped-back video separately from audio identity.
Full-white synchronization pulses are excluded only from source identification,
while moving-frame and A/V checks remain unchanged. The reliable-file checker
waits for actual receive-window readiness and records transfer start/end times;
a clean proof requires the complete transfer inside every receiver's window.
The codec readers pace buffered output at 30 video frames per second and one
20 ms audio frame per tick. Missed ticks are skipped, so codec startup or stalls
cannot turn buffered frames into a burst of future RTP timestamps. Capture
queues retain their two-frame bound.

All native inputs use a maximum six-frame VP8 GOP (200 ms at 30 fps), retaining
the 1 Mbps target and existing frame and queue bounds. More frequent independent
frames shorten the reference chain after packet loss; a received keyframe can
still be lost, so this source policy does not promise 200 ms receiver recovery.
Native QUIC uses periodic keyframes rather than negotiated keyframe feedback.

The native participant orders complete received frames and writes them live to IVF
and PCMU, recording actual arrival and emission times. Its intended local playout
hold is capped at 120 ms from receipt; the separate 200 ms stale-frame deadline
still applies. After a gap in completed RTP sequences, it suppresses dependent
VP8 interframes until a fresh keyframe rather than feeding a broken reference
chain into the decoder. The SDK exposes completed sequence ranges without
changing the existing encoded-frame interface. The automated checker
independently decodes that retained media with FFmpeg **after the run**. It binds
each decoded video PTS exactly to the real IVF presentation timestamp and live
complete-frame journal; skipped decoder inputs are counted, and unknown or
duplicate mappings fail. No offline frame sorting supplies the live order.

Native AV marker offsets measure receive-side emission alignment with pixels
and audio verified by independent post-run decoding. The two-second recovery
check verifies a decoded post-burst keyframe and subsequent changing pixels at
their recorded live emission times. These checks do not measure actual decoder
completion time, physical display/speaker alignment or live audio playback
latency. Browser measurements independently observe decoded video rendering
and received audio on the browser's performance clock. Browser audio readiness
requires three consecutive identified received-tone samples before measurement.
During the window, 100 ms RMS/frequency observations require at least 90% of the
configured sample count and no observation gap above 200 ms. Every clean sample
must identify the expected tone; mute/burst exceptions are bound to actual events
and measured silence/recovery, with recovery within two seconds. These are
observer coverage bounds, not a physical speaker or universal playback claim.

For canonical browser participation, borrow an authenticated signed WSS client
and retain explicit original intent IDs:

```js
const video = await BrowserVideoSession.open(signedClient, {
  conversationId, sessionId, connectionId, operationId, expectedRevision,
  participantId, audioStreamId, videoStreamId,
  inputStream: generatedOrExplicitCaptureStream,
});
// Explicit stream.subscribe commands use signedClient and original selection IDs.
await video.end(endOperationId, currentConnectionRevision);
await video.close(); // Local cleanup only; signedClient remains caller-owned.
```

The browser SDK snapshots scope before capture and owns supplied tracks. It uses
Opus audio and VP8; current fixed-host selectors accept audio participant pins
and exact current video sources. The earlier bearer-only native/browser runner
requires a new signed canonical fixture and does not qualify this profile.
See [the canonical browser API](../../docs/developer/canonical-webrtc-signaling.md#public-browser-session-api)
for configuration, uncertainty and remaining qualification scope.

See [the media profile](../../contracts/connect-media-vp8-v1.md) for wire limits,
subscription identities, feedback boundaries and supported codecs.

Qualification also checks fresh decoded motion across the entire measured window:
no unexcluded observation gap may exceed two seconds, including the start and end
boundaries. Native observations use exact decoded PTS mapped to the live emission
journal; browser observations use independently decoded render changes. Only
actual camera-mute intervals and explicitly timed injected network bursts are
excluded. Motion must resume within two seconds after each interval, and an
unrelated freeze fails even if the full-call average reaches 24 changing frames
per second. This does not measure physical display latency or native independent
decoder completion time.
