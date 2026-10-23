# Retained audio subscriptions (Gate 6 work in progress)

Managed participant audio starts with an empty receiver selection. The mixer
admits a contribution only when an explicit selection covers its current logical
stream and the current publication, receiver and audience policies permit it.
The raw subscription registry does not grant access to managed audio.

The trusted `Orchestrator::change_staged_audio_subscriptions` operation applies
additive exact-stream or Participant selections, idempotent exact-stream removals,
or a clear operation to one registered receiver. An exact selection pins the
original controller and logical stream; transport replacement preserves it. A
Participant selection follows current and future devices of the captured
credential owner and membership epoch. A removed and readmitted membership must
be explicitly selected again. Unsubscribing an exact stream also records an
exception to a Participant selection. Explicitly subscribing it again removes
that exception. IDs and retained input sizes are bounded.

Routing changes compare and advance the receiver's Connection revision. Unknown
sources, stale revisions, denied audience checks and failed preparation preserve
the previous routing program. Changes quiesce the receiver's independent routing
ticket, drain sends already admitted to IO, and then commit the new program and
revision together. Other listeners continue. Cancellation restores the previous
generation. Queued output carries the original routing ticket and every actual
contributing source; delivery rejects obsolete routing and rechecks current
source publication and destination audience. New selections exclude samples
captured before the routing change and reset encoder/resampler history.

Held control derives the exact subscription action, target and expected revision
through `MediaPolicy::current_control`. Per-source receive checks use
`MediaAuthorityLease::control_receive` on that same held lease. Its default is
unsupported, so implementations cannot silently acquire a fresh reader behind a
queued authority writer. The held grant is bounded by credential expiry,
cancellation and deadline and is never retained as frame authority.

`scripts/check-gate6-media-subscriptions.py` records source hashes, compiler test
binary hashes, private log hashes and required behavior cases on local macOS
arm64. It runs the complete Core library, eight selected Core integration suites,
the native peer-admission suite, schema vectors, formatting and progress checks.
Signed local TLS QUIC/WebTransport fixtures use distinct people and devices, real
negotiated RTP and decoded audio, and controlled current policies/readiness.

This is a trusted engine primitive, not a public durable operation receipt.
Canonical public subscription handlers must compose it with their retained
mutation identity and held Store authority. General video/screen/reliable-data
subscriptions, choosing a different logical audio stream, canonical negotiation
and production policies, broader guarded SIP/WebRTC/browser media, the general
durable Session projection and complete facade migration remain open. This step
does not close G6-020, G6-037 or Gate 6 acceptance.
