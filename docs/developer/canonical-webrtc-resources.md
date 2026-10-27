# Canonical WebRTC resource ownership

G6-051 now has a separate, typed Core resource preparation path for WebRTC.
This component reserves resources; SDP/DTLS and resource retention do not admit
a Participant, create a media route, or complete a canonical operation. The
protected signaling host and default server do not install this adapter yet.

`Orchestrator::prepare_peer_webrtc_connection` accepts a `PeerWebRtcIntent`
without a caller-selected transport. The trusted caller holds the current
`PeerNativeAuthorityLease` through synchronous preparation and retention. Core
checks the authenticated physical peer, verified WebRTC Endpoint, Device,
membership epoch, current permission and open Session/Conversation. It reserves
one never-reused Connection ID and the peer's shared Session/Connection quota
before invoking the separate adapter hook. The native preparation entrypoint
retains its QUIC/WebTransport/WebSocket allowlist.

`CanonicalWebRtcAdapter` implements that hook without allocating an RTC peer.
Its dormant Connection is inbound and Connecting, with no streams or media
capabilities. The trusted WSS factory creates `WebRtcPhysicalBinding` with the
original authenticated `PeerAdmission` and physical cancellation token. That
context accepts exactly one Core Connection and rejects another physical peer,
even before the first successful claim. Core cancellation and physical closure
both prevent allocation and observation of a current peer.

After reservation, `start_peer` starts one original `RvoipPeerConnection`
construction task. `peer(deadline)` observes that task; expiry or cancellation
of an observation cannot restart allocation or discard its original join.
The constructor validates primary audio encoding before starting its RTC driver.
A retired reservation closes a successfully constructed peer before returning
a definite construction refusal.

`shutdown` owns one original shared close task. It first cancels the physical
binding, joins original construction, and awaits actual RTC driver destruction.
Dropping a waiter cannot cancel that close task. A missing construction or close
join remains unconfirmed. Adapter teardown retains route liveness and Core quota
until cleanup succeeds; uncertainty cannot be reported as a quota refund. A
retained application `Arc` to the closed RTC peer does not keep its UDP socket
open. Drop requests cleanup without claiming confirmation.

The checked-in [resource probe](../evidence/gate-6/webrtc-core-resource-probe.rs)
uses ordinary production libraries and isolated policy/membership fixtures. It
checks preallocation denials and quotas, dormant state, exact context ownership,
never-reused IDs, actual UDP allocation/rebinding, canceled pending observations,
unretained candidate and physical EOF cleanup, definite construction failure,
and quarantine after a real constructor worker panic. That last case supplies an
invalid trusted fixture channel capacity; it does not inject an adapter result.
Recorded build and runtime evidence is in
[`webrtc-core-resources.json`](../evidence/gate-6/webrtc-core-resources.json).

The next integration must install this adapter alongside the service's Core
controller, hold original Store attempt custody, and bind actual ingress and
owned egress pumps to staged ports. Install cleanup before media allocation;
retain every original write through drain. Seal readiness only after actual
negotiated resources and current policy checks succeed. Complete browser capture,
Opus/VP8/data, ICE authority and required Chrome/Firefox/hosting profiles remain
part of G6-051. Publication is separate.
