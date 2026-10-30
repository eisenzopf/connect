# Canonical WebRTC ICE restart

G6-054 implementation is in progress. The bounded request types are implemented
in `connect-protocol::webrtc_ice_restart`. Private Store custody is implemented
in schema116. Signed WSS dispatch, private answer delivery, tracked original-peer
custody/cleanup and browser SDK handling are implemented. The generated server
HTTP/OpenAPI reference includes the action and its physical transport restriction.
Behavioral qualification and the full G6-054 network/media matrix remain separate.

The mutation is `connection.update` with the original Conversation,
Session and Connection scope, a new logical operation ID saved before IO, and
the current media revision assertion. Its payload is:

```json
{
  "action": "ice-restart",
  "details": {
    "expected_ice_generation": 0,
    "setup": {
      "kind": "websocket+webrtc",
      "sdp": "v=0\r\n...the original complete gathered offer...\r\n"
    }
  }
}
```

The ICE generation is an independent transport counter. It does not replace the
Connection ownership generation, Stream generations or membership epochs. The
caller asserts an expected counter; current authority and the physical owner
come from trusted Store/Core context. No body field grants either one.

The protocol boundary preserves the exact SDP string, requires the WebRTC
substrate, bounds SDP to 512 KiB and the serialized payload to 544 KiB, and
accepts only an exact nonnegative JSON integer whose successor remains safely
representable by browser JSON numbers. Unknown/mixed codec, gate, routing,
resource and approval fields refuse. SDP and ICE credentials are redacted from
Debug output. This boundary does not parse SDP semantics or establish readiness.

## Restart-only offer comparison

`validate_offer_change(previous, proposed)` compares the complete proposed offer
with trusted retained SDP from the same peer's last accepted remote offer. Both
ICE credentials must change for every active media section, including inherited
session credentials, and the numeric origin version must increase. Numeric
connection/RTCP addresses, active transport ports and bounded candidate lines
may change. Every other line and its order must remain unchanged: DTLS
fingerprint/setup, MID/BUNDLE, codecs, directions, source/track IDs and unknown
extensions. Active media cannot become disabled or vice versa. Duplicate ICE
credentials, identity/setup attributes, ambiguous line endings, invalid address
families and bounded-count/line overflows refuse. The guard never returns SDP
or credentials in diagnostics.

Canonical candidate admission uses `validate_restart_candidate`. It additionally
permits omission of media-level `extmap` lines, which Chrome removes when the
original answer declined them. Candidate admission retains intent; it grants no
RTC IO and does not prove that any omitted mapping was declined.

Before physical submission, `validate_negotiated_offer_change(previous,
trusted_answer, proposed)` checks the actual local answer captured from that
same admitted original RTC peer under current authority and its exclusive
control slot. An omitted mapping must be absent by **both ID and URI** from the
answer for the same MID, including session-level mappings. Negotiated mappings,
all retained mapping lines/order and every other non-transport field remain
unchanged. Missing/foreign/duplicate MID, ambiguous IDs and omissions with
unknown extension parameters refuse. The requester cannot supply this answer.
The strict two-description comparison remains the check on the host's new
local answer, whose negotiated media must stay identical to its old answer.

This is a conservative comparison, not a general SDP or candidate parser. The
RTC engine must still parse the complete offer before applying it. A trusted
previous offer is required; a caller cannot supply its own comparison baseline.
Status/replay must recover the original operation before this fresh-attempt
check, since unchanged original credentials deliberately refuse a new restart.

The local `check_ice_restart_offer` Cargo example compares two retained private
SDP files without printing their contents. The explicit
`scripts/video/browser-ice-restart-offer.mjs` fixture creates isolated headless
Chrome peers without ICE servers, captures full gathered initial/restart offers
for audio and audio/video and closes those peers. Both generated shapes pass
the current comparison. This fixture uses no Connect server and does not prove
restart application, ICE connectivity, decoded media, remote/relay behavior or
allocation cleanup in the canonical host.

## Observing the current engine generation

The recorded upstream `ice_generation_ready(local, remote)` query checks the
live ICE agent's complete local and remote credentials and current nominated
pair, current ICE/DTLS state and stable signaling under the same RTC core lock.
It rejects pending signaling, empty/mismatched credentials and closed peers.
Its async wrapper acquires that lock; it reads no historical statistics as
readiness evidence. The existing sticky `is_connected` observation must not
substitute for this generation-specific check.

The expected parameters must come from the original attempt's retained actual
local answer and remote offer. A caller-supplied counter, SDP answer or boolean
is not execution proof. This query supplies no current membership, Endpoint,
physical-owner or media permission; canonical execution must enforce those
independently before and after observation. It also does not prove decoded
media, direction/gate changes or full end-to-end reconnection.

The local engine tests exchange actual synthetic sans-I/O ICE/DTLS packets on
two retained peers, restart both credentials, drop the first restart flight,
then require a new selected pair before either peer reports ready. They also
refuse stale generations, same-fragment wrong passwords, empty parameters,
closure and historical Connected metadata. No sockets/providers/media sources
are created by that packet fixture. This read-only query does not install a
canonical wire handler or grant IO.

## Private Store custody

The Store admits a strict bounded ICE intent only against its current original
WebRTC source, exact media revision and separate expected ICE counter. The
existing native control slot excludes competing attempts. Schema116 retains
an immutable per-operation previous/proposed offer and actual local
baseline/answer, plus an independent committed counter for the original parent.
Unknown outcomes retain that same slot and operation; no lease is reconstructed
and no second effect is begun on retry or after runtime restoration.

Trusted host capture methods save the actual original peer's baseline before
IO and its gathered answer before delivery. Repeated capture must match the
same bytes. Baseline capture must validate any candidate header omissions
against that actual answer before saving it or invoking RTC. Removing a
negotiated mapping fails the original candidate without saving an answer or
advancing its counter; candidate admission is never an IO grant. An answer does
not advance the counter or complete the operation,
and cannot later be described as a no-effect outcome. A captured, attributed
engine completion consumes the original lease and atomically advances only the
ICE counter with the common operation completion. Replaying an old receipt
cannot reset a newer counter. The original stream revision, bindings and
readiness projection remain unchanged by this transport-only metadata commit.

SDP and credentials stay in dedicated private tables. Operation results and
shared stream projections carry no SDP. Reading the original answer requires
current source authorization and the exact retained request. Backup digests
cover both private tables. Older live schema paths are refused unchanged; use
the preserved matching binary/backup and explicitly initialize a new path.
The Store tests control source and engine-outcome metadata; they are not actual
RTC or decoded-media qualification. The broader Store run passed 1,284 functions
with three ignored worker entrypoints and one stale enrollment schema assertion. That assertion is corrected and its
exact function passes on unchanged production inputs; the original full run
remains failed. This is composite coverage, not a fresh full-suite pass.

## Original physical custody

`NativeIceOrigin` retains only Store-issued attribution for the original
command, complete credential chain, key, member epoch, Endpoint, effect and
runtime. It holds no authority readers. Each subsequent observation must use
`resume_native_ice_authority` with the original move-only attempt lease. Fresh
admission is bounded by its original expiry; Unknown attempts, foreign effects,
changed runtimes and withdrawn authority cannot obtain another IO grant.

Core's `TransportUpdateSlot` holds the same exclusive slot as media/gate,
audience and credential changes. It stages no media proposal and changes no
revision, port, source or gate. A separate default-deny transport policy must
recognize the exact held invocation. Each use checks current authority, the
original credential incarnation, same original port pointers and deadline.
Idle custody retains only the slot and attribution, allowing authority writers.

The internal service `Restart` custodian borrows the exact original canonical
RTC allocation, captures its actual SDP baseline and permits only one physical
answerer restart call. It persists the returned answer before returning it to
its caller. Submission errors/cancellation cannot become no-effect receipts;
replay/status must not invoke negotiation again. Fresh short authority reads
guard each live observation, and the RTC query compares the exact persisted
offer/answer and their live ICE generation together under one core lock.
Only that observed completion consumes the original lease into a receipt.
Metadata persistence can retry with that receipt without invoking RTC again.

The signed WSS handler runs this custodian in the Runtime's tracked worker set.
Initial negotiation derives authority from the handler's already-held interval,
without acquiring another reader behind a waiting writer. It returns the stored
answer in a correlated signed ACK:

```json
{"details":{"reply_complete":true,"ice_restart":{"proposed_ice_generation":1,"setup":{"kind":"websocket+webrtc","sdp":"v=0\r\n...original answer...\r\n"}}}}
```

The ordinary reply writer owns the actual original command authority through its
write. The worker drops its handler reference and originating readers before
waiting or observing ICE. A reply or proposed counter is not completion. Only
the original operation's captured result contains `ice_generation`. Possible
effects stay Unknown on cancellation, expiry or authority withdrawal. Cleanup
matches the original admitted peer and actual RTC allocation, releases the Core
slot and joins that exact route's original close. The parent custodian records
its own attributed retirement; another Connection or the shared WSS control
peer is not closed by this ICE cleanup capability.

`BrowserConnection.restartIce(streamRevision, expectedIceGeneration, options)`
and `BrowserVoiceSession.restartIce(operationId, streamRevision,
expectedIceGeneration, options)` use the same original `RTCPeerConnection`.
The operation ID is installed before gathering; the complete gathered offer is
saved before submission. Repeated uncertain waits join the original preparation
and answer-application promises and resend only the same original payload. A new
operation cannot replace an unresolved one. They wait for the original operation
completion before returning a historical `WebRtcIceRestartReceipt`; it carries
the operation ID, ICE counter and unchanged media/Connection generations, and
does not prove independently decoded media. No capture, transceiver or new RTC
peer is allocated by this operation. The explicit private `savedIceRestart`
snapshot on the voice/signaling owner supports attribution during recovery.

Source compilation and component tests alone do not establish actual canonical
restart, network recovery or decoded-media qualification. Consult the task's
current evidence for the exact qualified profiles and unresolved checks.

## Required public execution and qualification

Before physical IO, save the complete original payload, caller, scope, current
Endpoint/membership, Connection ownership generation, expected media revision
and expected ICE generation. A duplicate original operation must recover the
same record and exact original offer/answer, including after observer loss.
It cannot silently gather another offer, start another attempt, replace the
Connection, choose another Endpoint or switch substrate.

The canonical physical host must use its existing Core-reserved WebRTC peer.
Upstream's other WebSocket/WHIP signaling adapter already has an authorized
answerer restart primitive; that route does not grant access to canonical
resources. Public integration must retain the guarded original physical owner,
logical streams, source identities and gates throughout that mutation.
Validate unchanged media/DTLS identity and the new ICE credentials before
applying the offer. No changed route is accepted from an untrusted body alone.

An SDP answer alone is not reconnection evidence. Completion must observe the
new ICE generation on the original peer, enforce current authorization and
attribute the original effect. Late observations from an old attempt, stale
generations and superseded owners cannot complete a newer request. Unknown
outcomes retain their original operation and require observation/reconciliation.

Browser code must save its exact gathered offer and logical request before
submission, apply only the correlated original answer and preserve uncertainty.
The end-to-end proof must include direct/relay recovery, stale/refused/expired
authority, lost/repeated responses, independently decoded duplex media and
original allocation/peer cleanup. Browser compositor quality and other remaining
G6-054 matrix requirements remain open; thresholds are unchanged.

The public decoder/schema recognizes this action. HTTP and native QUIC or
WebTransport cannot restart a canonical WebRTC peer: mutation requires the
original signed WSS physical owner. HTTP status/cancellation remain separately
current-authorized metadata operations. See `x-webrtc-ice-restart` in
`contracts/openapi-canonical-http.json`. No compatibility or legacy route is used.
