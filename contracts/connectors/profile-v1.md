# Connect connector fixture profile v1

Manifest version `1` and harness profile `connect-connector-fixture-v1` version
independently from the Connect server. Trusted Rust composition registers adapters;
wire requests cannot install code, register contracts or grant permission.

The normative types are the public `connect-connector-api` crate. Manifest fields
are strict (unknown JSON fields fail); version, address, typed operation set, messaging, context, SMS,
audio format, unsupported video, finite byte/queue/teardown limits and configuration
reference declarations must validate before any job is recovered or claimed.
Secret fields accept `env:NAME` or `secret:opaque-key` references, never raw values.
The host resolves/stores secrets; a reference does not prove possession or remote
availability. `Ready` describes validated construction, not a successful provider
call. Missing configuration, missing secret, unavailable and reconciliation-required
are distinct readiness states. No manifest conveys caller authority.

The message extension accepts a stable logical mutation ID, current input and
separate bounded authorized historical context. Each context item retains its
source/event ID, author, provenance and derived flag. Replies identify the source
items actually used. A selected history item never becomes an instruction or a
credential. An independently initiated provider message is `IncomingMessage`
with its own logical mutation and external IDs, authenticated by the adapter.
Trusted host composition binds it to an already configured route, checks current
credential and conversation authority before duplicate handling, and journals one
ordinary message. IDs themselves grant no rights. The fixture harness creates a
new incoming provider contact without invoking outbound send and uses that contact
as a source in a later answer; application business authorization remains
application-owned.

Message observations are `Accepted`, `Delivered`, `Failed`, `Uncertain`.
Accepted only confirms provider acceptance. A synchronous contextual answer may
be persisted as Connect's existing `completed` job result; this does not assert
external recipient delivery. SMS is a message operation, never a voice leg.
Voice observations are `Starting`, `Active`, `Ended`, `Failed`, `Uncertain`.
Telephone control and native media bindings remain separate provider adapters.
No generic manifest implies Vapi vision, Telnyx PSTN video or SMS media.

An observation contains logical mutation ID, optional external ID, stable event
ID, typed state and verification provenance. Verified callback observations may
only be constructed after authenticating the exact raw bytes with the provider's
signature verifier. `AuthenticatedResponse` is an authenticated provider API
response; `LocalAttempt` describes a local timeout/cancellation, and
`Reconciliation` describes an explicit authoritative status/discovery query.
Caller-supplied IDs are correlation only. The host checks current access, key
revocation/expiry and SMS policy before replay/application, including duplicates.

`Projection::apply` is a storage-independent reducer, not an authorization engine
or journal. Hosts persist the projection transactionally with their effects.
Matching event replay is a duplicate; conflicting reuse is an error. A late
accepted/starting/active event cannot overwrite a terminal result. Conflicting
terminal observations require reconciliation, not arrival-order guessing.
Message status and voice status are explicit read-only reconciliation operations.
Uncertainty cannot be cleared by retrying creation; reconcile by logical mutation
and external binding. Retain exactly the same logical mutation ID across restart.
The reducer stops at 256 retained events rather than evicting deduplication state.

The contribution fixture declares mono PCM signed little-endian 16-bit audio,
16 kHz, one 20 ms/320-sample frame per exchange, bounded queue and <=5 second
teardown. Two generated opposite-phase signals are encoded and decoded at the
fixture's peer boundary. This measures lifecycle and codec framing, not network
quality, real-time capacity, a supported carrier or video. Production media
adapters should use rvoip's session/transport/media interfaces behind this extension
and leave conversation, authorization, journal and route policy in Connect.

Host opt-in uses `Connector::message_extension` and `deliver_observed`; legacy
`deliver_attempt`/`deliver` compatibility remains single-attempt. Protected explicit
`Worker::reconcile` reads status under the original durable job ID and commits the
normalized receipt, full event receipt and actual source IDs with the reply.
Protected `Worker::observe` applies adapter-verified callback receipts; protected
`Worker::receive` admits authenticated independent contacts under the
configured route. These APIs are trusted composition, not wire installation or
principal creation. Telephone `VoiceReceipt` carries normalized evidence and the
existing private control capability together through explicit
`voice::Worker::with_observed_driver` composition. Builtin startup and `with_driver`
retain the existing carrier path. Host physical status observation
continues for cleanup after revocation without authorizing further mutations.
