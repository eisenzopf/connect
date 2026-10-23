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
principal creation.

Telephone `voice::Worker::with_driver` requires a declared, validated manifest
before recovery or claims. It always uses typed `VoiceReceipt` evidence and the
private control capability; there is no alternate unnormalized dispatch mode.
Missing operations are refused before provider I/O. Invalid logical IDs are
refused before transport, and a normalization failure after possible transport
is uncertain, including a decoder's local rejection. Restart cannot redial an
uncertain creation. Status queries are attributed as reconciliation and retain
the original logical ID. Host physical status observation continues for cleanup
after revocation without authorizing further mutations.

Read-only telephone discovery returns `InboundVoiceReceipt` with authenticated
external call data, typed state, stable event ID and verification provenance. It
has no local mutation ID: the authorized Store import assigns that ID and binds
the projection, call and journal in one transaction. Current ingress credential,
route, assistant/phone binding, caller allowlist and conversation policy precede
duplicate handling. Invalid or local-attempt-only discoveries cannot create a
call. Duplicate discovery never dials or advances the existing lifecycle; the
owned status poll handles later observations. Failed batches preserve the scan
cursor for a read-only retry. Caller ID and provider IDs remain correlation data.

The canonical `canonical_provider_voice::Worker` now applies these normalized
contracts to ordinary Session invitations. Protected `provider_voice.ingress`
configuration selects an explicit existing Conversation/AI/telephone audience;
caller numbers never create principals, accept Participants or grant history.
Incoming direction and `remote_identity_verified: false` make that distinction
visible. A confirmed read-only observation effect binds the existing provider
resource without a VoiceStart. Session admission, observation/projection,
original audience journal and durable scan cursor commit atomically. Duplicate
incoming observations never redial or update an existing lifecycle; current
policy/epochs precede replay. Failed batches retain the original cursor.
Configured ingresses without VoiceDiscover remain dormant while independent
outbound/status/end capability stays available. VoiceDiscover/status/end without
VoiceStart supports incoming observation and cleanup while rejecting outbound
creation. The canonical `voice_flow` fixtures cover all five original functional
requirements through the public signed/HTTP Session SDK; they are synthetic
provider adapters, not real provider IO or media qualification.

`VoiceReconcile` is a separate optional read-only capability requiring
`VoiceStatus` and `VoiceEnd`, independently of `VoiceStart`. Its bounded query
preserves the original mutation, full assistant/phone/destination binding and
original time window. Only an authenticated unique correlated resource may
return `VoiceReceipt` with explicit reconciliation provenance. Missing, ambiguous,
truncated or failed lookups convey no absence proof and permit no retry of
creation. An adapter must not select a resource by Caller ID alone. The Vapi
profile sends an opaque correlation in the call name and rechecks the complete
binding on an authenticated fresh Get after bounded exhaustive list lookup.
Quarantined resource collisions require separate typed resolution. The host's
protected `ResolveQuarantine` dispatch owns an original-conflict digest ticket;
it uses `VoiceReconcile` IO without permitting another create. Only explicit
authenticated reconciliation of a distinct, unclaimed original resource can
commit the immutable resolution proof with effect/projection/journal updates.
Original conflict history remains immutable. Missing, unverified, ambiguous or
still-claimed results leave quarantine unchanged; an ordinary reconciliation or
operator assertion cannot clear it. Replays and competing captures retain the
first resolved resource and never reopen later terminal state. This does not
authenticate Caller ID or authorize Session acceptance or media.

These telephone worker contracts do not expose the retired application call
commands through the canonical API. Stock public provider installation and the
full generic operation/endpoint integration remain G6-044 work; a backend fixture
is not evidence of a public telephone workflow. Changes to receipt decoding,
uncertainty and discovery require the outbound/inbound, lost-response, duplicate,
cancel/restart and teardown live telephone proofs to rerun with the concrete
G6-093 configuration. Default verification remains isolated fixtures.

Telephone network requests require an owned `VoiceDispatch`, not a boolean
admission check. The current original transport interval stays held through the
bounded request, including credential/task/grant expiry. A consumed permit
captures the attributed result and releases transport readers before its atomic
metadata settlement. Immediate record attempts may retry only that same captured
result after contention or principal withdrawal. One worker owns recovery in an
installation runtime; overlapping starts are refused until its tasks and admitted
blocking mutations drain. See [telephone custody](../../docs/developer/telephone-provider-custody.md)
for the protected API and unqualified public/live-provider scope.

Canonical messaging selects explicit immutable route/verified Endpoint/account
bindings through protected `provider_messages` configuration. The generic worker
settles all adapter attempts through typed message observations, including built-in
Vapi and Telnyx adapters. Account references, current Endpoint/device facts and
Participant epoch are checked before context export and actual transport admission.
The narrow signed Telnyx callback listener is scoped to mounted SMS routes. Early
finals require an attributable signed number pair, and reconcile the original
job when its authenticated send response arrives; provider IDs cannot select an
unmounted route. Signed opt-out for an unknown contact to an owned mounted source
is still retained without granting enrollment or assistant access. See
[canonical messaging configuration](../../docs/developer/canonical-provider-messages.md).
