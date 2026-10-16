# Connect conversation profile / 2

Status: Gate 6 implementation contract. Owner authorized implementation on
2026-10-15. This document defines the target; it is not a claim that the running
server already supports it. G6-003, G6-007 and G6-010 own this contract.

## Target and dispatch

The baseline is rvoip `f4d528eaabbac935bce9a6404d3ce4fdec35ed0f`,
`docs/CONVERSATION_PROTOCOL.md`. The Gate 6 normative ledger records baseline
requirements and explicit corrections. The target is **UCTP envelope version 2**,
native ALPN **uctp/2**, and negotiated application profile **connect/2**. The
unchanged RTP datagram framing remains version 1, `rtp-datagram/1`; envelope and
media versions are independent. HTTP/3 uses ALPN `h3`.

No v0.1 Connect API, old SDK or `connect.request` compatibility is required.
An unsupported envelope/profile fails explicitly before effects. Experimental
AAuth and reserved federation are not advertised as available. Authentication
methods are discovered; the protocol describes conditional method semantics,
not a requirement that every installation run every identity backend.

`auth.hello.application_profiles` offers profiles. `auth.session.application_profile`
selects one offered profile. The authenticated peer retains that selection.
Connect selects `connect/2` and routes canonical conversation commands through
its service, authorization and durable store. rvoip supplies negotiation/media
mechanics. A generic core handler cannot also execute a command accepted by
Connect or create a second authoritative Conversation.

All 49 baseline envelope families remain in the coverage ledger. Recognition
alone is insufficient. Required behavior must pass its assigned integration
vectors; optional observations such as active-speaker events remain optional
only where the source contract says so.

## Envelope

An envelope has `v`, `type`, `id`, `ts`, `payload`, and optional `cid`, `sid`,
`connid`, `in_reply_to`, `operation_id`, `expected_revision`, `signature`.
`v` is 2. IDs are 1–128 bytes of URI-unreserved ASCII; UUID-based prefixed IDs
are the default generator. ULIDs remain valid bounded opaque IDs, not required
wire encodings. The local/federated identity distinction remains meaningful;
reserved federated identities cannot be silently interpreted as local identities.
Timestamps are diagnostic and never establish authoritative order.

`id` identifies one fresh transport envelope. `operation_id` identifies one
logical mutation and is required on mutating commands. `expected_revision` is
required for topology or membership changes to existing resources. New-resource
creation is protected by its operation identity. Responses correlate using
`in_reply_to`; operation progress additionally names `operation_id`.

Revisions are nonnegative integers no greater than 9007199254740991, so all SDK
languages and canonical JSON agree exactly. Conversation membership changes use
the Conversation revision; consult/handoff/device replacement and session updates
use the main Session topology revision; connection/stream routing updates use the
Connection revision. The command registry specifies the revision scope.

Unknown message types and unsupported features produce a bounded explicit error.
Unknown optional fields are ignored for execution, preserving UCTP's forward
compatibility seam. Signature verification still covers all fields present in
the original envelope before typed normalization. Privileged actor, tenant,
grant and audience assertions cannot be introduced through ignored metadata.
Duplicate JSON keys, invalid UTF-8, invalid identifiers, nonfinite numbers and
oversized payloads are rejected before dispatch. Default reliable envelope cap
is 1 MiB including framing-independent JSON bytes.

The actor, tenant, device and effective authority come from the authenticated
principal. Presentation fields such as `from`, `by` or `by_participant` cannot
select an actor. A supplied conflicting assertion is rejected; omitted assertion
fields are supplied from the authenticated binding where an engine needs them.

## Canonical application mapping

| SDK operation | UCTP command | Result / observation |
|---|---|---|
| conversations.create/list/get/close | conversation.create/list/get/close | opened, snapshot, closed |
| conversation.participants.add/update/remove | conversation.participant.add/update/remove | operation.updated and participant.changed |
| conversation.send/history | message.send/history | accepted message, recipient receipts, authorized history |
| conversation.subscribe | conversation.subscribe | snapshot and conversation.event cursor stream |
| sessions.invite/accept/reject/cancel/end | session.invite/accept/reject/cancel/end | accepted operation, started/ended facts |
| session.join | session.invite with existing sid, then accept | participant.joined and ready connection |
| session.leave/upgrade/changeRole | session.update with typed kind | operation.updated and authoritative session facts |
| session.consult | session.consult | operation.updated; private media/audience facts |
| session.handoff | session.handoff | operation.updated; responsibility and route transition |
| connection.replaceDevice | connection.replace | operation.updated; retained/replaced connection lineage |
| connection.negotiate/hold/resume/mute/end | connection.offer/answer/update/end | ready/update/end facts |
| stream.subscribe/unsubscribe | stream.subscribe/unsubscribe | accepted routing change and stream facts |
| operation.get/cancel/reconcile | operation.get/cancel/reconcile | authorized operation snapshot / updated |
| session.records.get | recording.vcon-fetch | authorized recording.vcon-fetched projection |

`session.update` kinds are typed: `participant-leave`, `role-change`,
`medium-upgrade`, `codec-renegotiate`. Session leave does not remove conversation
membership. Membership removal is a distinct authorized transition.
`connection.update` actions include hold/resume/mute/unmute/codec-renegotiate;
they cannot bypass audience or transition checks.

Generic read/admin/task extensions remain explicit application APIs. They are
not represented by an unbounded arbitrary-operation escape hatch.

## New canonical types

The baseline registry is extended with:

- conversation.get, conversation.participant.add/update/remove/changed;
- conversation.subscribe, conversation.snapshot, conversation.event;
- session.consult, session.handoff, connection.replace;
- operation.get/cancel/reconcile/updated;
- session.observation, carrying a typed transcript, speech or action-request observation.

These are shared semantics; no `handoff_to_browser`, `move_to_phone` or
travel-specific operation enters the shared protocol. Endpoint kinds and
capabilities select the appropriate adapter.

## HTTP server contract

The server exposes the same service through HTTPS/JSON. The target routes are:

| Method and path | Scope |
|---|---|
| POST /v2/commands | Typed canonical command, same service and operation ledger |
| GET /v2/conversations/{cid} | Authorized snapshot |
| GET /v2/conversations/{cid}/events | Cursor pagination / bounded long poll |
| GET /v2/operations/{operation_id} | Status for the authenticated operation owner or explicit delegate |
| POST /v2/operations/{operation_id}/cancel | Authorized cancel command with its own mutation ID |
| GET/PUT /v2/conversations/{cid}/artifacts/{artifact_id} | Authorized bounded file transfer |
| POST /v2/identities | Trusted provisioning only |
| POST /v2/identities/{identity_id}/endpoints | Trusted endpoint provisioning |
| POST /v2/identities/{identity_id}/credentials | Trusted scoped credential installation |
| GET /v2/health | Minimal process liveness, no private state |
| GET /v2/readiness | Authenticated component readiness |

The command body contains `type`, `payload`, optional resource IDs,
`operation_id` and `expected_revision`. The server supplies a fresh envelope ID,
timestamp and version; HTTP clients need not manufacture UCTP transport metadata.
HTTP authentication supplies the principal and never accepts a body-selected actor.

Accepted asynchronous mutations return HTTP 202 with a queryable operation;
synchronously completed operations may return 200/201. Validation, auth,
forbidden, unknown resource, state conflict, expiry, size/rate and unsupported
capability failures have stable typed bodies. Provider uncertainty is a queryable
operation outcome, not a hint to retry with a new ID. Canonical integer error codes
use machine-readable reasons; `401-1` is represented as code 401 and a reason,
not as an invalid fractional/integer wire value.

Provider webhooks are separately authenticated adapter routes, never ordinary
participant commands. OpenAPI must enumerate all supported routes and be checked
against actual server responses. The standalone HTTP reference includes runnable
curl examples, auth/scopes, errors, cursors, cancellation, files and limits.
Live media travels over negotiated UCTP/WebTransport/WebRTC paths.

## Execution authority

The Gate 6 plans are approved. Schema/profile changes are allowed. A plan or
contract acceptance is not a claim that runtime, remote, provider or media tests
have passed. Record those in their assigned evidence lanes.
