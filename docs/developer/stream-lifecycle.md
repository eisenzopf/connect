# Negotiated stream engine model

G6-020 is in progress. The recorded rvoip patch adds `stream_lifecycle` to the
engine. It is a process-local model for one exact Connection generation. It does
not perform adapter calls, commit a media graph, authenticate a request, persist
an operation or expose a new Connect HTTP operation.

`EndpointMediaCapabilities` declares send and receive formats independently for
audio, video, screen and reliable ordered data. Screen capability is explicit;
video support alone does not imply permission or ability to capture a screen.
Each requested direction requires the corresponding sender/receiver intersection.
Codec name comparison ignores case; clock rate, channels and format parameters
must agree. RTP payload types and datagram handles are transport bindings and
do not determine codec compatibility. There is no inferred transcoder.
One logical stream has one codec; a bidirectional stream requires a common
format in both directions. Asymmetric codecs use separate send and receive
logical streams with their respective compatible formats.

All requested streams are required. A heterogeneous peer can explicitly request
only its usable audio format. An unsupported audio/video request returns an error
before the committed stream snapshot changes. Endpoint capabilities are bounded
to four media kinds per direction, 32 formats per kind and 32 streams per
Connection. The owning coordinator must advertise only actual adapter support.

`StreamLifecycle::prepare` checks current membership, Session appearance,
verified Endpoint, active Connection generation, explicit control/send/receive
grants per medium and expected revision. `CurrentMediaAuthority` is supplied by trusted
policy under its current-authority lease. Its grant sets must never come from a
request body; roles and capabilities do not grant authority. Current credential
validation and policy installation remain responsibilities of the runtime owner.

One preparation can be pending for at most 30 seconds. It changes no committed
streams. Abort leaves the snapshot intact. An expired or foreign ticket cannot
commit or abort a later preparation. Any intervening pause, end or committed
change advances the revision and fences the prepared snapshot.

Commit requires all exact selected formats and directions, the captured
Connection generation, unique nonzero 16-bit transport-local IDs, and each required
source/sink readiness observation. Readiness values are protected adapter facts,
not client assertions. Authority is checked again at commit. The model defaults to at most 16 cumulative transport bindings per Connection.
Unchanged bindings consume no additional budget; replacements do, and logical
end/pause never refunds history. `with_open_limit` accepts an explicit limit
from 1 through 4,096; `opened_stream_limit` and `opened_binding_count` expose
the effective values. Logical tombstones are also bounded by that limit.
Exhaustion fails explicitly rather than evicting a fence. The physical peer's shared handle
allocator must additionally enforce nonreuse across all its Connections.
The 16-bit type matches the approved eight-byte UCTP datagram header; an external
JSON integer must pass a checked conversion before becoming a ready binding.

A replacement can retain a logical stream ID while receiving a new transport
stream ID and local handle and incrementing its stream generation. Old end or
readiness callbacks cannot affect that replacement. A completed logical stream
is terminal. Downgrades retain tombstones for removed streams, and changing one
stream leaves other committed streams intact. Pause/resume and explicit end are
currently model changes only.

The owner must next integrate these decisions with invisible adapter preparation,
atomic graph activation/retirement under current policy, cancellation and uncertain
cleanup, current audience-bound subscriptions, public UCTP updates and attributed
observations. Existing raw renegotiation helpers must not bypass those checks.
Real heterogeneous media, physical pause/resume, permission-denied capture,
overlapping adapter negotiation and stale transport readiness still need behavior
evidence. Screen/data media, providers and browser/native profiles remain
unqualified by this model. The scoped checker is
`scripts/check-gate6-stream-model.py`; its passing report does not close G6-020.

The current default-budget qualification is described in [`secure-profile.md`](secure-profile.md); the earlier model evidence retains its original source fingerprints.
