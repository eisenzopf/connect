# Durable native DTMF source custody

G6-046 is in progress. The Store foundation uses the common logical operation
namespace for exact `dtmf.send` intent. It requires the named `dtmf.send`
permission and the original completed native admission, current credential,
verified signing key, Device, Endpoint, Participant epoch, physical generation,
runtime and expiry. Media permissions do not supply signal permission. Existing
credential issuance is unchanged.

The original Conversation, Session, Connection, optional expected revision,
digit spelling, duration, requested method and extension fields remain part of
retry identity. A fresh signed envelope may retry that identity; changed content
conflicts. Validation checks the complete sequence before acceptance: one to 32
ASCII telephone digits, RFC4733 duration 40–6000 ms or INFO duration 100–5000 ms,
a total schedule at most 30 seconds including 70 ms interdigit gaps, and an
original payload at most 32 KiB. Schema 83 repeats these source-shape constraints
at the database binding boundary. An explicit stale stream revision records a
failure before an attempt starts.

Acceptance reserves no transport effect. Beginning work durably reserves one
attempt before the engine can run. Cancellation before that cutoff closes the
slot; cancellation or restart after it preserves the same uncertain effect and
requires reconciliation. Replay never starts another attempt. A definite
no-effect refusal can close custody. A media, quality, descriptor or data-queue
receipt cannot complete a DTMF attempt. Payload claims such as `source` are only
original request data; they cannot create authenticated received provenance.

Schema 84 adds a sealed original source observation and exact recipient
registrations. Each recipient reserves one journal outcome slot before IO. A
fresh write read independently requires `dtmf.send` and `dtmf.receive`, current
original source/target bindings and both recipient ceilings. It retains the
same complete authority interval through bounded IO. Queued work retains no
reader. One actual output frame is reserved before it can be submitted; another
frame cannot reuse that recipient attempt.

Existing physical Connections use separate named-signal readers. A current
`dtmf.send` source and `dtmf.receive` recipient do not need `connection.offer`
or `stream.subscribe` merely to signal. `NativeSignalRead` cannot be converted
into an RTP grant. Each pair still requires current Device/key enrollment,
credential epoch, original Session audience, physical generation, private or
handoff restrictions, and both current recipient ceilings. The original offer's
resource list remains a locking footprint; it does not grant signal recipients.
Credential rotation changes physical metadata under Binding authority and does
not restore removed media permissions. Media reads retain their own checks.
Physical lifetime polling validates current ownership, binding and original
Session access. Losing RTP permission alone does not retire a signal-capable
Connection; this metadata observation grants no packet or signal IO.
Canonical authentication derives the protocol data floor from either named
signal permission. It does not derive Session/media or subscription authority.
The owned audio helper preserves an explicit subscription refusal as an
application error. [Local signal-only evidence](../evidence/gate-6/native-signal-authority.json)
records real consent, independent receive-only refresh, actual audio refusal and
both signing algorithms; it does not close the full auxiliary task.

Trusted writer callbacks record `not_started`, `local_submitted` or
`outcome_unknown` atomically with an operation journal fact. Captured facts can
be recorded after credential loss or retirement, with no revival or new write.
On restart, a reserved frame becomes unknown, while an unstarted recipient is
recorded as not started. No recovered row creates another delivery job.

After all owned jobs stop, all local submissions complete the operation. Known
partial/no-effect outcomes fail it; a cancelled no-effect operation becomes
cancelled. Any uncertain submission keeps the operation unknown. The stopped
control slot can then close without changing uncertainty or replaying the
signal. Successful operations put the summary in `result`; other final states
keep it under `problem.signal`. The operation-status
[HTTP/OpenAPI reference](../../contracts/openapi-operations.json) describes the
summary. It explicitly labels local submission and includes the frozen source
observation, recipient incarnations and each actual output-frame reservation.
These facts do not prove remote playback.

The initial SIP gateway now has a separate Store signal reader,
`try_initial_telephone_signal_read`. Direction is relative to the telephone
Endpoint: `FromTelephone` requires `dtmf.send`, while `ToTelephone` requires
`dtmf.receive`. An answered call must have its original connected incarnation,
current runtime, Device, verified Endpoint revision, current Participant and
Session audience, route credential and recipient ceilings. The reader retains
those authority fences for at most 30 seconds and the credential expiries. It
does not require `connection.offer`, `stream.subscribe`, or the previously
selected audio codec merely to signal on the existing audio Connection. Media
reads continue to enforce their separate permission and codec checks.

This reader is one leg of the bridge's authority boundary. It cannot authorize
another party, freeze a source observation, reserve a delivery, or supply an RTP
grant. Mixed SIP/native DTMF execution, durable recipient custody and observed
SIP/native provenance still require implementation and production verification;
the Store reader alone does not qualify those clauses.

`try_initial_telephone_native_signal_read` acquires the two sides together and
rechecks the exact original public native/SIP pair under one merged authority
interval. From the telephone Endpoint it requires `dtmf.send` plus the native
party's `dtmf.receive`; toward that Endpoint the requirements are reversed.
Both current recipient ceilings, physical credentials and original Session
audiences still apply. Private consultation, preparation and suspended public
publication contexts cannot use this public gateway reader. The pair retains
no media grant and does not borrow `stream.subscribe` or `connection.offer`.
Keep original origin metadata in a queue and acquire the reader at the actual
signal boundary; release it before waiting for metadata writers.

The signed native QUIC service installs a recipient registration on its actual
physical handler and drives that handler's DTMF mux. The original source
observation uses managed Core ingress. The Store selects current named signal
recipients under the original invocation and freezes their exact registrations.
The handler returns durable preparation, releases its invocation, and an owned
worker performs delivery. Every actual writer acquires fresh joint Store
authority, validates the actual frame and recipient, and adds Core's original
and current audience fence. Closing worker admission waits for all actual write
leases to stop before recording local outcomes. Storage recovery retries the
captured fact rather than the signal.

Recipient selection and observation freezing share a preparation window of at
most two seconds, capped by the original lease expiry. Temporary contention
reuses the original registration candidates; successful freezing still creates
exactly one batch. The writer waits for Core audience metadata before acquiring
Store readers. If frame reservation is temporarily busy, it releases readers
and media owners before waiting, then reacquires current authority for the same
signed frame. It never replaces that frame or retries an attempted delivery.
The existing bounded native writer deadline continues to apply.

The Rust SDK exposes `CanonicalClient::send_native_dtmf(scope, digits,
duration_ms, method)`. Supply the existing Conversation, Session and Connection,
a stable operation ID and an optional expected stream revision. A `preparing`
reply is acceptance; use `operation.get` to observe settlement. Keep the exact
intent and operation ID after uncertainty. Verified `dtmf.received` envelopes
remain available through `receive` and the owned audio helpers' existing
`receive_signaling` methods. Request batches preserve unsolicited DTMF events
instead of consuming them as replies.

`PublicNativeAudio`, `PrivateNativeAudio`, and `PreparedHandoffNativeAudio` also
expose `send_dtmf(operation_id, expected_revision, digits, duration_ms, method)`.
They derive source IDs from their owned physical peer and retain the caller's
explicit optional revision. The helper supplies no additional permission and
does not change private or prepared-handoff isolation.

HTTP cannot originate this physical native signal. The canonical
[HTTP reference](../../contracts/openapi-canonical-http.json) documents that
boundary; operation status, cancellation and reconciliation retain their shared
HTTP contract. Caller-provided `source` or observation fields remain retry data
and never replace actual native provenance.

The local production checks include signed EdDSA/ES256 delivery and replay,
queued SDK events and provenance, missing named receive permission, journal
storage recovery after submission, actual HTTP result/schema projection, owned
public-audio send/receive, finite-credit partial writes, source/recipient
revocation, cancellation, and restart after a received but unjournaled local
submission. A private consultation check excludes the retained remote and a
public Connection of the same Participant, then verifies that completed private
replay cannot publish after return and a fresh public signal reaches the restored
remote. These establish their recorded local QUIC scopes only. The
[reusable native route](native-dtmf-route.md) and
[native recipient writer](core-native-dtmf.md) supply engine primitives,
including the managed original/current audience fence. An unfinished queued
private signal crossing return, the wider key/Endpoint/membership/expiry fault
matrix, private/handoff wrapper combinations, SIP/WebRTC attribution, and the
remaining remote/browser/Linux profiles still require their assigned
qualification. These are not supplied by Store fixtures or by successful local
submission. G6-046 remains in progress.

## Partial native writer retirement

A possibly submitted native DTMF frame destroys its physical sink and cancels
the corresponding physical route before releasing the actual source/recipient
IO readers. This ordering also applies when the write future is dropped.
Only then may its uncertainty receipt wake a journal worker or a new audience
be captured. A definite refusal before IO preserves the healthy writer.
Uncertainty retains the original logical operation and never schedules a second
delivery. Local submission remains distinct from remote playback.

## Bounded metadata admission

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
