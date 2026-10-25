# Canonical browser client (G6-050 in progress)

`@rudeless/connect/browser` exports `BrowserClient` for the canonical
`connect/2` WebTransport control path. It requires a secure context, WebTransport
and WebCrypto. The current checkpoint implements signed control, RTP and
WebCodecs primitives; capture, connected media/rendering and the complete media
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
submitted media. Queues drop excess work; a receiver waits for a fresh keyframe
after declared loss. `requestKeyFrame()` lets the owning media path request
recovery after dropping output. Decoded callbacks borrow the frame synchronously;
the codec always closes it, including on callback failure. `RtpClock` unwraps
RTP timestamps; it alone does not establish cross-stream A/V synchronization.

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

The full G6-050 completion still requires identified duplex Opus/VP8/data
between browser/browser and browser/native, capture/permission and unsupported
API/codec refusals, background/resume, impairment recovery, and owned teardown.
Transport availability and these control/RTP checks do not qualify those paths.
