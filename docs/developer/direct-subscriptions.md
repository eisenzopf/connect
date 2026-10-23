# Guarded direct stream subscriptions (Gate 6 in progress)

Staged video, screen and reliable-data destinations require an explicit exact
source-to-receiver logical-stream selection. They start empty. A video source
cannot borrow a screen binding even when both use VP8. Audio continues through
the separately guarded participant mixer and its audio selections.

`StreamController::change_direct_subscriptions` is a trusted engine SPI. A
`DirectSubscription` pins a publisher controller weakly, its source logical ID,
and the receiver's distinct logical ID. Each receiver binding has at most one
publisher. Additive subscribe replaces that binding explicitly; unsubscribe by
receiver logical ID and clear are idempotent. Inputs and the retained program
are bounded to 32 selections and IDs to 128 ASCII bytes. Publisher references
are weak so reciprocal selections cannot retain retired controllers in a cycle.
No serialized source identifier is itself an authorization grant.

Each receiver logical binding has its own routing ticket. Changing a video
selection neither drains unrelated data IO nor invalidates its queued messages.
A routing change shares the receiver's stream/Connection revision and in-flight
control permit with media changes. It validates exact conversation, Session,
policy domain, live logical bindings, directions, format intersection and current
per-source receive/audience authority before and after draining admitted IO.
Source-state acquisition is nonblocking, so reciprocal control cannot deadlock.
A failed or canceled change restores the previous delivery generation. New
selections exclude packets buffered before the change. A no-op preserves the
revision and queued generation. Removals need no fresh source permission.

Forwarding captures the original direct-routing ticket. Delivery reacquires
current publication and destination policies, validates the exact target gate,
and holds routing plus transport guards through the bounded local send. Changed
intent, source/receiver transport replacement, pause/end, membership retirement,
credential expiry or audience revocation cannot authorize stale queued output.
A new controller reusing source IDs cannot inherit a predecessor's selection.
Logical selections survive transport replacement; incompatible codecs still
refuse forwarding. The ordinary forwarding API never creates a subscription.

Guarded delivery also exposes exact source-binding metadata to the native egress
packetizer without changing clock provenance. A changed publisher, transport
binding/generation, source SSRC or supplied clock generation starts a fresh video
fragmentation map and egress SSRC. A publisher restarting its RTP sequence after
replacement is not mistaken for an old duplicate. The UCTP egress datagram
sequence continues; source clock anchors and timestamp evidence are not invented.

Reliable data uses the existing bounded queues and explicit backpressure. The
caller retains the same `MediaPacket` and retries after capacity returns; routing
does not acknowledge a drop or substitute a datagram transport. Authority or
subscription removal still refuses queued data. Local queue admission and local
writer completion do not prove remote application consumption.

Held control requires the exact `DirectSubscriptions` intent and derives each
source's receive facts with `MediaAuthorityLease::control_receive` under the same
held guard. It does not reacquire a reader behind a queued writer. Fresh frame
authority is never inherited from that completed control invocation.

The source-bound runner is `scripts/check-gate6-direct-subscriptions.py`.
Qualification is local macOS arm64. Signed local QUIC/WebTransport fixtures use
real RTP and controlled readiness/policy. They do not prove video decode/render,
browser media or production Store policy. Core ordered-data queue tests are not
native reliable-data transport evidence. Automatic general fan-out ownership,
Participant selectors for non-audio streams, public durable subscription and
negotiation handlers, native reliable-data transport, logical audio reselection,
SIP/WebRTC/browser staging, G6-038's durable Session model and complete G6-037
migration remain open. G6-020 and Gate 6 acceptance are not closed by this step.
