# Native media negotiation profile

This local Gate 6 profile composes the public `connection.offer` and
`connection.answer` families with the retained native Core stream controller.
It describes prepared bindings and endpoint parameter confirmation. A successful
confirmation is not a public ready event, Participant admission, Store commit,
durable operation receipt or observation of media flow.

The base protocol is pinned in `.deps/rvoip/docs/CONVERSATION_PROTOCOL.md`,
sections 7.4 and 10.1. Section 7.4 distinguishes the established media plane
(`connection.ready`) from observed per-stream frames (`stream.opened`). Neither
fact is emitted by this preparation exchange. The optional [native lifecycle observer](native-lifecycle-profile.md) separately
requires actual per-binding native flow and fresh event authority before emitting
readiness or Stream events. Production presence and general subscription
orchestration remain unfinished Gate 6 work.

## Capabilities and selection

Audio/video codec entries use `{ "name": "pcmu", "params": { "sample_rate":
8000, "channels": 1 } }`. Known standard defaults are PCMU/PCMA at 8000 Hz mono,
Opus at 48000 Hz mono and VP8 at 90000 Hz with zero audio channels. Explicit
parameters must fit their declared numeric range before conversion. Unsupported
encoding options cannot silently fall back to another encoding. RTP payload
types are hop-local; the selected native type is explicitly announced in each
answer. Current native backends support PCMU, PCMA, mono Opus and VP8.

Each offered logical stream selects the first compatible entry in its own
`codec_preferences`. The format must belong to every applicable local and remote
directional capability set. Directions describe the emitting endpoint:
peer `sendonly` creates a local receive stream. Bidirectional streams require
both intersections. Video support alone does not grant local screen support.
Reliable data explicitly selects `text`, `json` or `binary`; no unreliable data
transport is inferred from RTP availability.

The optional `capabilities.native_media` object has `version: 1` and `send` and
`receive` arrays. Each entry has a `kind` and `formats` using codec entries above.
Those formats must also belong to the advertised audio/video/data catalog.
Missing directional extensions use the advertised catalog in both directions.
Capabilities and claimed identity assurance are not authorization. The host
checks actual authenticated assurance and current relationship/media authority.

Unsupported new logical streams may be rejected while compatible streams
proceed. Zero compatible streams refuse the operation. An unsupported
replacement of an active logical stream refuses the entire change. A failed
change preserves the previous committed graph. `inactive` direction currently
refuses explicitly; public pause/resume orchestration remains unfinished.

## Prepared bindings and parameter confirmation

The server prepares the complete RTP/data batch before sending one correlated
answer. Its `substrate_setup` is:

```json
{
  "kind": "native-prepared/1",
  "revision": 1,
  "offer_id": "source-offer-id",
  "streams": [
    {
      "strm_id": "audio",
      "kind": "audio",
      "direction": "recvonly",
      "stream_local_id": 1,
      "codec": {
        "name": "pcmu",
        "params": {"sample_rate": 8000, "channels": 1, "payload_type": 0}
      }
    }
  ]
}
```

The server then sends one `connection.offer` callback with the same prepared
setup, conversation, Session, Connection and logical operation. The peer
validates and installs the exact provisional handle/codec mapping before
returning a signed, correlated `connection.answer`. Its setup must exactly match;
its `streams_answered` must match the offered stream list in order, with the
peer's inverse directions and exact selected codec parameters.

The actor checks correlation, physical peer, current credential, reply key and
scope under the current invocation's held authority. An endpoint assertion is
only a claim about those bound parameters. It does not grant media permission
or prove a working media plane. Mixed-family confirmation cannot complete before
both native families have prepared resources. Only successful confirmation
allows the local Core transaction to commit. Prepared handles are provisional:
precommit traffic cannot publish; rollback burns handles, retires staged
resources and preserves the previous graph. Retrying a failed attempt requires
new handles and the current stream revision.

Cancellation and deadlines retire the pending operation. Idle controllers retain
no invocation grant reader. Actual pending writer IO and reply verification own
their authority until completion/cleanup; cancelled queued copies cannot retain
independent readers. Media publication and forwarding acquire current authority
again, so a later denial takes effect even after successful negotiation.

## Checked datagram framing

QUIC and WebTransport media use the same eight-byte UCTP header and one complete
RTP v2 packet. The selected source-clock profile below additionally carries
bounded complete RTCP compounds in that wrapper. The physical stream handle
is nonzero; its authenticated route binding and lifetime are checked separately
from byte parsing. The outer 32-bit datagram sequence and inner 16-bit RTP
sequence are distinct. Both use network byte order and retain their full ranges.

The native checked APIs and the browser acceptance fixture validate the complete
RTP fixed header, CSRC list, extension block bounds and padding count before
returning a codec payload. Known one-byte and two-byte extension layouts also
validate element bounds; other extension profiles retain an opaque bounded
block. Header extensions and padding are excluded from the normalized payload.
The browser decoder handles byte-array views with a nonzero backing-buffer
offset. Parsed source metadata is not authenticated attribution or authority.

The pinned upstream `cross_runtime_media.json` corpus has eight valid and 31
invalid vectors. `scripts/check-gate6-media-framing.py` executes the production
Rust checked APIs and the actual browser fixture module on Node, including
lossless Rust serialization of the valid vectors. The corpus covers endian and
identity boundaries, CSRCs, extensions, padding, truncation and invalid handles
or versions. Payload bytes are test markers; these checks do not establish
codec decoding, live browser/WebTransport networking, datagram MTU or media
fragmentation behavior. Those require their separate runtime profiles.

## Source-clock transport

The signed parameter capabilities advertise `source_clock: "rtcp-sr-sdes/1"`.
A browser only emits or consumes clock reports after that profile is selected;
unknown selections and changes to an installed route refuse explicitly. Timing
availability is independent of codec/transport availability and media authority.

Clock datagrams retain version1, zero flags, the current nonzero allocated handle
and the outer datagram sequence. Their body is one SR-first RTCP compound with
one matching SDES CNAME. The body is at most1180bytes and16members, fits the
actual negotiated datagram MTU, and uses the frozen payload profile's RTP/RTCP
packet-type discriminator. Missing/duplicate/malformed identities and BYE as a
live clock sample refuse. Unknown bounded members invoke no feedback or control.

Current ingress direction, binding, policy and scene/consumer guards precede
clock-state mutation. A report cannot enqueue media, imply readiness or hold idle
authority. One Connection normalizes a common CNAME/NTP reference; individual
stream/SSRC incarnations retain distinct source IDs and weak liveness. Original
packet admission age and current authorization remain separate from capture time.

Sender reports sample send time and the corresponding RTP clock value, with
packet/octet counts from actual successful RTP submissions. Destination streams
share one CNAME; report writes do not count as RTP or media flow. The native
packetizer emits reports only during an actual guarded media send, at most once
per second. No idle reporting task exists. Clock identity/generation/support changes and loss of an available anchor
retire the old egress SSRC, so an unavailable mapping cannot inherit its
receiver's earlier anchor. Acquiring the first anchor preserves the current
SSRC/sequence and accumulated sender counts, including an in-progress VP8 frame. Frame slicing and nominal resampling
preserve truthful source timing; unknown or unaligned mixes stay explicit.

The timing correspondence and CNAME roles follow
[RFC3550 sender reports](https://www.rfc-editor.org/rfc/rfc3550.html#section-6.4.1)
and [CNAME](https://www.rfc-editor.org/rfc/rfc3550.html#section-6.5.1).
Current component evidence is in
`docs/evidence/gate-6/browser-client-source-clock.json`. It does not qualify
physical audio/video synchronization, codec filter delay, a live native SDK
renderer or the full operating profile. Shared receiver scheduling remains open.

## Feedback belongs to the registered payload

A NACK/RTX cache belongs to the registered primary codec's SSRC and payload
type. Sharing that SSRC does not give telephone-event or an unregistered payload
the primary codec's feedback contract. Those original packets continue through
the interceptor unchanged, but cannot enter that primary retransmission cache.
A changed codec binding must register its new payload type before receiving its
own feedback behavior.

The standard WebRTC audio profile advertises TWCC only. The explicit registered
Opus/NACK regression tests the interceptor's supported registration API with
payload 111 and auxiliary payloads 101, 110 and 126, plus unregistered payload
127, both with and without RTX. It does not claim a live browser negotiated that
optional audio feedback profile. Existing primary repair and source-clock
feedback remain separate regression checks. Current permission through actual
WebRTC driver IO is still required for guarded signal routing.

## Opt-in native video epochs and feedback

The candidate `rtcp-pli-epoch/1` profile is negotiated separately through the
`video_feedback` capability in the authenticated offer and signed parameter
confirmation. It is disabled by default. Omitted confirmation cannot activate
an offered feedback profile. Existing sender-clock negotiation supplies no
feedback permission.

The signed `stream.epoch` lifecycle notice describes an actual ordinary packet,
not readiness or permission. Its envelope carries the exact Conversation,
Session and Connection. The payload binds `original_operation` and
`original_offer` (the `native-prepared/1` offer that allocated this resource),
current authenticated-writer `incarnation`, `strm_id`, `stream_local_id`, `stream_generation`,
`connection_generation`, monotonic `revision`, `direction` (`publisher` or
`receiver`), opaque `epoch`, actual `ssrc`, `first_sequence` and
`first_timestamp`. Incarnation and epoch are 32-character UUID/u128 hexadecimal
identifiers. Sequences and timestamps retain the original unsigned 32-bit native
and RTP clocks. A reselection can change epoch and first sequence while retaining
the same physical handle, SSRC and stream generation.

Notices use the existing owned native lifecycle writer and current original
physical-source authority. They may precede connection readiness so a receiver
can bootstrap without waiting for unrelated streams. Retained registry entries
contain only bounded metadata and weak generation predicates. The current
binding and epoch are rechecked before reliable submission. Consumers must check
the signed physical/operation/binding scope, reject stale notice revisions, and
classify packet boundaries without reusing decoder history from a prior epoch.
Receipt of a number never authorizes feedback: the native adapter separately
checks the physical handle, emitted SSRC and current selected route, then obtains
both endpoints' current permissions before dispatching to the publisher.

The signed epoch component has focused local ARM transport evidence. Production
host enablement, SDK notice ordering and source-worker feedback remain pending;
it does not qualify G6-058 media quality.

### Selected video allocation metadata

When both peers explicitly select `rtcp-pli-epoch/1`, each Video entry in the
signed `native-prepared/1` setup includes a strict `video_feedback` object with
`profile`, `original_operation`, `original_offer`, `stream_generation` and
`connection_generation`. Positive generations fit the JSON safe-integer range.
The confirmation echoes the complete setup exactly. Advertising a capability
without this selection does not enable feedback. Audio, Screen and Data entries
do not carry this object.

New allocations bind the operation and offer that actually prepared them. A
later offer reusing an allocation retains all five fields; its new top-level
`offer_id` cannot overwrite the allocation origin. Feedback-profile changes on
reused video allocations refuse before parameter exchange. Actual replacement
allocations receive new origins. Signed epoch notices must match these selected
fields, the current physical client's roots and handle, and monotonic notice
ordering. The writer incarnation can rotate during credential refresh; it is not
a substitute for exact allocation provenance or the owned physical channel.

The SDK preserves this metadata through initial negotiation, codec changes,
stream selection and typed Node/Python media descriptions. This addition is pending its
new focused signed allocation/SDK checks. It does not itself enable feedback or
admit video to a decoder.


### Receiver video packet epochs

The explicitly selected `rtcp-pli-epoch/1` profile uses a distinct receiver-video
datagram form: native version `1`, flags `0x01`, the existing 16-bit handle and
32-bit datagram sequence, a 16-byte big-endian nonzero route epoch, then the
original complete RTP body. The tag adds 16 bytes to the existing native header;
packetization subtracts all 24 framing bytes from the transport MTU. The tag
changes no RTP timestamp, payload, marker or source clock. RTCP payload counters
exclude the extra framing.

This form is server-to-receiver Video only. Publisher ingress, Audio, Screen,
Data, clock-only reports and unselected peers keep their existing formats. The
ordinary native media parser refuses nonzero flags; callers must use the
explicit epoch-video parser and validate the selected profile. Native feedback
PLI remains its separate 48-byte epoch-bound form.

Core issues the route epoch inside the same current delivery permission interval
that submits the packet. Feedback endpoint installation and the signed notice
still follow successful actual submission. A packet can arrive before its
signed notice: its in-band epoch prevents silent adoption into a preceding
history that reused the same handle and SSRC. The client must match the tag,
SSRC, sequence boundary and original allocation against the signed notice before
decoder admission. Pending packets retain their original arrival age and must
expire under the existing200ms bound. A tag alone is never permission or a clock.

The packet syntax and signed native transport cases pass on local ARM64 at
`b7e5b97f`. See `docs/evidence/gate-6/native-video-packet-epochs.json`. The
production profile remains disabled pending end-to-end SDK and encoder recovery.


### SDK notice and packet join

The owned Rust SDK reader consumes `stream.epoch` only after the existing
physical peer signature verifier. It matches logical roots, selected original
operation/offer, handle, stream and connection generations, direction and a
monotonically newer notice revision. Incarnation and epoch must be nonzero
canonical 32-character lowercase hexadecimal values. Incarnation identifies the
current authenticated writer and can change on credential refresh; it is not
used as an immutable substitute for the owned physical peer.

A notice for a confirmed but not yet adopted selection is retained as bounded
metadata on that preparation. It cannot create an inbox or admit media. The
existing paired resource-capture path installs it only after resource adoption;
failed or superseded preparation drops its metadata.

Explicit receiver video accepts only tagged RTP. The tag, actual SSRC, serial
native sequence boundary and exact first-packet timestamp must match the signed
notice before delivery. A future unknown tag waits in the existing bounded
queue with its original arrival time. It expires at 200 ms even if no notice or
new packet arrives. A late notice does not restart this deadline. Local receive
hold invalidates active history while retaining its revision, so replay of the
same notice cannot reactivate it. Changed allocations discard old history.

`RtpPacket::video_epoch()` exposes `VideoReceiveEpoch` only after this join;
ordinary RTP returns `None`. Metadata never changes RTP bytes or timestamps and
never authorizes feedback. These SDK changes have focused checks pending; typed
publisher feedback, helper/browser/direct-peer integration and real decoder
bootstrap/fragment-loss recovery remain required before production opt-in.
