# Native DTMF and quality signals (G6-024, in progress)

The approved protocol is `.deps/rvoip/docs/CONVERSATION_PROTOCOL.md` sections
7.5 and 10.3. DTMF and structured quality travel over the authenticated signaling
channel. Native datagrams carry complete RTP media, not RTCP or invented data
messages. This document records the current engine component; it does not close
G6-024 or qualify a live browser, gateway or provider deployment.

## DTMF ingress

`CanonicalNativeAdapter::receive_native_dtmf` consumes the actual
`AuthorizedEnvelope` for one current native physical peer and its retained
negotiated Connection. It checks the concrete peer, credential incarnation,
Conversation/Session/Connection wire binding, signed resource confirmation,
current audio bindings and original logical operation ID. A pre-negotiation
request, foreign physical peer, stale credential/revision or mismatched scope
cannot borrow another Connection's source attribution.

The resulting `NativeDtmfObservation` is privately constructed from those actual
engine facts. It records the Core relationship IDs, membership epoch, verified
Endpoint/device, native transport, current stream/credential revisions and audio
binding generations. Its receive time is observed locally. Its source is
`uctp-native`; an optional input `payload.source` or claimed receive timestamp is
ignored. The request's `method` expresses the requested gateway method; it does
not assert that native ingress arrived as SIP INFO or an RTP telephone event.

Methods are `rfc4733` and `info`. Whole-sequence validation reuses the existing
media engine validator before invoking any observation sink: at most 32 valid
DTMF digits, 40–6000 ms per digit and at most 30 seconds including the existing
70 ms interdigit schedule. These are the existing gateway scheduler's bounded
technical limits. This ingress path does not generate tones or wait for the
sequence duration.

A separately supplied `NativeDtmfPolicy` must derive an owned current signal
permission from the actual invocation's held domain. It must bind the complete
source/operation/intent and current relationships, reuse that same guard without
reader reentry behind a queued writer, and retain no idle permission reader.
Ordinary media negotiation/routing authority does not imply DTMF permission.
The default signal policy and default `NativeDtmfSink` both return Unsupported.

A sink makes one bounded synchronous local observation decision while the
current grant is retained. It may record/journal or enqueue metadata; it cannot
reenter engine state or detach an authorized side effect. Later delivery,
endpoint confirmation or other effects need their own current authority and
actual generation fences. The observation contains no grant. The host preserves
and reconciles the original logical operation ID in its authoritative journal.
There is no automatic retry, redial, tone generation or assumed downstream
success. A fixture ACK explicitly labels this `local_dtmf_observation`; it is not
a durable Connect receipt, public readiness or Participant presence.

## Native egress

`native_dtmf_registration` seals the recipient's own authenticated physical
origin after its signed resource negotiation. An ingress request from another
peer cannot create that origin. Idle registrations and source observations hold
metadata and weak engine references, with no current authorization reader.

`NativeDtmfRegistration::prepare_delivery` requires a separate
`NativeDtmfEgressPolicy` for the exact source operation and recipient. Its scope
includes the recipient relationship, current credential/stream revisions and
actual audio binding generations. It must revalidate both endpoints and current
source/recipient permission; ordinary routing permission is insufficient. The
default is Unsupported. An actual supplied held domain must be reused without
reader reentry, not replaced by a guessed credential or another guard.

One active write owns a bounded queue reservation and a one-use current grant.
The submission must be retained and awaited or dropped to cancel. Queued copies
contain a cancellable grant slot rather than independent readers. Source and
recipient physical/credential/media fences remain current through admission and
actual writer IO. A bounded watcher cancels retired sources/recipients, changed
media, caller cancellation or deadlines. Actual IO retains its guard until its
sink has completed or been aborted. Idle metadata retains no grant.

The frame is `dtmf.received` in the recipient's own wire Conversation/Session/
Connection scope, signed by its actual physical actor. It preserves the original
logical operation ID, validated digits/duration and observed source time. Local
writer completion reports only signed transport submission. It does not prove
that a remote device consumed the digits or establish a durable Connect receipt.
Uncertain IO must be reconciled under the same operation; it never causes
automatic redial or a second native delivery. The host owns operation deduplication.

The SIP gateway's shared `DtmfTransmitter::send_sequence` now keeps each tone
scheduler inside the owned sequence future. Dropping that future stops the
current scheduler and all later digits. Already admitted RTP packets cannot be
recalled; cancellation does not assert that no digit reached the remote end.
The existing RTP scheduler, duration/sequence limits and requested timing are
reused rather than duplicated.

## SIP receive timing

The SIP public `Event::DtmfReceived`, endpoint event, stream receiver helper and
callback interfaces preserve `duration_ms: Option<u32>`. `Some` carries a receive
measurement; `None` marks an incomplete digit observation from a source that
provided no timing. The Core adapter forwards measured durations unchanged and
refuses to turn an incomplete observation into an automatic tone effect. It
never substitutes a 100 ms receive measurement. The callback and stream APIs
change together; there is no old-signature adapter.

The local RTP receive lane uses its negotiated telephone-event profile (PT101,
8000 Hz). The existing media engine's observed duration passes through the exact
session publisher and current SIP-to-Core route. Final retransmissions remain
one event. This lane does not establish support for arbitrary telephone-event
payload mappings/clocks, provide a durable logical operation receipt, or seal a
future forwarding grant.

For SIP INFO `application/dtmf-relay`, both one `Signal` and one explicit
`Duration` are required before creating an effect-capable Core DTMF event. The
existing shared whole-sequence validator checks the digit/duration bounds,
with the INFO-specific 100–5000 ms decimal body range. Missing, duplicated,
negative, signed, overlong or out-of-range durations are refused rather than
inferred or clamped. This bounded body profile follows the explicit Signal and
Duration form in the [IETF DTMF INFO-package draft, section 5.2](https://datatracker.ietf.org/doc/html/draft-kaplan-dispatch-info-dtmf-package-00#section-5.2);
that expired draft is not a claim of full INFO-package negotiation support. The actual in-dialog request owns its response and the
current exact route owns its local observation. Explicit Core routing now uses
method-specific INFO authority and the canonical owned transaction; sealed
native observations can select that same INFO path under fresh independent
source/recipient signal permission. Received SIP/WebRTC composition follows the source-registration contract below.
Production journal installation remains required.

## Measured native quality

The retained native observation registration emits `connection.quality` through
its authenticated signaling writer. The default cadence is five seconds.
`interval_ms` is the actual elapsed monotonic measurement window, including
scheduling delay, rather than a claim that a timer fired exactly on schedule.
Windows begin at confirmed resource negotiation, successful gate/resource
changes, or an authorized cadence update. They contain only current retained
RTP bindings. An idle negotiated Stream may report measured zero traffic without
announcing `connection.ready` or inventing remote playout.

For each logical RTP Stream, `packets_received` counts actual locally accepted
native ingress datagrams; malformed, wrong-codec, foreign, retired or rejected
packets do not enter that counter. `packets_sent` counts successful local
`send_datagram` submissions individually. A later fragment failure cannot erase
an already submitted prefix. Counts include admitted retransmissions and are
not a unique-sequence or remote-consumption count. `bitrate_bps` is the integer
aggregate of accepted ingress and successfully submitted egress UCTP datagram
bytes (including the UCTP/RTP headers), multiplied by eight and divided by the
same observed interval. Each physical Connection and actual binding generation
has its own measurements, even when logical Stream names repeat.

`loss_pct`, `jitter_ms`, `rtt_ms` and `mos` are optional measurements. They are
omitted when this observer has no measurement; zero is not an unavailable-value
sentinel. Native normalized frames do not justify inferred packet loss or MOS,
and aggregate physical-peer transport statistics are not attributed to one
logical Connection. Reliable data messages are not counted as RTP packets.
This component does not qualify provider quality estimators or interop RTCP.

A signed `connection.update` with `action: "quality"` and a
`quality_interval_ms` in 100–60000 tunes one Connection. It requires the current
media `expected_revision`, original operation ID and exact native physical/wire
scope. It cannot also contain media gates, codec changes or arbitrary details.
The separate `NativeQualityConfigPolicy` defaults to Unsupported and derives its
grant from that actual invocation's held domain without reader reentry. This
observer configuration preserves the negotiated media resources and their
revision; the host journals/reconciles the logical configuration operation.

Every report needs fresh exact `NativeLifecyclePolicy` permission for its typed
`ConnectionQuality` event and current recipient credential/key. Idle timers hold
no reader. The existing bounded one-use grant, authenticated writer and actual
IO custody are reused. Reconfiguration or resource retirement invalidates queued
old intervals and releases their reporting grants. Refused/uncertain reports
are not replayed; a later window needs new current permission. Quality is a hint,
not an authoritative operation receipt, media-readiness assertion or presence.

## WebRTC RTP observations and owned scheduling

The receive decoder retains actual SSRC, negotiated payload/clock, original and
last-segment RTP timestamps, an extended timestamp within this physical pump,
event code and sample duration. Integer milliseconds use ceiling conversion;
raw samples and clock remain available for exact interpretation. A missing
observed beginning stays explicit. These are wire facts, not credentials,
current permission, a durable operation ID or remote-consumption proof.

Replay state holds at most 64 sources per physical track and one current event
per source. It never evicts a source to make an old replay new. Timestamp wrap
extends the local identity; late, ambiguous backward-clock and conflicting
events are refused. A source-clock restart requires a new actual track/decoder,
not a caller readiness flag. Ambiguous negotiated mappings are unavailable.
Packed payloads contain at most 32 events, are fully validated before any
prefix is returned, and retain each derived contiguous timestamp. Known
65535-sample segments reconstruct one event with its original identity. The
reserved RTP event R bit is ignored as required by
[RFC 4733](https://www.rfc-editor.org/rfc/rfc4733.html#section-2.3.3).

The bounded DTMF observer queue uses nonblocking admission. Its local drop
counter is separate from audio loss and provider quality; slow/closed observers
do not stall the primary audio pump. No tone is retried to compensate for a
missed observation.

Sending validates the entire intent using the shared Core/media-engine bounds:
32 digits, 40–6000 ms per digit, a 30-second complete schedule and 70 ms gaps.
An invalid suffix cannot follow a submitted valid prefix, and whitespace is
not silently removed. Exact negotiated sample counts replace duration clamping
or tick rounding. Long high-rate events use the RFC 4733 segment timestamps;
only the final segment ends the tone. Six seconds at 48 kHz remains 288000
samples, rather than being shortened to one 16-bit segment.

The schedule stays in the owned request future. Dropping it stops the current
scheduler and suffix. Already admitted RTP packets remain historical effects,
and local completion never promises that a remote party consumed a digit.
The general gateway integration still needs exact current source/recipient
signal authority with bounded per-write custody; it must not retain an idle
authorization reader for an entire multi-second sequence.

Primary audio uses the same typed unbound-track error as the stack RTP writer.
Binding retries remain in the owned send future and share the media pump's
bounded write deadline. A stopped track or retired source clock refuses the
send; dropping the future releases the wait. A missing negotiated MID alone
does not reject primary audio or a telephone event. An expired binding wait is
a local failed submission, not a reason to replay a DTMF operation.

## Fresh per-packet RTP write permission

The shared `DtmfWriteAuthority` contract is an opt-in transport custody primitive.
A trusted policy instance must bind the exact source and recipient, physical and
resource generations, complete intent and original logical operation. Each
`DtmfWriteScope` supplies the actual RTP event, clock, SSRC, payload, timestamp,
duration, marker and digit index for that write. These facts carry no grant.

`DtmfTransmitter::send_sequence_authorized` validates the complete intent before
scheduling and obtains a new owned permission for every actual UDP/RTP write,
including every final retransmit. The default policy refuses. It uses the
session's existing ordered RTP writer without holding the outer session mutex
while acquiring permission. A permission retains the actual host/store guard;
no reader spans a packet tick or interdigit gap. Its nonblocking validity check
must include current source/recipient lifetimes and generations.

Acquisition and actual write custody share a 200 ms bound, shortened by the
policy deadline. During pending IO, validity is checked at 5 ms intervals. The
owned IO future is completed or dropped before releasing its guard. Caller
cancellation drops both. A refusal or acquisition timeout describes that packet
before submission; earlier packets in the operation may already have effects.
Any failure, retirement or timeout after IO starts has typed `OutcomeUnknown`
semantics and requires reconciliation of the original operation, never an
automatic resend or a new logical operation. The sequence API tracks successful
local packet submissions: once any prefix has been submitted, a later failure
returns `OutcomeUnknown` for the complete sequence, including refusal after a
completed digit during an interdigit gap. A later refusal cannot erase those
historical effects. Success still means only local IO, not remote consumption.

Core routes explicit immutable DTMF intent under independent source/recipient
signal permission and exact current lifecycle metadata. Raw adapter events no
longer auto-forward through media bridges, and target-only Core `send_dtmf` is
removed. Sealed native observations bind their original operation and actual
registered adapter to that route; each physical write also fences the observed
native stream, credential and flow epoch. No observation is a stored grant.
The host journals/deduplicates the original operation and supplies current
signal authority for every write. Received SIP/WebRTC composition is described
below; production installation remains separate work.

## Owned WebRTC DTMF submission

`send_dtmf_authorized` uses the negotiated audio sender and shared whole-intent
validation. Its queued request contains packet facts, cancellation state and a
trusted authority callback; no permission reader is retained in the queue. The
driver validates the actual sender, final payload mapping, clock, connected
transport and supported interceptor profile before asking for permission.

Support defaults to false. The reviewed default interceptor composition opts
in explicitly, and an unreviewed wrapper does not inherit that declaration.
Only a negotiated telephone event with no NACK contract can use this path.
The registered primary payload owns its NACK/RTX cache; sharing its SSRC does
not let an auxiliary telephone event enter that cache. A busy or unsupported
pipeline refuses before submitting the event, preserving earlier ordinary IO.

The callback holds fresh current permission through encryption and the actual
UDP `send_to` future. Synchronous pipeline output is drained into that owned
future before any socket wait; cancellation cannot leave a tone in the normal
driver queue. Local success acknowledges full socket submission, never remote
consumption. The queue wait, permission acquisition and IO share a 200 ms bound.
Caller cancellation wakes the driver directly and wins over a socket becoming
ready in the same turn. IO is dropped before the guard is released.

A one-use atomic pending/started/cancelled ticket resolves cancellation against
submission. A definite non-write must first win cancellation against admission;
otherwise its result remains uncertain. A rejected initial request restores the
primary audio sequence and timestamp reservation. Once submission may have
started, those numbers are retained to avoid reusing a possibly transmitted
packet. Any later failure after a successful sequence prefix, including a
completed first digit, remains `OutcomeUnknown` for the original whole intent.
No guard spans a tone tick or interdigit gap, and no failure closes a healthy
peer as a fallback.

## Negotiated SIP telephone events

The UDP transport treats RTP as codec-opaque data. An RTP session decodes
telephone events only after an explicit receive profile is installed; an empty
profile never guesses PT101 or an 8kHz event clock. The shared decoder preserves
actual remote SSRC, negotiated payload/clock, original and extended event time,
observed sample duration and whether the start was observed. Its bounded replay
history survives removal and restoration of a mapping.

SIP negotiation validates dynamic payload identities, installed 8/16/48kHz
clocks and bounded event lists before allocation or commitment. The remote
SDP's events parameter describes what the remote endpoint can receive. Transmit
therefore uses the remote receive set, while incoming validation uses the local
advertised receive set. The complete requested sequence must fit the selected
transmit set before any first digit or permission acquisition. An explicitly
negotiated primary codec may use PT101; its number alone is never signaling.

The authorized SIP controller snapshots the committed transmit profile and
configuration generation, drops its initial reader before host authorization,
then obtains a nonblocking current configuration guard for each write. It
rechecks the generation and exact RTP resource and retains that guard with the
host lease through UDP submission. The SIP adapter additionally fences the exact
session registry handle and route and retains the actual session transaction
lane, refusing an in-progress SDP transaction without waiting while holding
host authority. No configuration or transaction reader spans packet ticks or
interdigit gaps. Rollback or reusing the same profile cannot revive a retired
configuration generation.

Audio frame input cannot schedule control effects: the former PT101 payload to
invented100ms digit conversion is removed. INFO observations require explicit
valid timing under the existing separately named profile. A general Core/native
operation still needs its own source, requested method, stable operation and
independent current signal-policy composition; transport opt-in APIs do not
supply those facts or a durable Connect receipt.

## Shared RTP accounting and primary flow

Admitted primary and telephone-event datagrams share their SSRC's sequence/loss
accounting, including wrap, duplicate and reordered arrivals. An event payload
uses an independent timestamp clock, so it does not update primary audio jitter
or enter the primary jitter buffer. Aggregate physical reception counts include
auxiliary packets even when the event payload cannot be decoded.

`primary_packets_received` separately counts packets matching the currently
configured primary payload. SIP's physical primary-flow hint uses this counter,
so auxiliary-only traffic or a preceding codec's payload cannot establish current
primary flow. This counter is not proof of decoded audio, bidirectional readiness
or remote consumption. Those remain separately observed facts.

## Primary audio admission

Core's audio pump and media graph compare a labeled frame with the current
primary payload before routing it. An unnegotiated payload is dropped; a
four-byte codec failure never becomes a signaling fallback. Primary payload101
is ordinary audio when its negotiated codec says so. The graph applies normal
codec conversion, clock translation and source provenance to that payload.

The graph rejects mismatched source payloads before recording primary activity
or buffering for a future sink. Dropped input remains counted as a drop. Its
buffered frames are checked against the profile current at routing time as well.
Successful pump transcoding labels bytes with the destination payload identity.
Explicit telephone-event observations and authorized signal writes remain
separate from these audio routes.

## Core native-recipient submission

Core routes a whole immutable DTMF intent to an explicitly registered native
recipient through its own authenticated signed writer. The requested gateway
method remains separate from the native wire format and actual source kind.
`DtmfRoutePolicy::acquire_native` receives actual signed envelope bytes, original
operation/source observation, current Core source/recipient bindings and sealed
native target identity/device/endpoint/member epoch, credential/stream revisions,
physical credential incarnation and negotiated audio handles. RTP/INFO policy
cannot substitute for this independent signal grant.

The bounded native queue retains metadata and a current-policy callback, not a
permission reader. The sole physical writer acquires fresh authority after its
own dequeue. Acquisition and owned IO share a 200ms bound, within the original
caller/credential deadline and one-second native submission bound. Native
flow/controller/credential/audio-port custody surrounds Core/source IO fences;
all synchronous locks release on Pending. Replaced registrations cannot lend a
successor to old queued work. A definite pre-IO refusal preserves the writer;
a possibly partial write destroys the sink before releasing its actual guard
and preserves `OutcomeUnknown` under the original operation.

Source observations preserve actual kind, physical observation ID and receive
time independently of the requested gateway method. Native ingress derives
these from its accepted signed frame; direct application commands use
`application` provenance without inventing packet observations. The host owns
the original-operation journal, deduplication and current replay authorization.
Success is local signed reliable-writer submission, not remote playback or a
claim that already admitted transport bytes can be recalled. See
[Core native DTMF](../docs/developer/core-native-dtmf.md) for the scoped procedure.

## Received SIP and WebRTC source ownership

`SipAdapter::subscribe_dtmf_source` and `WebRtcAdapter::subscribe_dtmf_source`
bind one installed adapter and original live Core endpoint to a bounded queue.
Replacing or dropping its subscription invalidates old metadata and prepared
routes. Supported Core binding mutations invalidate its epoch even after a
return to the same Session. A physical observation ID is separate from the
host's original logical operation and the requested gateway method.

SIP INFO uses the actual parsed request and transaction lifecycle. SIP RTP
retains exact decoded facts and receive times through the causal media event,
with the original RTP allocation and configuration generation. Updating media
invalidates old observations and queued packets, including rollback. Original
first-packet time survives segmentation and profile changes: an old tone cannot
acquire a new media generation or source subscription merely by ending later. WebRTC
uses the actual received track, SSRC and pump lifetime. Its signal registry is
separate from primary clock anchors and also applies when clock capture is
disabled. Same-SSRC replacement cannot revive an old track epoch.

The host journals/deduplicates observations before preparing an effect with its
original operation. Incomplete starts or fractional sample durations remain
observable but cannot become inferred or shortened tone effects. Raw digit
hints and payload source claims supply no signal permission. Every admitted
target IO poll still requires fresh independent source/recipient permission,
original Core metadata and actual source lifecycle/media/track custody. These
source registrations retain no idle permission reader or physical session.

Actual local SIP input to signed native recipients and SRTP input through Core
to a second SRTP peer are component checks. See [received DTMF source composition](../docs/developer/received-dtmf.md) for the scoped procedure. They do not install
production Store/service/HTTP/SDK/journal ownership or qualify remote/browser
or Linux profiles.

## Remaining qualification and implementation

Signed native ingress and egress are verified with local TLS QUIC/WebTransport
and controlled relationship storage. Component checkpoints bind their exact
source and executable digests; later changes require affected checks again.
Full G6-024 remains unfinished: full quality/interop coverage, and the remaining cross-runtime
framing/size/fragmentation/late/malformed/feedback/decoded-family qualification.
Production Store/HTTP/SDK/journal operations remain their assigned G6-037/G6-038
and dependent work. Approved Linux x86_64 and remote/native/browser profiles and
G6-093 bounded named rehearsals remain required separately. Publication remains
a separate action.

## Partial native writer retirement

A possibly submitted native DTMF frame destroys its physical sink and cancels
the corresponding physical route before releasing the actual source/recipient
IO readers. This ordering also applies when the write future is dropped.
Only then may its uncertainty receipt wake a journal worker or a new audience
be captured. A definite refusal before IO preserves the healthy writer.
Uncertainty retains the original logical operation and never schedules a second
delivery. Local submission remains distinct from remote playback.

## Bounded metadata admission

The complete requested DTMF schedule still has the 30-second bound. Its durable
native operation has a separate 40-second admission-to-settlement budget so a
valid maximum sequence retains bounded time for source/route preparation and
terminal packet redundancy. The original validated source and credential expiry
can only narrow this deadline; replay retains the first recorded expiry. Other
native stream commands keep their existing 30-second operation budget. This
does not lengthen a packet's 200 ms signal grant or retry a physical schedule.

RFC4733 tone scheduling uses the monotonic clock after the first submitted
packet. Fresh authorization work does not accumulate as an extra delay after
every 20 ms tick. A delayed writer skips obsolete continuation ticks, keeping
sample durations monotonic in the negotiated clock and retaining the exact
final duration and terminal redundancy. It queues neither old packets nor
permission readers while waiting for the next tick or interdigit gap.

Temporary Core relationship metadata contention keeps the original native DTMF
operation preparing before any writer is allocated. Connect recaptures only the
same sealed source observation and frozen recipient, within at most two seconds
and the original operation expiry. Core also waits for the captured route to
become current before invoking downstream policy or reserving its send slot.
These waits retain no source or recipient permission reader. Actual IO still
requires fresh independent authorization and original/current audience custody.

A planning deadline before IO records `not_started`; cancellation records the
original cancelled outcome. Replay returns that same outcome without sending.
No wait retries an adapter invocation or a possibly submitted frame, and actual
IO retains its existing uncertainty, cancellation and physical-retirement rules.

Native source flow/controller contention also preserves `NotReady` during
metadata preparation. A passive source liveness hint may wait on that state;
it is never an IO grant. Actual source polls retain every physical guard around
IO, and cancelled/retired/stale source epochs continue to refuse.
