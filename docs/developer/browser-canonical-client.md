# Canonical browser client (G6-050 in progress)

`@rudeless/connect/browser` exports `BrowserClient` for the canonical
`connect/2` WebTransport control path. It requires a secure context, WebTransport
and WebCrypto. The current checkpoint implements signed control, RTP,
WebCodecs and negotiated media/data integration; capture/playback and the complete media
qualification are still in progress. The existing WebRTC voice surface uses an
earlier signaling dialect; canonical fallback is the separate G6-051 task.

The trusted application backend provisions the participant's scoped credential,
Device endpoint and signing key enrollment. Pass a browser-owned private
`CryptoKey` and separately enrolled server verification `CryptoKey`. Both
Ed25519 (`EdDSA`) and P-256 (`ES256`) are supported. Ordinary HTTPS certificate
verification remains enabled. Explicit `serverCertificateHashes` are available
for designated short-lived development certificates, independently of the
server's enrolled Sig9421 identity. Credentials and private keys must not be
placed in URLs, diagnostics or application logs.

```ts
import { BrowserClient } from "@rudeless/connect/browser";

// These values come from the application's trusted enrollment flow.
const client = await BrowserClient.open({
    url: participant.webTransportUrl,
    identityId: participant.identityId,
    device: participant.device,
    credential: participant.credential,
    signing: { key: privateSigningKey, keyId, algorithm: "EdDSA" },
    server: {
        key: enrolledServerKey, keyId: enrolledServerKeyId, algorithm: "EdDSA",
        notBefore: serverKeyNotBeforeUnixSeconds,
        expiresAt: serverKeyExpiresAtUnixSeconds,
    },
    signal: controller.signal,
    timeoutMs: 10000,
});

// Persist this ID with the user's intention before submitting a mutation.
const result = await client.createConversation(savedLogicalOperationId);
const operation = await client.operationStatus(result.id, savedLogicalOperationId);
client.close();
```

`createConversation` requires the durable receipt, rather than inferring
completion from an ACK. Reopening with current authority and the same original
logical operation ID asks the server for the original result. Every envelope
has a fresh ID, timestamp and signature. Deadline or cancellation after
submission retires the uncertain physical peer without retrying or rolling back
the mutation. Keep the original logical operation ID and reconcile through a
fresh authenticated client. Cancellation before submission sends nothing.

`request` exposes advanced canonical commands with typed envelope scopes.
`onEvent` registers a synchronous observer for unsolicited notifications and
returns its disposer. Notifications are hints: reconcile state through operation
status or authoritative snapshots. Callback failures close the client. Each
physical peer allows 16 outstanding requests, at most 128 envelopes/1 MiB per
reply, 256 envelopes/1 MiB of unconsumed notifications, and 4096 replay IDs
within the signature freshness window. Overflow closes the peer explicitly.
The streaming parser rejects duplicate decoded JSON keys, malformed Unicode,
nonfinite numbers, excessive nesting, and reliable frames above 1 MiB.

The separate RTP module parses complete RTP v2 packets after the eight-byte
UCTP header. `MediaBindings` retains retired handles to refuse physical handle
reuse, wrong directions and mismatched payload types. It is local routing
state; server authorization remains authoritative. `RtpSender` packetizes Opus
and VP8 using the frozen profile. `Vp8Assembler` accepts reordered contiguous
fragments across sequence wrap, expires incomplete frames at 200 ms and holds
at most eight assemblies/64 fragments. It never emits partial frames.
`MediaSendQueue` drops obsolete work and enforces ten audio packets, 64 video
packets and 76032 total bytes. These classes do not themselves establish a
negotiated Connection, a subscription, observed media readiness or playback.

`browserCodecSupport()` checks the actual Opus/VP8 encoder and decoder
configurations. `BrowserOpusCodec` consumes 960 mono float PCM samples per 20 ms
frame at 48 kHz and targets 32 kbit/s. `BrowserVp8Codec` consumes and closes
`VideoFrame` inputs, targets 640x360/30 fps and 500 kbit/s, caps emitted video
at 1 Mbit per sliding second and requests a keyframe at least every second of
submitted media. A rolling window permits at most 30 input frames per second
while accepting ordinary capture timestamp jitter. Queues drop excess work; a receiver waits for a fresh keyframe
after declared loss. `requestKeyFrame()` lets the owning media path request
recovery after dropping output. Decoded callbacks borrow the frame synchronously;
the codec always closes it, including on callback failure. `RtpClock` unwraps
RTP timestamps; it alone does not establish cross-stream A/V synchronization.

`BrowserMediaPeer` owns one authenticated client's datagram reader/writer,
bounded send queue, codec routes and expiry timer. `offerConnection` takes
stable Conversation, Session, Connection and operation IDs, named Opus/VP8
Streams, and optional named `text`/`json`/`binary` data Streams. It binds the
server's signed native parameter offer to the original wire ID, exact resource
scope and current stream revision, confirms those parameters once, then checks
the authoritative operation status. There is no additional root ACK after a
native `connection.answer`. A historical completed offer on a fresh physical
peer cannot establish new media; inspect its original operation status rather
than automatically redialing with another logical ID.

`connection.subscribe(operationId, participantId, kinds)` explicitly authorizes
the requested source selection on the server. Non-audio subscriptions create
separate receiver ports named after the original source Streams. The SDK permits
these signed receiver additions within declared codecs and selected kinds; they
cannot introduce a local publisher. Local publishers remain distinct. A
Connection advertises a bounded 32-port capacity so a five-Stream publisher can
also receive its peer's four video/data Streams. This is an implementation bound,
not a qualified scale claim. Up to eight Connections/256 active datagram bindings
are tracked per media peer.

`connection.encodeAudio` takes a named local Stream, 960 float samples and a
microsecond capture timestamp. `encodeVideo` consumes/closes a `VideoFrame` for
the named local video/screen Stream. Decoded media callbacks carry the negotiated
binding. `BrowserMediaCapture.open(connection, options)` captures into named
negotiated senders using an owned microphone/camera/screen or an explicitly
transferred `MediaStream`. `browserCaptureSupport()` detects required primitives
before permission. The audio worklet holds at most ten 20 ms mono frames and
discards partial samples and stale credits on pause/resume. Video uses frame
callbacks at 640x360. The helper stops late permission grants after cancellation
or the startup deadline, pauses locally when hidden, and requires explicit
`resume()` from a visible page. Connection closure stops all owned tracks, frame
callbacks, graph nodes and worklet ports. Device IDs stay local to capture.
These ownership paths have controlled SDK tests and a separate real-Chrome
checkpoint using identifiable supplied streams, fake devices and permission
denial. Real tab visibility pauses capture; visible-page resume restores it.
The short fixture uses a mock Connection, so connected capture and native
interoperation still require their own evidence.

`BrowserMediaRenderer.open(connection, { video: { [streamId]: canvas } })`
creates a Connection-owned Web Audio graph and uses application canvases for
named receive Streams. Forward decoded callbacks to `renderer.audio(binding,
frame)` and `renderer.video(binding, frame)`. Both borrow synchronously; the
renderer copies PCM and never retains decoded frames. Playback holds at most
ten 20 ms sources and 200 ms of scheduled audio. Hidden pages pause and discard
pending output; call `resume()` from a visible page. `close()` stops buffers and
closes the graph; await `finished` to observe completion or graph-close failure.
Diagnostics expose bounded counts, audio graph state and aggregate RMS, without
raw samples. This checkpoint verifies actual canvas/audio-graph rendering;
full cross-stream synchronization remains unqualified.

`connection.sendData(operationId, messageId, streamId, body)` uses signed,
reliable ordered canonical publication with original logical IDs and at most
64 KiB decoded bytes. Completion proves queue admission, rather than recipient
consumption; the recipient observes its actual data callback with original
sender/message/operation attribution. `connection.end` reconciles terminal
operation status and retires local routes. Normal clearing names the exact
Connection and original operation ID without conditioning terminal intent on a
stream revision that server reconciliation may advance. Current credential,
Connection generation and physical ownership checks still apply. Before admitting
a new terminal or reliable data operation, the service may wait up to one second for an
internal managed reconciliation to settle. Each wait releases SQL custody and
rechecks the original authority and physical lifetime. No new operation ledger, durable message or
engine effect exists during that wait. Competing client controls remain
conflicts; a bounded wait that does not settle returns `busy`. Accepted or
uncertain effects are never resubmitted by this wait.
Physical client closure retires all
owned media codecs, readers, writers, timers and queued datagrams.

The explicit short codec lane synthesizes two distinct tones and moving color
markers, encodes them with the production modules, packetizes/reassembles RTP,
and independently inspects decoded PCM and rendered canvas pixels in Chrome.
It is a four-second primitive check, separate from capture, network, native
interoperation and the full 60-second operating measurements:

```sh
npm run build --prefix sdk/typescript
node scripts/rehearse-gate6-browser-codecs.mjs .local/gate-6/browser-sdk/unique-codec-run
```

Default SDK checks:

```sh
bash scripts/prepare-dependencies.sh
npm ci --offline --prefix sdk/typescript
npm test --prefix sdk/typescript
```

The explicit local Chrome control lane builds the SDK and server, owns a fresh
private browser profile, serves the actual SDK modules, and seals the executable
and source digests before running. It also repeats the independently implemented
browser fixture as a regression; its PCMU decode is not SDK media evidence.

```sh
python3 scripts/rehearse-gate6-browser-sdk.py --installed-browser \
  --target-dir /absolute/path/to/private-build-target \
  --output .local/gate-6/browser-sdk/unique-run
```

Add `--media` to select the short synthetic Opus/VP8/data integration cases
alongside the control/regression cases. The fixture observes independently
identified decoded readiness before its short measurement window; a successful
offer or subscription ACK cannot begin a media-ready measurement. Raw results
retain failed and unresolved checks explicitly. This lane does not substitute
for capture/playback, browser/native or the full frozen operating measurements.

Add `--capture` to also exercise supplied MediaStream capture, AudioWorklet,
production codecs, real WT routing and the production renderer together.
Independent decoded PCM and rendered pixels identify both directions; measured
playback counts, signal power and bounded audio queues are checked. No personal
devices are opened. Current full connected capture results remain intermittent
on the unchanged audio threshold; separate individually successful methods
do not qualify the whole matrix. See the linked Gate 6 evidence.

The full G6-050 completion still requires identified duplex Opus/VP8/data
between browser/browser and browser/native, capture/permission and unsupported
API/codec refusals, background/resume, impairment recovery, and owned teardown.
Transport availability and these control/RTP checks do not qualify those paths.

The explicit `--clean60` lane implies `--capture` and applies the approved
60-second measurement to the two capture methods. `--case` can select one exact
capture method for diagnosis; omitted methods remain unqualified. It requires
Chrome 155.0.8059.85 before the measurement, at least 24 identified decoded and
rendered video frames per second over the measured window, and at least 42
uniquely paired audio-graph/canvas markers per receiver. Median absolute skew
must be at most 80 ms and nearest-rank p95 at most 120 ms. The existing audio
checkpoint ratio remains an additional check. Partial initial markers are
excluded; every independently observed matching ID is paired without filtering
by latency. The receiver observes the real renderer analyser and canvas rather
than treating RTP timestamps as synchronization evidence.

One-second observations attribute the owned Rust server test host and its Chrome
processes separately; resource files describe whole-test lifetime CPU and sampled
peak RSS limitations. This browser/browser topology has no native helper. The
profile and observer parameters are frozen in
`docs/evidence/gate-6/browser-client-clean60-profile.json` before execution.
The initial harness checkpoint is syntax/smoke verified and remains runtime
unqualified. Packet tracing cannot be combined with clean measurement, and
inherited profile/trace environment flags cannot silently select another lane.

VP8 complete frames carry their first and last RTP sequence numbers. The
receiver requires a fresh keyframe after a sequence gap, including loss of an
entire frame that leaves no partial assembly to expire. Late or duplicate
frames cannot rewind the current reference chain; contiguous RTP sequence and
timestamp wrap remains valid. The explicit codec rehearsal option
`--whole-frame-loss` checks this recovery with actual Chrome WebCodecs and
independently identified canvas output. Connected throughput and the full
impairment profile require their separate runtime evidence.

Current packet authority reads allow a ten-millisecond retry window for
transient SQL or file-admission contention. Every attempt resolves current
authority, and the engine checks the original gate/scene stamps afterward.
The window bounds retry admission; it does not claim a hard bound on an
individual SQL projection. Full operating latency remains subject to the
frozen marker and queue measurements.

`peer.routeDiagnostics` includes numeric VP8 recovery observations: source
epochs, complete-frame sequence gaps, late frames, assembly losses, decoder
queue drops and whether a keyframe is required. These counters support
troubleshooting and grant no authority; they contain no media bytes.

Server-side retained subscription reconciliation treats a transient catalog
read failure as a request to read the current catalog again. An unchanged
recovered catalog does not create another reconciliation operation. Current
authorization retries and exact captured-outcome recovery keep their own
state; genuine source or capability changes still require fresh authorization
and negotiation. Packet publication and delivery continue to check current
authority independently.

The explicit `--native` option adds four short browser/public `CanonicalClient` QUIC Opus/VP8/data cases (both signing algorithms, synthetic and supplied-stream capture). Native encoded fixture sources use the frozen codec bounds; native received RTP is independently decoded with its actual arrival-window mapping. Only the browser private key is given to the browser in this topology. The current checkpoint passes all four native cases with independently decoded received Opus/VP8/data. The separate 60-second browser profile still fails sustained audio delivery; see `docs/evidence/gate-6/browser-client-native-interop.json`. The same-host native actor, preencoded native source and offline independent decoder do not qualify live native encoding/playout, separate native resource usage, full60seconds or synchronization.

Retained-selector reconciliation preserves the exact engine error as well as success while journal recording retries. A metadata-only observer cannot reenter the engine. For an unchanged-resource atomic routing failure, the adapter can issue an opaque proof of no graph commit, bound to the original scope, operation and error. The host records that original failed attempt before freeing its pending parent; later work requires fresh current authority. Possible native-resource IO failures retain uncertainty and their original logical identity. Operation status, including the canonical HTTP operation query, reports the captured failed/unknown outcome; neither status nor this historical proof authorizes media. The isolated six-method/239-Store evidence is in `docs/evidence/gate-6/browser-client-reconciliation-outcomes.json`; current host checks pass all eight short browser baseline, four native and 28 transport methods. Sustained audio delivery, A/V alignment and impairment remain unresolved.

Managed audio mixes at 48 kHz and decodes consecutive original RTP intervals in order. After each output it retains the newest waiting original plus any remainder of the packet currently being played, preventing an initial burst from creating permanent queue delay. Its decoded PCM queue still holds at most 200 ms in ten chunks before insertion. Each output still checks current source permission and the original packet age and route; pauses and expired inputs cannot become delayed playback. Overlapping replacement packets keep the existing newest-input behavior. Evidence from the 71 focused mixer and scene tests is in `docs/evidence/gate-6/browser-client-original-pcm.json`; current host delivery and A/V alignment require their separate runtime checks.

For bounded server packet-age diagnostics, select one capture method with
`--diagnostic60 --packet-trace summary --case <capture-method>`. This uses the
same 60-second source and observer, but records a diagnostic result separately
from `--clean60`. The summary caps metadata buckets at 128, seconds at 121 and
age histograms at 21 bins; it writes once on test-guard drop and reports dropped
events. It includes setup and teardown and cannot prove complete IO or cleanup.
The current diagnostic reproduces sustained audio failure with queued server
audio near its original 200 ms expiry; browser queues average 11–30 ms. See
`docs/evidence/gate-6/browser-client-audio-age.json`.

After a scheduler stall, the managed mixer advances its 20 ms cadence and RTP
clock by the elapsed intervals. It discards only the corresponding missed PCM
quanta while preserving the newest available quantum after an idle gap. Each
output retains the existing original-age and current-authority checks. The
controlled stall, recovery and regression evidence is in
`docs/evidence/gate-6/browser-client-mixer-cadence.json`; connected sustained
audio and alignment still require their own measurements.

Rehearsal executables are sealed once per content digest and executable mode
under ignored `.local/gate-6/sealed-executables/`. Runs link only independently
copied, verified sealed bytes. Cargo outputs remain separate so a later build
cannot rewrite the sealed executable. Corrupt entries refuse admission; source
and executable digest checks still cover each run.

The waiting-packet policy passes 76 focused Core mixer/scene/cadence checks,
including ordered two-packet bursts, long packet remainders, fresh recovery and
current authorization. These component results do not qualify sustained browser
audio or A/V alignment. See
`docs/evidence/gate-6/browser-client-audio-backlog.json`.
