# Canonical WebRTC ICE restart

G6-054 implementation is in progress. The bounded request types are implemented
in `connect-protocol::webrtc_ice_restart`. Canonical dispatch, durable Store
admission, physical execution, SDK methods and HTTP/OpenAPI registration are
still pending. This document does not advertise a working public restart API.

The intended mutation is `connection.update` with the original Conversation,
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
are created by that packet fixture. The Store operation and canonical restart
worker are not installed by this read-only engine API.

## Required durable execution

Before physical IO, save the complete original payload, caller, scope, current
Endpoint/membership, Connection ownership generation, expected media revision
and expected ICE generation. A duplicate original operation must recover the
same record and exact original offer/answer, including after observer loss.
It cannot silently gather another offer, start another attempt, replace the
Connection, choose another Endpoint or switch substrate.

The canonical physical host must use its existing Core-reserved WebRTC peer.
Upstream's other WebSocket/WHIP signaling adapter already has an authorized
answerer restart primitive; that route does not grant access to canonical
resources. Add the corresponding guarded canonical primitive and retain the
same original physical owner, logical streams, source identities and gates.
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

The public HTTP reference and shared OpenAPI schema must be updated with the
new action when canonical dispatch is installed. Until then the existing public
decoder continues to refuse this action; no compatibility or legacy route is
added.
