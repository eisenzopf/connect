# Canonical WebRTC signaling component

G6-051 is in progress. The internal TypeScript module
`browser_webrtc_signaling.ts` implements a signaling owner against the frozen
`connect/2` envelope shapes. It is not exported from the browser entry point yet.
The default canonical server does not enable a WebRTC media route. Protected
startup composition can install the G.711 staged media host described below.
The older browser voice/video entry points still use their historical signaling
dialect. These components do not establish browser fallback support or media
interoperability.

The component borrows the authenticated `BrowserClient` and takes ownership of
one supplied `RTCPeerConnection`. The caller configures transceivers, channels
and capture before transferring that peer; capture tracks remain caller-owned.
Conversation, Session, Connection, participant, revision and logical operation
IDs are explicit. The component snapshots offered capabilities and streams before
its first asynchronous operation. It gathers complete ICE before sending one
signed `connection.offer` with `substrate: websocket+webrtc` and the frozen
`substrate_setup` SDP shape.

The original exact SDP, payload, scope and submission-attempt flag remain
available after failure. A missing answer, lost response or timeout cannot
generate another SDP offer or operation ID. Recovery must inspect the original
operation using current authorization. Local close/cancellation does not issue
durable operation cancellation or invent a remote outcome. The shared client's
existing timeout policy can retire uncertain physical control; this helper does
not add a second control transport or retry policy.

An answer must match the original Conversation, Session, Connection and operation,
name the same substrate, and select unique offered stream IDs and kinds. SDP is
bounded to 512 KiB and applied through the browser API. Applying an answer is a
signaling observation: actual media requires independent ICE/DTLS, decoded
content and presentation evidence.

Canonical incoming ICE uses `sdp_mid` and `sdp_m_line_index`, converted explicitly
to the browser's camel-case fields. Candidates arriving before the answer wait
until the remote description is applied. One serialized queue has a lifetime
limit of 64 candidates/64 KiB, including work held inside the browser API; one
candidate is limited to 4 KiB. Foreign scope and operation candidates are ignored.
Malformed candidates or exhausted bounds retire the owned peer. Terminal
Connection/Session/Conversation events and shared control loss remove listeners,
gathering callbacks and queued candidates, and close the peer once.

Verification uses controlled peer and WebSocket boundaries. Two signing
algorithms exercise the actual `BrowserClient`, signature verification and
canonical frame encoding, without a WebTransport API. These are component tests,
not TLS/network, installed Chrome/Firefox, Core WebRTC or actual media tests.
The subsequent peer-state correction has 279 passing SDK tests, including
15 signaling methods; see the separate peer-lifetime evidence.

Next, complete the wider media profiles and integrate capture,
track/data routing and the public browser surface, remove the old offer-ready
dialect, and run forced WT/UDP failure in Chrome and Firefox with the same
Conversation and transition intents. Current authorization, uncertainty,
permission/background/resume, loss and owned teardown remain required.

## Durable offer preparation

The initial Connection intent contract now accepts `websocket+webrtc` and maps it
to the already provisioned `webrtc` Endpoint technology. Its SDP setup must have
the matching `kind`, a `v=0` CRLF prefix, no NUL, and at most 512 KiB of UTF-8.
The serialized offer is limited to 544 KiB, with at most 32 KiB of non-SDP
metadata. Native QUIC/WebTransport offers retain their 32 KiB limit. These are
contract bounds; the WebRTC engine must still parse SDP and validate the actual
selected media before exposing IO.

Trusted preparation selects exactly one verified Endpoint for the authenticated
Device and the intent's technology. A same-Device QUIC Endpoint cannot satisfy a
WebRTC offer, or vice versa. The original selected Endpoint, exact SDP and all
other intent fields remain immutable on replay. The Store preparation API and
current Connection projection enforce the same technology binding. These APIs
are trusted backend operations; this checkpoint adds no browser provisioning or
new HTTP endpoint.

Preparation creates only a dormant Connection and an accepted original operation.
It allocates no WebRTC peer, starts no physical effect, and reports no media
readiness. Endpoint revocation prevents current selection, projection and new
attempt issuance. Under current owner authorization, historical receipt reads
remain possible; those reads do not restore the revoked Endpoint or grant IO.
The protected service composition below retains this preparation boundary before
starting its original physical effect.

## Owned transport preparation

The internal server component `canonical_webrtc::Peer` starts from a Store-issued
origin and consumes its non-cloneable original attempt lease. It accepts no caller
SDP, Device or Endpoint selector. The backend retains the owner outside answer
and connection waits, so a cancelled wait cannot discard or replace that lease.
The original attempt remains available by reference for later attributed Core
readiness or cleanup work.

One worker owns SDP application and ICE/DTLS observation on the exact original
`WebRtcPhysicalBinding`. Trusted service composition first attaches the current
WSS peer to the Session, reserves its typed Core WebRTC Connection, and configures
its controller. Only then may that binding start its one physical allocation.
The binding and Core route retain the host capacity permit through actual RTC
close and original controller/resource cleanup.
Explicit shutdown callers share the original completion; dropping one shutdown
wait cannot take its join. Owner drop or supplied physical cancellation fences
the worker. Current Store lifetime checks retain no domain reader while idle;
current answer/connection observations reacquire their own source read.

Before allocation, the worker parses the saved original SDP and checks active
media kinds/counts against the original logical stream intent. SCTP channels share
one application m-line. This is transport preparation, not Core stream admission:
Codec, direction, MID/SSRC, subscription, media packet and transition authorization
belong to the canonical adapter/controller integration. The component exposes
no media frame interface. An SDP answer or ICE/DTLS connection cannot complete the
durable media operation. The default canonical WSS route does not install this component; no new HTTP
endpoint or fallback support is advertised.

Four local server-library tests verify actual loopback ICE/DTLS, current endpoint
revocation while prepared/idle, original attempt custody, refused foreign origins
and unoffered media, deadline refusal and owned cleanup. A test-only gate holds
close pending after a shutdown wait is dropped: both capacity and the actual
configured UDP port remain occupied. After the original close completes, that
port can be rebound. The ordinary library also compiles with `cfg(test)` disabled.
These checks use trusted isolated Store admission; they do not qualify signed
canonical WSS dispatch, browser interoperability or decoded media. Details are in
[the peer-owner evidence](../evidence/gate-6/webrtc-peer-owner.json).


## Protected canonical WSS SDP dispatch

`Service::install_canonical_webrtc` installs one trusted signaling host before the
first physical admission. Configuration is sealed by that admission. This hook
is not a public command or JSON configuration field. The server's
`canonical_webrtc::SignalingHost` composes the existing owned peer with a bounded
capacity pool. Its initial profile requires exactly one G.711 mono audio codec
and one logical audio stream. Opus mono, video and SCTP remain implementation
work; this is not the final Gate 6 browser media profile.

A WebSocket offer enters the same signed canonical handler and Store authority
as other transports. The exact verified `webrtc` Endpoint is selected, one
original offer attempt is issued, and its lease enters outer service-worker
custody before the first setup suspension. The host receives the exact context
already reserved by Core; it cannot allocate a second peer context. Exact replay returns the existing operation record without starting a
second peer or resending an SDP answer. An origin-issuance or capacity refusal
retains that original lease for attributed cleanup.

The host derives the selected RTP payload mapping from the actual original SDP
and engine answer. The service independently plans streams against current
Session acceptance, verified Endpoint capabilities and the authenticated
principal. Stream identity, medium, direction, codec, clock, channels and
participant must agree. It binds one immutable answer payload into the current
invocation grant. The physical signed writer admits exactly that payload with
the original Conversation, Session, Connection, operation and reply correlation;
the existing native-prepared datagram policy is unchanged. Standalone ICE
candidate admission is still required; this path gathers complete ICE in SDP.

The initial RTC preparation borrows the actual originating Store reader through
the saved origin. It revalidates that exact reader against the Store rather than
reentering an authority fence already held by the signed command. The tracked
worker releases every initial reader after answer/refusal, retains the peer after
answer delivery, releases the original command grant during idle waits, and observes spontaneous peer closure, physical
retirement, installation stop and the original preparation deadline. It joins
actual peer close before recording Store retirement. SDP and ICE/DTLS leave the
operation preparing, with no admitted managed Session Connection, media-ready result or fabricated
lifecycle success. The default host continues to advertise control only.


Temporary metadata contention grants no IO. RTC setup and answer observations
wait for fresh read admission within the original preparation/observation deadline,
and stop on physical retirement. Only reads are retried: SDP creation, peer
allocation, signed answer submission and the original Store attempt are not
repeated. The service also bounds originating reader/answer-plan preflight.
Worker negotiation and HTTP3 fixture authentication futures are heap-owned at
async composition boundaries. Verification uses the default worker/test stack.

RTC close now uses the recorded dependency patch
`gate6-webrtc-driver-close-join.patch`. An abort request alone does not confirm
cleanup: the actual driver must finish destruction before close returns. The
original handle remains owned if an observer cancels its wait. The separate
[driver-drain evidence](../evidence/gate-6/webrtc-driver-drain.json) records a
forced pending UDP destructor and 64 public RTC close/immediate-rebind iterations.

The current protected integration has 20 passing selected methods: 12 WSS,
7 RTC owner and 1 decoded WebTransport regression. The final run uses
the default worker stack. Full scope, bounded repetitions, source/artifact guards
and earlier failures are recorded in [the WSS dispatch evidence](../evidence/gate-6/webrtc-wss-dispatch.json).


## Deferred primary RTP delivery

The recorded `gate6-deferred-owned-rtp.patch` adds a driver-owned primary RTP
submission boundary. Queued jobs hold an opaque producer and a one-use delivery
ticket. The producer supplies its packet only when the RTC driver invokes it
against a borrowed socket sink. This is the boundary where the Core
adapter calls `QueuedMedia::deliver`, retaining fresh source and target
permission through the actual UDP future.

`TrackLocalStaticRTP::owned_rtp` returns an owner before asynchronous queue
admission. Result observations share the original worker; cancelling an observer
does not cancel that ownership. `cancel_and_drain` requires the original driver
receipt after callback and IO destruction. Missing receipts remain unconfirmed.
A cancelled write that may have started reports an unknown outcome and is never
automatically retried. A producer returning success without a socket write cannot
produce a successful delivery receipt.

The initial supported path is negotiated PCMU/PCMA, 8 kHz, one channel, with an
exact sender SSRC and payload binding, current and requested sending direction,
and the reviewed immediate interceptor pipeline. Its transport-cc feedback is
allowed; retaining repair profiles are refused. The ordinary queued RTP writer
is unchanged. The staged canonical adapter consumes the exact port-owned queued
frames through this boundary.

The [reproducible probe](../evidence/gate-6/webrtc-owned-rtp-probe.rs) exercises
actual loopback ICE/DTLS/SRTP with an instrumented permission fixture. It verifies
packet content, queued expiry, revocation before callback, foreign SSRC/payload,
direction withdrawal, a cancelled result observer, actual pending UDP destruction
before permission release, no late packet, and subsequent delivery on the same
peer. These are driver-boundary checks. The probe does not implement or qualify
Core `QueuedMedia`, canonical media admission/readiness, Opus/VP8/SCTP, installed
browsers, or the complete fallback journey. See
[the scoped evidence](../evidence/gate-6/webrtc-owned-rtp.json).


## Core port audio egress

`rvoip_webrtc::media::core_audio::CoreAudioEgress` consumes the egress receiver
belonging to one exact Core `AdapterMediaPort`. It accepts no arbitrary queued
frame, payload, stream ID or source permission. Binding checks the Core audio
format against the actual track encoding; each write is independently checked
against the current negotiated RTC sender by the driver. A selected sibling
codec cannot silently rewrite the requested payload type.

`submit_next` waits for that port's queued frame and the existing shared
audio/DTMF sequence owner without retaining a Core reader. Its deferred producer
calls the production `QueuedMedia::deliver` inside the RTC driver. Fresh source
and target authorization, original frame age, cancellation and route generations
therefore cover the actual socket future. Sequence and timestamp allocation
happens only inside that guarded call. Unknown submissions burn their sequence
and are never automatically repeated. A pending write retains the shared writer
even if the queue owner is dropped, so rebinding cannot restart the SSRC timeline.

The physical adapter must retain its existing shared RTP writer for the complete
track lifetime, independently of port retirement. The caller must register
exact-port cleanup before binding and retain every
returned original `OwnedRtpWrite` through `cancel_and_drain`. This component does
not own physical peer shutdown or report stream readiness. A missing original
driver receipt remains unconfirmed cleanup even if another owner subsequently
closes the physical peer. The driver destroys uninvoked producers and their
sequence custody before acknowledging definite pre-write refusal.

This is the initial G.711 mono egress implementation used by the protected
canonical server adapter. The peer and engine share one configured
primary codec table, so single G.711 peers allocate an actual G.711/8 kHz track
and shared writer. `RvoipPeerConnection::core_audio_egress` derives the unique
completed SDP mapping and peer-owned track/SSRC, checks stable connected state
and exact Core format, and retains the physical peer. It rejects ambiguous or
mismatched candidates before taking the queue. Configured preference alone does
not prove final codec selection; payload-aware multi-codec media and active-clock
changes remain work.

The [peer audio probe](../evidence/gate-6/webrtc-peer-audio-probe.rs) verifies
actual PCMU with dynamic PT109 and PCMA with PT8, two exact packets per peer,
refusal without queue consumption, retained ownership and physical close. Its
[scoped evidence](../evidence/gate-6/webrtc-peer-audio.json) records eight assertion
groups, the twelve Core probe regressions and twenty Connect methods separately.
The protected service integration now supplies ingress, owned port pumps and
cleanup, canonical Core admission and sealed actual readiness. Opus/VP8/data, timing,
MID/SSRC mapping and browser qualification remain open. The [Core egress probe](../evidence/gate-6/webrtc-core-egress-probe.rs) uses
actual Core controllers/queues and a real RTC pair with isolated policy and
synthetic capture fixtures; its [evidence](../evidence/gate-6/webrtc-core-egress.json)
keeps those limits explicit.


## Fresh original-offer continuation

`Store::resume_webrtc_offer_authority` reacquires current control for the exact
original Store-issued WebRTC origin and non-cloneable attempt after the SDP
handler releases its readers. It restores the original offer request, checks the
current credential, key, Device, Endpoint, membership, runtime owner and original
attempt, and derives a media read under that same new proof. It never prepares or
begins another attempt. Caller cancellation/deadline and the original source and
operation expiry bound acquisition; fresh authorization cannot renew those bounds.
An expired attempted operation remains uncertain, rather than becoming successful.

The protected G.711 service path now waits for the original handler to finish
and releases its originating readers before network resource observation. A
connected peer with no primary RTP remains `Preparing`; an independent
Conversation writer can run during that wait. The exact original physical
lifetime and host completion cancel a pending resource wait when that owner
retires. Observer cancellation retains the original source job and cleanup join.

After actual resource observation, the service reacquires the original offer's
current Store proof and derives its media read under that same interval. The
bounded control hook stages only the original selected streams, admits the
participant once, and commits readiness from the actual controller, activation,
peer, bindings and managed Session. Only precommit participant revision conflicts
may retry admission; metadata contention retries the same observation and
original effect. A fresh proof cannot renew the original source/operation expiry.
No reader survives into idle resource monitoring.

The local signed-WSS cases verify genuine primary RTP, original completed replay,
current endpoint withdrawal, and original RTC/source/port/controller joins before
slot and live-peer Core quota reuse. A remote UDP close is detected through the
pinned ICE policy (5 seconds disconnected plus 25 seconds failed detection),
rather than an invented close acknowledgement. Retirement appends its causal
journal event while preserving the original completed result and effect. WSS
control remains usable after the media route retires.

## Explicit RTC audio subscriptions

Protected WSS `stream.subscribe` and `stream.unsubscribe` use their own current
signed command and original Store stream-operation lease. The service checks the
actual originating WSS `PeerAdmission` against the installed RTC route, as well
as the current WebRTC Endpoint and original runtime Connection/generation. A
second WSS peer using the same credential cannot begin the original route's
mutation. Completed replay reads the same journal outcome under current authority.

Typed public participant and stream selectors resolve through the guarded Core
scene. The current source/receiver reads belong to that same command interval;
Core commits only its exact selection plan. Audio routing uses the existing
negotiated mixed port, with no SDP/native counteroffer or resource replacement.
Other media or a selection requiring new negotiated resources is explicitly
unsupported in this profile. Later routing revisions keep the original resource
monitor current while its actual bindings and activation remain identical;
initial readiness completion still requires its exact original revision.

With no selected source, the mixer emits no audio packet. Actual received media
continues to require fresh source and receiver permission through the final UDP
write. Unsubscribe removes delivery, and explicit reselect can restore it on the
same physical peers. Metadata contention retries only the captured original
outcome; it cannot repeat the routing effect or retain idle command readers.

The [stock RTC subscription evidence](../evidence/gate-6/webrtc-explicit-subscriptions.json)
records two signed EdDSA/ES256 journeys with independent G.711 decoding, no
implicit receive, same-credential foreign-peer refusal, replay, stale/unsupported
refusal, unsubscribe/reselect with reverse positive control, and publisher
Endpoint withdrawal. Those cases and all fourteen existing WSS methods plus one
decoded WebTransport regression pass on the same guarded source and executable.

## Explicit RTC termination

Signed `connection.end` requires its own current permission and the exact
originating WSS peer and RTC route. The service retains one original Store
stream-operation lease while Core closes the installed RTC connection. A
successful returned Core call and confirmed absence of that original route are
both required before it captures an ended receipt. The adapter retains physical,
source, port and controller cleanup custody through the joins; absence after an
uncertain call cannot manufacture a successful outcome.

Deadline, stop or cleanup uncertainty keeps the original attempt uncertain.
Metadata retries record only the captured actual success and cannot repeat the
close. A completed replay uses current journal authority, returns the same end
receipt and never creates another peer. The WSS control connection remains usable
after its media connection ends.

The [explicit end evidence](../evidence/gate-6/webrtc-explicit-end.json) records
both signing algorithms, real marked duplex before termination, original resource
and quota release, completed replay, continued WSS use, and refused end without
scope while duplex stays usable. Four RTC methods plus all fourteen existing WSS
methods and one decoded WebTransport regression pass. Separately injected
service-level pending-close/lost-receipt/deadline uncertainty remains unqualified.

The default host still makes no browser media advertisement. Wider RTC lifecycle control, reoffers, DTMF,
reconciliation/query/credential refresh, Opus/video/data,
ICE/MID/SSRC/source-clock profiles, actual Chrome/Firefox forced fallback and
approved Linux/remote hosting, package and full acceptance qualification remain
required. G6-051 remains in progress.


Idle RTC readiness now reconciles current durable Session roles through the
shared bounded role projection. Signed EdDSA/ES256 fixtures verify actual Core
observer projection, publication silence with reverse decoded audio, restoration
and superseded-operation replay without reinstating an old role. Short registry
and activation metadata exclusion no longer establishes withdrawn authority;
fresh authority and actual resource checks still follow after unlocking.

The [current role checkpoint](../evidence/gate-6/webrtc-current-roles.json) records
controlled negative/fixed cases and the original failed repetitions. The
[subscription contention correction](../evidence/gate-6/webrtc-subscription-contention.json)
keeps one original command while waiting for the scene tick slot or source
metadata before commit. It preserves the original deadline, cancellation,
current authorization and exact revision. A genuine revision conflict remains a
conflict; uncertain operations are never resubmitted.

The same six component cases fail on the original implementation; all35 fixed
Core scene methods pass. The unchanged signed journey passes all21 selected
RTC/WSS/WebTransport methods, four quiet six-method RTC repetitions and two
native QUIC regressions. Full G6-051 media/browser/hosting/package and acceptance
qualification remains open.
