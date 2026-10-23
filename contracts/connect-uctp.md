# Connect conversation profile / 2

Current shared control dispatch supports signed native QUIC and explicit
`POST /v2/commands` HTTP through the same service router. See
[canonical HTTP](../docs/developer/canonical-http.md) and its
[OpenAPI reference](openapi-canonical-http.json). Generic application entry points
refuse canonical Operation/`connect.request` dispatch; named application/task and
trusted administration APIs remain separate closed interfaces. Earlier
continuation status notes below retain their historical checkpoint scope.

The installed general Session control routes invite, accept, update and end,
with durable current appearances; physical orchestration remains unfinished.
Session invitation `to` uses a dedicated bounded selector type: current
Participant/Identity IDs or configured `tel:`/`sip:` references. URI resolution
requires a verified Endpoint on a current member's Device and the installed
host's exact gateway policy. The original invitation freezes its URI binding;
replay cannot redirect after address reassignment. Ordinary routing/resource
identifiers keep their existing alphabet and bound. Resolution grants no IO.
See [general Sessions](../docs/developer/general-sessions.md). Its
pending-invitation decisions route `session.reject` and
`session.cancel` through the durable service and current original source
authority. It preserves logical IDs, Participant assertions, optional topology
revision and original reasons. A target rejects its own invitation; a current
creator/manager cancels pending invitations without ending other established
appearances. Exact replay reports the original result under
current authority; it cannot rewrite an already decided invitation. These
commands do not imply Session readiness or established media termination.
See [pending invitation decisions](../docs/developer/session-decisions.md) and
the [current HTTP reference](openapi-canonical-http.json). G6-037 shared dispatch is qualified for its installed scope. G6-038 general
Session orchestration remains in progress.

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

`auth.session.identity_id` names the authenticated Identity. A Participant
is conversation-scoped, so `participant_id` is optional and requires an envelope
`cid` when supplied. An unscoped peer handshake does not fabricate a Participant
or create membership. Device metadata remains a claim until the installed policy
checks its ownership; authentication alone does not establish endpoint readiness.

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

Correlated observations of a mutating family (such as history's `message.send`
items) need no invented command operation ID or revision precondition. Incoming
correlated observations are authenticated and routed only to exact current scoped
waiters; they never enter command execution. The closed command exception is
`session.cancel` with a logical `operation_id` and an `in_reply_to` assertion of
this native physical caller's original invitation. Current signature, named
permission, assurance, membership/source/recipient authority and revision rules
still govern the ordinary cancellation transaction. The reference grants no
authority. Other correlated families remain observations. Unsigned HTTP rejects
native wire references and accepts ordinary logical cancellation commands.
An uncorrelated command retains all of its mutation and revision requirements.

Opaque cursor/page tokens use URI-unreserved ASCII with a separate 1–2048 byte
bound. They are not resource identifiers. History returns chronological
`message.send` observations followed by an `ack` with optional `next_cursor`.
Conversation snapshots expose an optional `next_page` for their current frozen
page set. Token syntax never substitutes for current Store authorization.

Authenticated handlers may return an ordered sequence of at most 128 application
envelopes and 2 MiB of encoded application JSON. The challenge advertises these
bounds. Each signed reply has a fresh ID and the original request correlation;
one owned current lease covers the entire writer submission. Step-up's fixed
session/assurance prefix precedes that bounded application sequence. Capacity
failure never silently truncates a history page or omits its final ACK.

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

An installed authenticated live-peer route supplies `auth.session.reachability`.
Its transport/contact come from trusted adapter context, with current verified
Identity/Device ownership and a bounded credential/physical lifetime. A signed
keepalive renews only that binding and returns current hints in
`ack.details.reachability`. A peer without an installed route reports an empty
list. A hint is operational availability evidence, not membership, media readiness
or delivery. Registry-routed submission returns only local writer admission;
uncertain writes retain their logical identity for reconciliation.

### Inline signature and step-up problems

Canonical v2 signature failures use code 401 with category `auth` and reasons
`signature_required`, `stale_timestamp`, `invalid_signature` or
`replay_detected`. Unknown keys, unsupported installed algorithms and failed
cryptographic verification share `invalid_signature`, without key identifiers.
Replay-store unavailability is 503 `replay_protection_unavailable`; exhausted
peer-lifetime capacity is 429 `replay_capacity_exhausted`. The physical peer
retains consumed envelope IDs across refresh and step-up without eviction.

A failed matching step-up completes the original mutation with 403
`assurance_not_established` and its response with 401 `step_up_failed`, each
correlated to its own envelope and the original logical operation scope.
Interruption once a handler may have started produces 409
`operation_interrupted` with `details.outcome: unknown` for the mutation and
408 `step_up_interrupted` for the response. These observations do not replace
querying the currently authorized durable operation outcome.

The explicitly installed canonical handler supports message.send/history,
conversation.get and durable-ledger operation.get/cancel/reconcile. Recovery
commands share current HTTP/SDK decisions: payload operation_id selects the target,
while cancel/reconcile top-level operation_id identifies a separate decision.
Optional CID/SID/Connection assertions must match the stored target; unsupported
expected-revision assertions are refused explicitly. On fresh schema 32 ordinary message receipts share the operation ledger and
include an exact get status reference and completed local-mutation boundary. Task
internal wire receipts retain their task/grant origin and are private to that
facade. Production listener/full command installation remains open.

Installed Store policy admission uses the same bounded refusal distinctions as
the handler: permission denial is 403 `auth/forbidden`, exhausted capacity is
429 `capacity/capacity_exceeded`, temporary authority contention is 503
`authorization/authority_unavailable`, and SQL availability failure is 503
`storage/storage_unavailable`. No refusal receipt admits a mutation; retry with
the same logical operation ID after recovery. This does not qualify availability
propagation from every authentication backend or cryptographic resolver.

The installed canonical handler maps current credential refusal to
401 `auth/authentication_failed`, resource denial to 403 `auth/forbidden`, invalid
input/cursor to 400 `validation/invalid_request`, an uninstalled feature to
501 `unsupported/unsupported_feature`, logical/revision conflicts to
409 `conflict/operation_conflict`, storage failure to
503 `storage/storage_unavailable`, and capacity refusal to
429 `capacity/capacity_exceeded`. Other application refusals use the bounded
400 `state/operation_rejected`. Replies retain request correlation and scope;
raw Store error text, credentials and private payloads are never echoed.

The schema-34 canonical receipt continuation adds `message.read` through the
ordinary original-recipient receipt engine. `by_participant` is a checked assertion;
`read_at` is diagnostic and cannot select evidence time or establish human viewing.
Native/HTTP/canonical reports share one logical ID and result. Subscription delivery
can emit a typed receipt hint with explicit channel/provenance/server time before
its cursor-bearing fact. The effective canonical page ceiling is 63 facts (a
requested limit up to 127 remains an upper bound), within 128 application envelopes
and 2 MiB. Current opt-in, original epoch, scope and source readers apply before
emission. Inbound `message.delivered` cannot forge a server observation. See
`docs/developer/canonical-receipts.md`; production native installation remains open.

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

The server exposes the same service through HTTPS/JSON. The target routes below
include the additional facade inventory owned by G6-065 and deployment routing
owned by G6-053. The installed canonical command endpoint is documented in the
[current reference](openapi-canonical-http.json); this target table does not
claim every route is installed.

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

The target simplified command body contains `type`, `payload`, optional resource IDs,
`operation_id` and `expected_revision`. The server supplies a fresh envelope ID,
timestamp and version; HTTP clients need not manufacture UCTP transport metadata.
The currently installed endpoint accepts an unsigned normative v2 envelope,
including its ID and timestamp; the public SDK supplies those transport fields.
The simplified body facade remains part of G6-065. HTTP authentication supplies
the principal and never accepts a body-selected actor.

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

## Implemented trusted provisioning subset

G6-030 implements the identity/endpoint administration subset through the shared
service. See [the provisioning OpenAPI](openapi-provisioning.json) and
[backend guide](../docs/developer/provisioning.md). Additional typed routes are
`POST /v2/conversations/{id}/participants`, `POST /v2/endpoints/{id}/verify`,
`POST /v2/endpoints/{id}/revoke` and `POST /v2/identities/{id}/revoke`.
Current admin, tenant and recipient authority is required on mutation and replay.
Endpoint provisioning alone does not assert ownership verification, SMS consent
or live media readiness. The remainder of this document remains the Gate 6 target
until its assigned implementation and qualification tasks complete.


Canonical transport backend composition is documented in
[`canonical-transports.md`](../docs/developer/canonical-transports.md). QUIC
requires actual `uctp/2` ALPN; HTTP/3 WebTransport uses one bidirectional
length-prefixed signaling stream; TLS WebSocket uses bounded JSON text messages.
Each canonical factory drives the same v2 authentication and lifecycle actor.
Configured HTTP Origin allowlists require exactly one matching Origin. Physical
drain stops admission, and explicit shutdown joins retained peer tasks. These
backend entry points do not by themselves qualify Connect server installation
or durable operation reconciliation.


The canonical secure backend enforces `uctp:session`, `uctp:data` and
`uctp:subscribe` command floors on retained authenticated claims, independently
of current resource authorization. Generic ACK/error replies inherit the
registered request's floor. Missing scope yields signed 403
`insufficient_scope`; payload scope assertions have no authority. Generic
operation lookup/cancel/reconcile requires current policy to resolve the original
operation and apply its original command scope and resource checks. Pre-auth
hello/response IDs use a separate non-evicting eight-entry fence consumed before
credential validation awaits; cancelled retries require fresh wire IDs.
Staged Connection media defaults to 16 cumulative new-port opens, including
failed/uncertain preparation batches. Reoffers, cleanup and logical stream end
cannot reset that budget. See [`secure-profile.md`](../docs/developer/secure-profile.md)
for the exact scope and remaining per-peer admission work.

Valid signed IDs count against the physical replay bound even when command scope
is denied. At capacity, a canonical peer refuses new IDs before shared verifier
work; no capacity refusal establishes signature validity or execution authority.


Physical-peer resource context is server-owned and persists across credential
refresh/step-up. Managed core admission defaults to 32 associated Sessions and
64 Connections, including pending and cleanup-quarantined routes. Confirmed
cleanup releases exact permits; EOF retires only the owning peer's routes and
preserves other peers in a shared Session. Quota metadata grants no resource
permission. See [`peer-admission.md`](../docs/developer/peer-admission.md) for the
managed boundary and [`peer-sessions.md`](../docs/developer/peer-sessions.md)
for reservation before zero-Connection Session creation. Explicit physical
views and Connection-backed Sessions share the same limit. The installed
current typed lease protects creation and attachment; the runtime origin
budget retains terminal IDs without replacing the journal. Invitation
delivery/acceptance and complete canonical engine/public service installation
remain separate open work. [Native preparation](../docs/developer/peer-native.md)
now reserves an exact never-reused Core ID and physical quota before a dormant
native adapter hook. Actual physical transport factories bind the accepted
server transport; retention grants no readiness or Participant presence.
Generic outbound/replacement integration must preserve the relevant physical
source and retained remote-leg ownership.

Canonical actor handlers may issue a same-peer scoped request/reply through
[the exchange capability](../docs/developer/peer-exchange.md). Commands and
auth transitions remain ordered, with bounded independent reply verification
and writer progress. Both directions require explicit current policy under
the held guard. Reply keys bind the full owner/key ID; ordinary signature,
freshness, global replay and physical replay checks still apply. A correlated
reply is an observation and cannot grant future media/mutation authority.

### Canonical native media resource integration

The actual canonical QUIC and WebTransport drivers now own one checked media
reader/router alongside signaling. Explicit trusted native RTP configuration
retains one staged controller on the Core-issued route, with frozen allocation
origin checks, shared non-reusing physical handles and cumulative binding history.
Core retirement fences that controller before asynchronous resource cleanup;
uncertain cleanup retains quota. WSS cannot opt into native datagrams.

This implementation increment does not change envelope families or advertise
completed media negotiation. The current fixtures use a controlled media policy
and readiness provider; the reserved ACK is not `connection.answer`,
`connection.ready`, Participant presence or a completed operation. Canonical media
command installation, production policy/credential rotation and managed graph
adoption remain open. See `docs/developer/canonical-media.md`.

### Held media control composition

An installed media policy can explicitly derive one bounded control lease from
the canonical invocation's existing current guard, checking exact action,
revision, streams, actor and Core resource scope. The default hook refuses this
path; it cannot fall back to a recursive grant read. Returning from the driven
handler cancels its derived engine-work token. Media publication and forwarding
still use fresh current policy and never inherit this control context. Engine
control waits and every media admission also enforce the earlier original
credential or task-delegation expiry, even while a grant guard remains held.

`docs/developer/media-control.md` records this engine/native composition seam
and its local fixture evidence. It does not add envelope families, advertise
completed media negotiation, install Connect durable dispatch, or implement
credential rotation in an existing controller.


### Terminal authentication retirement

When the physical authenticated binding expires or its current policy lifetime
retires, the peer closes its execution authority and managed physical routes.
A bounded signed `identity.assurance-changed` reports the verified previous
assurance, `current: anonymous`, and the factual retirement reason before
`auth.bye`. These unscoped observations name no fabricated Connection and confer
no lower-assurance execution permission. Specific key/delegation/Device/Identity
revocation reasons require the installed current policy's corresponding signal;
generic route cancellation remains `authority-retired`. Earlier credential/task
expiry determines its own reason. Old-incarnation signals cannot retire a
refreshed/stepped-up successor; an invalid candidate cannot install or finish
held execution under retired authority. An interrupted mutation may have an
unknown business outcome. Failed/partial writes do not guarantee observation
delivery. See `docs/developer/assurance-retirement.md`.


### Implemented signature algorithms and assurance floor

Canonical inline signing/verifying supports EdDSA and ES256. ES256 is P-256 with
SHA-256 and JWA fixed-width 64-byte R || S signatures; ASN.1 DER is rejected.
All verifier paths retain current owner/key/algorithm, validity/revocation, JCS,
freshness and shared replay checks. The challenge advertises the implemented
algorithm set and the existing `identity_assurance_required: identified` minimum.
Neither a signature algorithm nor a lower assurance assertion establishes a new
grant or membership. Fresh envelope/signature retry retains logical operation
identity. PS256/RS256 and HTTP-header signature processing are not advertised.
See `docs/developer/es256.md` and the reviewed G6-026 evidence mapping.


### Installed Connect response completion

The installed canonical Connect handler ends each successful command reply batch
with a correlated `ack`; refusals end with `error`. Conversation create/close ACK
`details.conversation` contains the authorized shared Conversation DTO and original
mutation receipt; list ACK `details.page` contains its shared bounded page, with
`next_cursor` also at the ACK payload level. Formerly fact-only Conversation get,
operation get/cancel/reconcile and Participant add/update/remove replies append an
ACK with `details.reply_complete: true`. Existing ACK-terminated history, message,
read-report and subscription responses keep their existing terminator. These use
the published open ACK details field and require no schema conversion.

The ACK completes that response batch, not an asynchronous logical operation or
media negotiation. Live subscription deliveries keep the subscription request
correlation and cannot complete another command. The signed Rust collector bounds
each batch and its separate retained inbox at 128 frames / 2 MiB, checks response
resource scopes, and retires the physical peer on overflow or incomplete exchange.
See [typed canonical calls](../docs/developer/canonical-calls.md).


### Default signed bootstrap

Fresh CLI `init` generates explicit TLS and separate server/client signing keys,
provisions each client Device and key under trusted offline administration, and
writes protected canonical config/pins before atomic no-replace publication.
Default `serve` accepts that canonical config and runs only the authenticated
canonical profile; prior server config and the interim `serve-canonical` command
are refused. No default parsing/profile downgrade is installed. This qualifies
only the current control listener; the earlier high-level SDK, remaining CLI
workflows and command/media families remain migration work. See
[bootstrap](../docs/developer/canonical-bootstrap.md).


### SDK application boundary and default command input

Default CLI `request` accepts SDK `canonical::Command` (typed `command` plus
`scope`), generates fresh signed envelope correlation, and returns its verified
canonical reply batch. It does not translate prior native Operation envelopes.
`request --application` uses a separate protected loopback HTTP config and a
closed whitelist of named application Operation APIs. Canonical command families
are rejected by that SDK constructor before I/O; unsupported native commands
never fall back to HTTP. Default health/diagnostics/resolve use the named HTTP
Service API. The application client enforces original expiry and one bounded
transaction deadline, response profile/type, byte limits, no proxy/redirect
forwarding, explicit task discovery and grant-only attenuation without owner
fallback. See [application SDK](../docs/developer/application-sdk.md).


The current signed dispatcher also admits `session.consult`, `session.handoff`
and `connection.replace` through the shared transition ledger and exact retained
peer proof. Canonical first admission selects one bounded operation expiry;
replay preserves the original expiry, including an HTTP-first request’s deadline.
An `operation.updated` accepted state and its terminating ACK do not establish
ready media or transition completion. See `docs/developer/canonical-transitions.md`.

G6-046 native record retrieval installs `recording.vcon-fetch` under current
`record_get` permission and exact Conversation/Session source authority. UUID/URL
selectors only address the local index. The response contains projected vCon
bytes and their exact digest; prior hashes and URLs grant no future access.
Participant ceilings, original epochs and Connection ownership remain source
checks. Capture diagnostics are separately authorized. See
`docs/developer/native-record-retrieval.md`; HTTP record custody, readiness
notifications and expiring references are not qualified by this native increment.
