# Connect control profile v1

Status: experimental, local Gates 1–3. Profile `connect-control/1`, UCTP envelope
version 1, raw QUIC ALPN `uctp/1`. These identifiers do not derive from crate
semver or the datagram wire version.

## Negotiation and authentication

The server advertises the profile in `auth.challenge.server_capabilities.application_profiles`.
Clients require it before sending `auth.response` with a bearer credential.
The coordinator retains a validated principal with `connect:control` scope.
The application rechecks its backing credential record on every operation and
inbox page, including already-authenticated connections and replays.

The application consumes only envelopes with this exact profile and type
`connect.request`. Replies use `connect.result` and `in_reply_to` set to the
request envelope ID. Unsupported profiles fail before mutation. Application
operations never also enter the stock core conversation handler. Stock UCTP
traffic without a profile retains its existing dispatch path.

## Operations

UCTP payload is `{ "profile": "connect-control/1", "op": "...", ... }`.
HTTP/JSON and the CLI accept the bare operation `{ "op": "...", ... }`;
the native SDK adds the UCTP profile wrapper. HTTP responses include the profile.

| Operation | Inputs | Outcome |
|---|---|---|
| resolve | address: `connect:alias@local` | permitted stable identity and static media declaration |
| capabilities | address, optional conversation_id | caller-permitted operations, configured routes, availability and limits |
| browser_binding | none | authoritative conversation, participant, optional session and signaling URL for this credential |
| create | request_id, participants: identity IDs | durable conversation; caller included |
| send | request_id, conversation_id, body: UTF-8 text | accepted receipt |
| history | conversation_id, after: opaque cursor or null, limit | ordered messages and next cursor |
| inbox | cursor: opaque cursor or null, limit, wait_ms | ordered journal events and last scanned cursor |
| request_status | request_id | accepted/rejected/not_found; saved outcome when permitted |
| close | request_id, conversation_id | closed conversation; retained history |
| health | none | installation ID, schema, profile; optional runtime readiness |
| provider_status | conversation_id | deliveries: job/message/connector IDs, state, nullable provider_id, sms_decision and sms_grant_id |
| call_start | request_id, conversation_id, route_id, destination: E.164 | durable call receipt |
| call_status | conversation_id, call_id | current durable call state |
| call_end | request_id, conversation_id, call_id | durable end intent |
| session_invite | request_id, conversation_id, participant_id, media | durable invitation inside the existing conversation |
| session_accept / session_reject / session_cancel / session_end | request_id, conversation_id, session_id, optional reason | idempotent durable session transition |
| session_get | conversation_id, session_id | current durable session state |
| sip_dial | request_id, conversation_id, session_id, destination: SIP URI | durable outbound SIP leg intent |
| sip_status | conversation_id, sip_leg_id | current durable SIP leg state |
| sip_end | request_id, conversation_id, sip_leg_id, optional reason | durable SIP termination intent |
| context_view | conversation_id, limit | authorized ordered context with source provenance, watermark, omission count, and bounded provider-delivery outcomes |
| context_deliveries | conversation_id, before: opaque cursor or null, limit | newest-first authorized delivery receipts and next cursor |
| availability_set | request_id, available/unavailable, optional preferred_route, expires_at | expiring self-owned observation |
| availability_get | identity_id | available/unavailable or unknown after expiry |
| artifact_begin / artifact_append / artifact_finish | request_id, conversation_id, artifact metadata/chunk, optional Unix `expires_at` on begin | resumable digest-verified artifact with bounded expiry |
| artifact_get | conversation_id, artifact_id, offset, limit | bounded reliable artifact chunk |
| artifact_delete | request_id, conversation_id, artifact_id | durable deletion and content erasure |

Send derives its sender from current authenticated credential state. Unknown
fields, claimed senders, invalid IDs and excessive limits fail validation.
All referenced identities must be in the caller's recipient scope. Conversation
membership is immutable at Gate 1; membership and credential scope both apply.
Unauthorized or unknown foreign resources return equivalent denials.

## Persistence and retries

A mutation's request_id identifies one logical operation, scoped to the stable
actor identity. Store a versioned SHA-256 digest of canonical JSON inputs and a
receipt atomically with its application effects. Same request + inputs returns
the stored result; different inputs conflict. Current authority is checked
before disclosing any cached result. Transport envelope IDs may change when a
caller retries; logical IDs must not. Reads use fresh envelope IDs.

Accepted means stored durably with recipient journal events, not read or
processed. A timeout/cancellation does not undo a commit. Not found while a
request may be in flight does not justify minting a new logical request ID.
Retry the original operation or query its result.

Inbox notifications are wake hints; the SQLite journal is authoritative.
Cursors are opaque, installation- and identity-scoped, authenticated tokens.
Delivery is ordered and at least once. Clients persist handled event IDs and
cursor after durably accepting work; apps deduplicate their own effects.
Wait is bounded and cancellation-safe. Capacity errors never imply acceptance.

## Errors and limits

Application error codes: unauthorized, forbidden, invalid-request,
unsupported-profile, conflict, conversation-closed, capacity-exceeded,
storage-unavailable. Transport errors/timeout remain distinguishable.

Defaults: 16 KiB/message; 100/page; 8 participants; 100,000 journal events;
100,000 retained operation receipts; 64 MiB retained message bodies; 5 s inbox wait; 10 s client deadline;
64 simultaneous peers per listener; 10,000 sessions; 16 MiB per artifact and 256 MiB total non-expired declared artifact bytes;
32 KiB decoded artifact chunks; 256 outbound/coordinator queue (upstream).
No automatic history expiry at Gate 1. Capacity is serialized with admission.
Disk-full/IO failure must roll back and return failure before acceptance.

Connect owns durable application state. rvoip owns transient sessions, live
media bindings, authentication and wire framing. No duplicate in-memory core
conversation projection is created for these application commands.

Receipt capacity includes authorized rejected operations. At that limit a new
logical request fails before mutation, has no saved receipt, and request_status
reports not_found. Existing receipts remain replayable under current authority.

Read pages additionally cap serialized entries at 512 KiB to fit the existing
1 MiB UCTP frame limit. The requested count is an upper bound; paginate until
an empty page. Cursors never advance past an unread authorized entry.

The optional WS parity endpoint is plaintext loopback-only and carries the
same application profile through rvoip coordination. It is not native QUIC or
a provider/WebRTC media endpoint. Wire fixtures, including result shapes, live
in contracts/fixtures and run through contract_fixtures and transport_parity.

## Provider messaging and telephone control

All operations share the same authorization, UCTP profile, request/reply
correlation, and durable conversation journal. Provider credentials and private
call-control URLs stay server-side. A voice route binds an assistant and provider
number to the permitted conversation/identity; a scoped voice grant authorizes
outbound destinations. Caller ID on an inbound telephone call is an observation,
not authentication or permission to act as a Connect identity.

`send` acknowledges durable local acceptance. `provider_status` separately reports
provider delivery; failed/uncertain work is not silently resubmitted. Vapi chat
context is persisted. Telnyx SMS uses programmable messaging and signed callbacks;
live carrier delivery remains deferred pending approval.

Gate 3 context delivery records expose only bounded state: delivery ID, Session
ID, provider name, selected watermark, terminal state, an opaque provider
reference, and timestamps. They never expose a provider WebSocket URL,
credential, tool payload, or hidden assistant configuration. The canonical
selected source items remain independently authorized by `context_view`.

Final Vapi transcript events are appended to the same durable conversation as
`voice_transcript` context items. The bound Session selects the conversation and
maps provider `user`/`assistant` roles to its initiator/invited participant; the
provider cannot supply an identity or conversation ID. Partial transcripts are
not retained. Repeated delivery of the same final transcript or end report is
idempotent. If a stable provider reference is redelivered with different
permitted content, Connect appends a `voice_transcript_correction` or
`voice_outcome_correction` whose JSON body names the corrected source ID. It
never replaces the original observation.

A `call` result has id, conversation_id, route_id, destination, state, direction,
end_requested, nullable provider_id and nullable ended_reason. Start acceptance
is not proof of ringing or audio. Replaying start/end returns the immutable saved
receipt; query `call_status` for fresh state. End intent remains durable while
termination is being reconciled. Ambiguous provider creation is marked uncertain
and never automatically redialed. Recovery reconciles calls with known IDs.

Authenticated provider polling discovers inbound calls and advances a durable
watermark only after the whole batch imports successfully. Journal events include
an optional `call`; consumers must handle events with `message: null`. Import is
deduplicated by provider ID. Status visibility is eventual, not a live media feed.

The Gate 2 qualified telephone path is handset ↔ Telnyx ↔ Vapi. Connect provisions and
controls that route and records its lifecycle; the audio does not pass through
Connect, QUIC, or a Connect WebSocket. A Telnyx SIP → Connect → Vapi audio bridge is
separate Gate 3 route. Its local SIP↔Vapi and SIP↔WebRTC media paths are
implemented; real Telnyx/PSTN reachability still requires recorded live proof.
Local QUIC or SIP loopback tests do not prove that provider route.

## SDK behavior and compatibility

Python and TypeScript validate response profiles, correlation, outcome kinds and
required fields. Malformed/oversized responses raise `protocol`; connection loss
or deadline expiry raises `transport-uncertain`. Application error codes are
preserved. A native helper protocol/transport failure closes that client; open a
new client and recover using the original logical request ID. No automatic
mutation retry occurs. Concurrent native requests are serialized; each exchange
has its own deadline after reaching the front of the queue. Responses are bounded
to 600,000 bytes, requests to 64 KiB; inbox wait must fit the configured deadline.

This profile is experimental. Additive operations require a matching server;
older servers reject unknown operations, without silent transport fallback.
Readers tolerate additional outcome fields and unfamiliar state strings, but
require known outcome kinds and required fields. Request fields remain strict.
Incompatible semantics or removal of fields requires a new negotiated profile,
not merely a crate version bump. Fixtures cover every operation, rejection of
extra request fields, every outcome kind, and inbound call events. Rust, Python
and TypeScript consume the same result fixtures.

## Gate 3 conversation and session rule

A conversation is the durable context boundary. A session is a bounded live leg
inside it. Starting a new voice, video, data, WebRTC, SIP/PSTN or provider leg does
not create a new conversation. Only an explicit `create` operation does. An
ambiguous inbound address remains unresolved until application policy chooses a
conversation; caller ID is never sufficient authority to merge history.

Session states are `invited`, `accepted`, `ready`, `rejected`, `cancelled` and
`ended`. A ready Session returns to accepted when all live bindings disappear;
the journal records the interruption rather than pretending a socket survived.
The first implementation admits one invited participant per session;
the surrounding conversation can retain its full participant set. Live rvoip
connections and streams are disposable projections. SQLite session intent,
events, terminal outcomes and context provenance remain authoritative after a
restart. A restart cannot claim to restore a socket or media pump.

Availability is an expiring observation, separate from declared identity
capabilities and verified transport reachability. Expired or absent state reads
as `unknown`; Connect does not silently choose another route. Artifact chunks are
padded base64 only on the bounded control/HTTP compatibility surface. They are
not an audio transport. Native QUIC stream transfer and datagram audio use UCTP
media facilities and keep their own framing and flow control.

## Discovery and context inspection (pre-Gate 4 repairs)

`capabilities(address, conversation_id?)` requires existing `resolve` authority.
It returns only caller-permitted operations, configured routes, actual bounded
message/file/audio limits and (with `availability_get`) the target's expiring
availability. `configured` means the installation has that route configured;
it does not promise an active worker, reachable peer, carrier delivery or SMS
eligibility. Video is not advertised by this implementation. The legacy
`resolve.capabilities` field remains a static identity declaration.

`browser_binding()` requires `session_get` and the exact configured WebRTC route
credential. The returned conversation, participant and signaling URL are the
server's binding. The reference client compares these with its selected
conversation/participant before acquiring a microphone.

`context_deliveries(conversation_id, before?, limit)` requires `context_view` and
current conversation authority. It returns newest-first receipts, a signed,
identity/conversation-bound older-page cursor, and explicit `next: null` at the
end. Insertion order is stable across outcome updates and restart. `context_view`
now includes the newest 100 receipts and `deliveries_omitted`, independently of
source-item `omitted`. Diagnostic pagination does not enlarge model context.

## SMS eligibility (schema 10)

A configured SMS route is not consent. At dispatch, Connect requires an active,
unexpired, reviewed grant matching the exact recipient, sender number and
conversation, plus a fresh authenticated authority snapshot and no local STOP.
The trusted enrollment integration is a server boundary, not an agent operation.
SDK, HTTP and QUIC message submission all use the same durable outbox check.
`provider_status` exposes nullable `sms_decision` and `sms_grant_id` evidence.
A message accepted into Connect can therefore have a cancelled SMS delivery;
acceptance is not a promise of provider dispatch.

Snapshots refresh every 30 seconds of worker availability and have a 90-second
lease. Process startup invalidates old leases. Missing, stale or malformed
snapshots cannot grant access; revoked approvals disappear on the next successful
refresh and, on outage, the old lease expires. Revisions cannot go backwards or
change content at the same revision. Enrollment records are immutable and
retained for audit. Operators must synchronize clocks.

Verified Telnyx STOP callbacks persist independent suppression and cancel pending
SMS jobs. The provider's `occurred_at` orders controls; STOP wins timestamp ties.
HELP is recorded without changing permission. START/UNSTOP require prior
recorded enrollment; they cannot create it, override task expiry, or revive a
cancelled message. Replayed provider events are idempotent. Signed provider
`autoresponse_type` classifications cover custom control phrases. Telnyx sends
its configured control acknowledgements; Connect does not duplicate them.
Already-claimed/in-flight provider requests cannot be recalled by a later STOP.
Unknown send outcomes remain uncertain and are never automatically resubmitted.

Rudeless is the first enrollment authority. Its form records evidence, an
administrator reviews a task-specific activation, and its authenticated feed
publishes approved grants. Connect owns recipient suppression and dispatch
history. A fresh form submission never clears STOP. Other applications can
implement the same snapshot contract without adopting the Rudeless website.


### Additive operational health observations

The authenticated `health` result may include `readiness` with `state` and
`components`. Each component has `component`, `state`, `reason`, and Unix-second
`observed_at`. State is `ready`, `degraded`, `unavailable`, or `uncertain`; reasons
are stable nonsecret codes. No components means uncertain. Older servers may omit
this field. This is a sampled operational diagnostic, not a durable event,
authorization decision, delivery acknowledgement or end-to-end reachability promise.
SIP observations expire with the registrar's accepted lease even if a stale coarse
state still says registered. Worker failures are not silently retried.
