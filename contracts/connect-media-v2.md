# Gate 6 negotiated media profile

G6-009 / G6-011. Target behavior; measurements are required before qualification.

## Negotiation

Capabilities describe permitted media kinds, send/receive directions, codecs,
screen/data support, transport and limits per endpoint. Negotiate the usable
intersection; transcoding is an explicit adapter capability, never assumed from
a generic audio/video flag. A capability mismatch rejects an upgrade before
disturbing existing media. Mid-session updates use expected resource revision and
current endpoint authority.

The initial qualification codec set is Opus mono 48 kHz for browser/native audio,
PCMU/PCMA 8 kHz for SIP/telephone, and VP8 for video/screen content. Opus/G.711
transcoding is qualified explicitly. Existing PCMU native fixtures remain useful
as a separately named negotiated path, not an API compatibility obligation.
App data is reliable and ordered; large files use reliable artifact transfer,
not media datagrams. Screen content uses a separately identified video stream
with explicit browser capture permission.

## Framing and route authority

Native QUIC uses reliable framed JSON control and datagram media. WebTransport
uses its reliable streams and datagrams. WSS fallback carries canonical control;
WebRTC carries fallback media. All paths use the same authorized session binding.

The datagram header stays the existing eight-byte version-1 format followed by
a complete RTP packet. `stream_local_id` is nonzero, physical-peer-scoped and
never reused during that peer's lifetime. It maps to an authenticated logical
Session, Connection, Stream and generation. Server fanout rewrites subscriber
handles and respects audience plus subscriptions. A revoked or retired binding
cannot forward using a stale handle. Header version and envelope version are
independent; no framing change is implied by envelope v2.

Malformed RTP, oversized datagrams, unknown/retired handles and codec mismatch
are rejected with bounded work. Keep codec/RTP fragmentation in the documented
profile; do not wrap codec-only bytes and claim RTP conformance. Reliable control
and observability backpressure must not delay media.

## Streams and readiness

Open/close, pause/resume, mute, subscriptions and capability changes are explicit.
An admitted participant does not automatically hear all streams. Audio/video/data
and screen can differ between participants. A telephone endpoint rejects video
without rejecting usable audio. Replacement creates explicit new transport
bindings even if the logical application stream remains related by lineage.

Media readiness means observed compatible bidirectional route readiness and any
configured endpoint confirmation, not an SDP offer or answer alone. Qualification
uses identifiable speaker/motion markers and decode/render observations. Hold
treatment cannot deliver private consultation media to the remote participant.

## Frozen measurement definitions

- Clean AV: 640×360 VP8 at up to 30 fps; at least 24 decoded fps for 60 seconds.
- Audio: 20 ms frames; Opus target 32 kbit/s mono; telephone G.711 uses its
  negotiated payload type and 160 samples per 20 ms frame.
- Video: target 500 kbit/s, maximum 1 Mbit/s per 360p sender; complete RTP
  packets are at most 1180 bytes excluding the eight-byte UCTP header. Encoded
  frames are at most 256 KiB. Incomplete assembly expires at 200 ms, holding at
  most eight incomplete frames and 64 fragments. Each video boundary queues at
  most 64 packets and drops media older than 200 ms; audio queues at most ten
  20 ms frames. Default per-peer datagram send buffer is 76032 bytes. Native
  examples must decode moving content; keyframes occur at most one second apart.
- Handoff/device interruption: at most 1 second between the last identified
  pre-switch sample and first identified resumed sample after replacement is
  ready, excluding user answer and configured confirmation time. Report both
  total transition time and audio gap; at least twenty transitions per topology.
- Teardown: within 5 seconds of terminal intent/observation; owned resources
  return to their idle state within 10 seconds. Late pending captures stop.
- Impairment: reproducible 3% packet loss and 100 ms delay variation, plus
  blocked preferred transport and explicit fallback. Degraded video is reported;
  privacy, authority, retained IDs, bounded work and teardown remain hard checks.
- Clean decoded A/V alignment: median absolute skew at most 80 ms, 95th
  percentile at most 120 ms, at least 42 uniquely paired markers in 60 seconds.
  Impaired alignment is an explicitly reported degraded-quality observation,
  never a fabricated synchronization claim. The impairment run includes two
  250 ms bursts and fresh decoded motion recovery within two seconds.
- At least four Participants, six Connection slots including pending candidates,
  and two concurrent Sessions. Limits are configurable and reject overload
  before external effects; this is a tested minimum, not advertised scale.

Measure server, native helper and browser process groups separately. Report CPU,
resident memory, queues and open resources for the declared workload. Fix limits
before the corresponding run; never rewrite a failed threshold after observing
a failure without a reviewed change and a new identified qualification run.
