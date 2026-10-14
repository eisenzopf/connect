# connect-video-vp8/1

Gate 5's explicit two-party encoded-video adapter profile is under qualification.
It extends the accepted Connect Session's media to exactly `audio` and `video`;
ending a live Connection preserves that Session's parent Conversation and context.
See [decision 011](../docs/decisions/011-gate-5-profile.md) for frozen budgets.

## Native wire and SDK

Native control uses stock reliable UCTP session/connection/stream envelopes.
`connection.offer` supplies exactly one PCMU sendrecv audio stream and one VP8
sendrecv video stream. The coordinator admits current Session authority before
allocating the connection. One participant has at most one active AV connection
per Session. A rejected/partial negotiation has an explicit error and bounded
cleanup; it cannot become an audio-only success.

Each native media datagram is the existing eight-byte UCTP header followed by
one complete RTP packet. Audio's internal `MediaFrame` payload remains codec
bytes. Video's internal `MediaFrame` payload is complete validated RTP, retaining
sequence gaps, timestamp, marker and the VP8 payload descriptor. This adapter shape
is not a replacement UCTP wire version. The native profile uses PCMU PT 0/8 kHz
and VP8 PT 96/90 kHz. Local routes and stream handles come from authenticated
`stream.opened` events. New code advertises complete negotiated CodecInfo; a
fixed-name legacy descriptor must still agree with this profile.

The Rust `NativeVideoSession` owns separate bounded audio/video receivers and
outbound streams. `send_vp8` fragments one encoded frame with a valid VP8 RTP
descriptor and final marker. `VideoFrameAssembler` yields complete encoded frames
only: sequence loss, duplicate discontinuity, overflow and late partial assembly
never become a claimed complete frame. Native audio/video use a shared monotonic RTP epoch; resuming a muted or stalled
source does not restart that stream’s clock. The caller selects an independent codec.
A missing first keyframe waits for a later keyframe; packet receipt is not decoding.

Native publisher IDs are `audio-{identity_id}-{session_id}` and
`video-{identity_id}-{session_id}` in this profile. The public SDK's participant
subscription helper applies those IDs through authorized UCTP subscriptions.
They identify streams; they do not confer permission. Browser/native direct
bridges require no native subscription. Stale streams are not recreated on replay.

## Runtime projection lifecycle

The durable accepted Session owns its disposable Core projection. The projection
has an explicit external-origin marker; transient loss of its final Connection
removes subscriptions and releases active-session accounting while keeping it
idle for a fresh authorized attachment to the same durable IDs. Ordinary Core
and SIP-created Sessions retain their original last-connection auto-end behavior.
An application cannot adopt an ordinary Session or revive an explicitly ended
projection by reusing its ID. Current credential and membership checks still run
for every replacement attachment.

The process retains at most 10,000 external projection IDs, including terminal
tombstones; exceeding that bound rejects admission before allocating a new
Conversation. This matches the default durable Session row budget. The server
reconciles durable terminal or closed/absent Session state at its one-second
media-authority tick, including idle projections without sockets, and explicitly
ends Core projections to release their subscriptions and active resources.
The journal and parent Conversation remain the durable source of context.

Shutdown starts native physical drain with intake shutdown. Independent WebRTC
routes clean up concurrently with owned joins, so incomplete peer count does
not multiply per-peer cleanup deadlines. Failed negotiation and failed shutdown
are separate evidence failures; neither is counted as media qualification.

## Browser boundary

The browser SDK uses authenticated WebRTC, including one live audio track and
one VP8 video track. It validates its loopback signaling URL and credential
before acquiring devices, restricts video codec preferences, and uses separate
camera/microphone muting. Capture, signaling failure and hangup release owned
tracks. The configured accepted video route admits only its current members;
ordinary audio routes keep their original exact-credential policy.

A video `MediaFrame` is validated and rewritten at an RTP hop boundary. Source
extensions and CSRCs are removed; negotiated payload type and the destination
SSRC replace hop-specific fields. Native source sequence and RTP marker are retained. Oversized browser VP8
packets use bounded sequence translation when split: missing input reserves an
output gap, duplicates reuse their mapping, and late unresolved packets drop.
A source packet larger than the native profile must be explicitly repacketized
before native delivery; it may not cause unchecked QUIC fragmentation.

Browser PLI/FIR requests target the exact local video SSRC and traverse an owned,
bounded bridge feedback worker. Requests are coalesced and limited to four per
second. Audio and unrelated track feedback cannot request a video keyframe.
Native senders currently use the profile's periodic keyframe requirement (at most
one second), rather than an invented UCTP feedback extension. Qualification must
report that distinction and independently measure loss recovery on each route.

## Limits and evidence

VP8/640×360, maximum 30 fps, target 1 Mbit/s; native RTP at most 1,180 bytes;
encoded frame at most 256 KiB; eight incomplete assembly frames, 64 reordered
fragments across them, and 64 queued video packets; stale packet/assembly
limit 200 ms. Audio retains its ten 20 ms frame buffers. A clean route must sustain
at least 24 independently decoded changing frames per second for 60 seconds,
with duplex identifiable audio. Teardown is bounded to five seconds. Loss,
reordering, duplication, stalls, credential revocation and restart need separate
behavior proof. An ignored external-codec/browser test does not qualify a route.

SIP dialing requires an accepted audio-only Session. An accepted audio/video
Session is rejected with `unsupported-media-profile` before a SIP leg can be
queued; it is never silently downgraded. A developer can explicitly create an
audio-only Session in the same Conversation to use the telephone channel while
retaining all shared context. Canonical rejected-operation receipts remain part
of the ordinary journal and do not represent a queued or dispatched call.

No H264/AV1 transcoding, SIP/PSTN video, Vapi vision, screen sharing, multiparty
video or media-byte MCP API is advertised. Read [the acceptance map](../docs/gate-5-acceptance.json)
before calling this profile release-ready.

Authenticated browser Session selection is additive to the existing configured
route. `browser_binding` and an initial WebRTC `offer-ready` may supply paired
`conversation_id` and `session_id` values. Each ID is nonempty, at most 128 UTF-8
bytes, and contains no whitespace or control characters. Both selected Session
participants must belong to the configured route pair, in either invitation
direction; the Conversation may be another currently permitted Conversation.
The Session must currently be accepted/ready with exactly audio and video, and
the configured credential and joining credential must both retain authorization.
The offer selection is an owned, redacted routing hint bound to the authenticated
connection. It does not establish authority. Wrong pairs, mismatched IDs,
pending/terminal Sessions and current revocation reject before media admission.
Omitting the selection retains the configured route default, including its
single configured audio credential. Explicit Session end is terminal; a new
accepted invitation can be selected without operator configuration or restart.

QUIC video egress uses the actual peer `max_datagram_size()` minus the eight-byte
UCTP header, capped by the 1180-byte complete RTP profile. A bounded VP8 packetizer
splits larger normalized RTP payloads, retains descriptors and final markers,
and translates sequences while reserving gaps for missing source packets. Known
duplicates reuse their prior fragments only while those fragments fit the current
budget; conflicting or unresolved late inputs reject. History is bounded to 64
inputs/200ms and a pathological peer budget cannot allocate more than 64 fragments
for one input. An MTU change racing an actual send remains an explicit Quinn
failure; it cannot be counted as delivery.

The existing authenticated `health` operation exposes optional
`readiness.media` process aggregates. Its static stage labels contain no identity,
Session, credential, address or media payload. Queue capacities come from actual
bounded channels; high-water values are **sampled at enqueue and pump poll**, not
exact event peaks. Raw WebRTC depth has `unit: track-events`; other stages have
`unit: frame-packets`. Per-queue fullness is not end-to-end packet loss, including
a native publisher's shadow input queue alongside Core fanout. Stale discard
counts are separate. Forwarded-frame age is sampled at dequeue before handoff or
transport write, with an explicit `frame_age_boundary`; it excludes remote
network/decoder/playout and time within a later write.

The native-to-native fanout passes the mux's original frame directly into the
subscriber's normal outbound queue. Its publisher `quic-video-ingress` queue is
an unconsumed shadow, so it has depth/fullness observations but no forwarding-age
sample. This route requires age observations from `quic-video-egress` and timing
samples from `core-video-fanout`. Native-to-browser requires forwarding ages for
`quic-video-ingress`, `webrtc-video-ingress`, `quic-video-egress`, and
`webrtc-video-egress`; browser-to-browser requires both WebRTC application stages.
Both browser routes require `core-video-bridge` timing samples. The separate
generation-fenced `quic-video-peer-egress` writer is unused by these AV paths;
raw track events and Quinn's opaque byte buffer do not report application-frame
forwarding age. Native SDK queue observations have their own client scope and
are not inferred from server Health. Its public handle snapshot samples both
native stream directions and the shared physical-peer byte buffer. The SDK
receiver discards video packets older than 200 ms before returning them to the
application; stale discards are separate from fresh forwarded-age observations.

Forwarding latency measures monotonic adapter frame publication (after raw RTP
poll) to successful Core destination queue handoff. Native fanout and owned video
bridges have separate stages. Two fixed 65-bucket histograms provide p50/p95 bucket
upper bounds and an exact maximum in microseconds. Missing receipt keys and
handoff timeouts are explicit counters, never synthetic zero-latency samples.
At most 512 private receipt keys survive for one second; snapshots include only
observed stages. There is no separate approved forwarding latency pass threshold.
QUIC aggregates report observed peer datagram/path-MTU ranges, attempted sizes,
actual `TooLarge`, `UnsupportedByPeer`, `Disabled`, `ConnectionLost` send failures,
and separate video budget/repacketization/rejection counts. Quinn acceptance does
not prove remote receipt. All three promised media paths still require independent
active-stage samples and actual decoded-media qualification.

Connect explicitly applies a 76,032-byte (64 × maximum 1,188-byte profile wire
packet) physical-peer QUIC DATAGRAM send buffer before endpoint/connection
creation. This shared audio/video buffer is a byte bound, not a 64-packet count
or a measured packet-age bound. `quic.send_buffer` reports the configured capacity,
sampled available-byte range/high-water occupancy and mismatches. A send larger
than the immediately sampled free space is separately counted as potential
pressure, not an observed eviction. Quinn's internal evictions and queued age
remain unobservable through this API (`evictions_observed` and
`queued_age_observed` are false). Per-frame fragment emission performs no extra
await beyond its owned pump; congestion may produce bounded local enqueue/loss
rather than invisible unbounded buffering. The actual decoded envelopes remain
required to qualify the resulting loss/recovery behavior.

An opt-in upstream host admission callback checks current authority before any
server PeerConnection/UDP media allocation on a new WebSocket offer. Connect
installs it for explicit selectors and omitted configured defaults alike. Selected
AV and configured default AV validate the current accepted Session/pair; audio
retains the exact configured credential. Callback denial is generic and discloses
no database details. Media ingress then consumes the owned hint and revalidates
again, so expiry/revocation between preparation and publication is not accepted.

The optional `webrtc_video_input` Health object exposes fixed numeric observations
before normalization: payload size, observed sequence skips/reordering, padding
and typed rejection reasons. A sequence skip does not prove network loss.
The `webrtc-video-reorder` queue is bounded at 64 packets with 100 ms gap-only
lookahead; in-order input is released immediately, and the 200 ms age ceiling
still uses original local receipt time. Valid padding-only RTP consumes a source
sequence without emitting a video fragment. Real missing media is never filled
with fabricated bytes. These source changes remain under qualification.
