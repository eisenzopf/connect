# Staged engine media ports

G6-020 remains in progress. `staged_streams` adds an opt-in engine controller and
bounded media ports around the negotiated stream model. Built-in UCTP, WebRTC
and SIP adapters do not yet opt in. `ConnectionAdapter::staged_media` defaults
to unavailable; the controller never falls back to live-mutating renegotiation.
There is no new public Connect operation or HTTP schema in this increment.

An adapter prepares only newly requested ports. Existing ports are read-only
during preparation. The engine supplies their transport IDs, stream generation
and nonzero 16-bit local handles. One allocator must span every Connection on a
physical peer; failed candidates burn handles and exhaustion never wraps.
Readiness must come from the exact negotiated adapter route, including source
and sink availability and the actual RTP payload type. Screen remains a distinct
logical medium using a video transport port. It does not imply capture permission.

Core port directions are explicit: Receive is endpoint to core; Send is core to
endpoint. The adapter translates the peer's wire direction when building its
current capabilities. A candidate port cannot publish before commit. The controller
rechecks current credentials, membership, appearance, verified endpoint, Connection
generation, per-medium grants and expected model revision after preparation.
Changed old ports quiesce their admitted sends before cutover. Unchanged audio
keeps its receiver, transport ID, local handle and delivery generation.

The trusted `MediaPolicy` returns owned, Send authority leases. The implementation
must retain its authorization fence while the lease exists; a queued permission
snapshot is insufficient. A policy lease protects a bounded local send, never an
unbounded network wait. Actor binding and credential expiry are checked by the
engine as well. The Connect store/service policy and its revocation integration
still need installation and verification in their later coordinator work.

An incoming `MediaPacket` retains its source port and delivery generation. Forwarding
requires current source publication and destination receive/audience permission,
matching media/format, the same Conversation and Session, and the same trusted
policy instance. Admission does not create an implicit subscription. Retained
subscription commands and the managed participant graph still need integration.

Queued outbound media exposes no payload accessor. After dequeue, `deliver`
rechecks source and destination authority and both exact port generations, then
retains their policy and delivery guards until the local send completes. The
send is bounded to 200 ms. Adapters must not detach work that outlives that guard.
Pause/resume drains admitted sends and advances a private delivery generation,
so pre-pause buffers cannot reappear on resume. Stale terminal callbacks can
retire their own old port but cannot end its replacement.
An authorized explicit stream end drains that port and keeps unrelated media and
the Session intact. Logical end and confirmed resource cleanup are separate facts.

Audio queues contain at most ten frames; video, screen and data queues contain
at most 64. Encoded audio and each data message are at most 16 KiB; complete video
RTP packets are at most 1,180 bytes. Input checks include stream ID, kind, actual
payload type, RTP structure/timestamp and declared text/JSON data syntax. Audio
and video packets expire after 200 ms of engine queueing. Ordered data has explicit
backpressure and a borrowed packet remains available for retry; adapters must stop
reading or retry the same message before accepting another. A full queue never
counts as accepted or delivered data. Transport framing, RTP hop rewriting and
network loss behavior remain responsibilities of the actual adapter integration.

Register exact-port idempotent cleanup before allocating adapter resources.
Cancellation, dropping a control future and shutdown fence candidate ports
immediately. The controller retains retired resources and capacity until cleanup
confirms, after all admitted sends drain. Missing, failed or timed-out cleanup
is quarantined and consumes capacity. Cleanup attempts are serialized and each
attempt has a bounded deadline. Keep the controller owned until shutdown and
cleanup are confirmed; dropping it stops delivery but cannot attest async cleanup.
The service maintenance driver and built-in adapter resource ownership still need
to enforce this lifecycle through real transport failures.

The scoped runner is `scripts/check-gate6-staged-ports.py`. Its fixture evidence
can establish engine ownership, current authority and packet/codec boundaries.
It cannot establish physical screen capture, VP8 decoding/rendering, live network
upgrades, public UCTP conformance, independent clients, remote profiles or providers.


The controller now defaults to 16 cumulative new-port opens per Connection,
independent of its active/quarantined port semaphore. It checks the whole planned
batch before allocating handles. Immediately before polling adapter preparation,
it consumes the new-port count; failed, cancelled or uncertain preparation does
not refund this lifetime budget. Reusing an unchanged port costs zero, while a
replacement costs one even when the logical stream ID is retained. Cleanup,
pause, logical end and repeated reoffers cannot reset the count. This conservative
accounting treats a failed batch as potentially opened; the adapter result does
not prove that none of its ports opened. `stream_open_budget()` reports attempted
opens and the effective limit. `with_open_limit` supports an explicit bounded
operator configuration. Keep the controller owned across reoffers; a new object
is not a protocol operation for resetting the same Connection's history.
See [`secure-profile.md`](secure-profile.md) for current qualification and limits.
