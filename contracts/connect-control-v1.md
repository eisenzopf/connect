# Connect control profile v1

The native framing below is historical Gates 1–5 context. Current default native
commands use [UCTP/connect 2](connect-uctp.md). The existing bare HTTP Operation
DTOs remain only the closed named application extension API under their
`connect-control/1` response profile. Canonical commands use the current
[`POST /v2/commands`](openapi-canonical-http.json) envelope/batch contract and
`connect/2` profile. The new SDK admits an explicit application subset rather than using it
as a canonical-command fallback. Default CLI request input is a typed canonical
command, or an explicitly selected HTTP application operation. See the
[application SDK contract](../docs/developer/application-sdk.md). Earlier native
SDK/CLI consumers still await migration; this file does not qualify those paths
against the current default listener.

Historical framing status: experimental local profile. Profile `connect-control/1`, UCTP envelope
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

## Historical operation inventory

The table below is Gates 1–5 inventory, not the current application allowlist.
Canonical conversation/message/membership/operation/Session controls and old
provider call/SIP mutations are refused by generic application entry points.
Current named extension admission is the shared closed
`connect_protocol::application_api::response_kind` classification.


UCTP payload is `{ "profile": "connect-control/1", "op": "...", ... }`.
HTTP/JSON and the CLI accept the bare operation `{ "op": "...", ... }`;
the native SDK adds the UCTP profile wrapper. HTTP responses include the profile.

| Operation | Inputs | Outcome |
|---|---|---|
| resolve | address: `connect:alias@local` | permitted stable identity and static media declaration |
| capabilities | address, optional conversation_id | caller-permitted operations, configured routes, availability and limits |
| browser_binding | optional paired conversation_id and session_id | configured default or currently authorized accepted AV Session and signaling URL |
| create | request_id, request: tenant_id assertion, policy, idle_close_secs, metadata, initial_participants (identity_id, role) | durable current conversation with local mutation receipt; caller included |
| conversations | query: filter, cursor, limit | bounded current conversation page and next_cursor |
| send | request_id, conversation_id, message: explicit typed draft | accepted receipt |
| history | conversation_id, after: opaque cursor or null, limit | ordered messages and next cursor |
| inbox | cursor: opaque cursor or null, limit, wait_ms | ordered journal events and last scanned cursor |
| request_status | request_id | accepted/rejected/not_found; saved outcome when permitted |
| close | request_id, conversation_id, options: expected_revision, reason_code, reason | closed current conversation and local mutation receipt; retained history |
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
eligibility. Gate 5 source advertises native VP8/PCMU AV and the configured
browser pair, under the [video profile](connect-media-vp8-v1.md); final media
qualification is still required. The legacy `resolve.capabilities` field remains
a static identity declaration.

`browser_binding()` requires `session_get`. Without IDs it retains the configured
route default and its exact audio credential policy. Gate 5 adds paired
`conversation_id` and `session_id` fields to select a currently accepted/ready
AV Session belonging to the configured identity pair in either invitation
direction. Both the configured and joining credentials retain current authority;
IDs never grant access. Partial, wrong-context, terminal or unauthorized selection
is denied. The returned peer, Session and signaling URL are authoritative. The
reference compares all selected IDs before acquiring camera or microphone.
A new accepted invitation can be selected without server configuration/restart.
The same paired fields accompany authenticated WebRTC `offer-ready`; see the
[media contract](connect-media-vp8-v1.md) for bounds and admission.

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

## Exact native reliable-control framing (Gate 5 protocol kit)

This section defines the wire input for an independently authored implementation.
It describes the deployed rvoip UCTP envelope and Connect extension separately;
it does not declare Connect extensions an adopted external standard.

Raw QUIC runs over UDP with TLS 1.3 and ALPN `uctp/1`. The client verifies the
server certificate against an explicitly configured trust anchor and verifies
the configured DNS server name (`localhost` for the generated local certificate).
The installation exports that trust anchor as DER. Convert it to PEM if your
runtime requires PEM; disabling verification or falling back to another transport
is forbidden. Local qualification listens on loopback only. Neither WebTransport
ALPN `h3` nor the optional WebSocket transport is this native proof.

After QUIC establishment the client opens one bidirectional reliable stream.
Both directions contain consecutive frames: **four unsigned big-endian bytes
of JSON byte length, followed by exactly that many UTF-8 JSON bytes**. The prefix
excludes itself. Maximum JSON frame length is 1,048,576 bytes. Reads must handle
split/coalesced packets; a packet boundary is never a frame boundary. A truncated
frame, invalid JSON or missing required envelope field terminates the stream.
An oversized declared length is rejected before allocating that payload. The
public helper/client additionally admits requests at most 65,536 bytes and
responses at most 600,000 bytes. Reliable control does not carry audio/video frames.

Every envelope requires `v: 1`, `type` (dotted string), `id` (string), `ts`
(RFC3339 timestamp with timezone), and `payload` (JSON object for this profile).
Optional `cid`, `sid`, `connid`, `in_reply_to`, and `signature` can be omitted.
These are UCTP envelope fields; unknown envelope fields are currently tolerated.
`signature` belongs to the optional upstream RFC9421 facility and is not required
for the configured Connect bearer profile. No client-side signing key is needed
for this proof. Envelope IDs and reply correlation IDs are 1–128 URI-unreserved
ASCII characters (`A-Z a-z 0-9 . _ ~ -`); the canonical generated form is
`env_` plus a UUID without hyphens. Invalid IDs are silently dropped before
correlation; clients must bound the resulting wait. Sender timestamps never
establish event order. Unsupported `v` gets UCTP `error` code 505.

Handshake, each with a fresh envelope ID:

1. Client sends `auth.hello`, payload
   `{"device":{"id":"example","kind":"desktop","platform":"darwin","sdk_version":"1"},"auth_methods":["bearer"],"capabilities":{}}`.
2. Server replies `auth.challenge`, `in_reply_to` = hello ID. Payload contains
   `nonce`, `accepted_methods`, `server_capabilities.application_profiles`.
   Require `bearer` and `connect-control/1` before disclosing a credential.
3. Client sends `auth.response`, payload `{"method":"bearer","credential":"<private bearer>"}`.
   For this deployed bearer method the nonce is informational, not a signed
   challenge response; `in_reply_to` on this client envelope is unnecessary.
4. Server replies `auth.session`, `in_reply_to` = response ID, with
   `identity_id`, `participant_id`, `session_token`, `expires_at`, `assurance`
   and optional `reachability`. Failure is an `error`, typically code 401.
   Session tokens are private; the independent client reconnects by reauthenticating
   its original backing credential and does not assume a session token grants access.

An application envelope is `type: "connect.request"`, with the operations above
inside `payload`, always including `profile: "connect-control/1"`. When supplied,
`cid` must equal the operation's conversation ID. The server returns
`type: "connect.result"`, `in_reply_to` = request envelope ID, and preserves the
request's optional `cid`. Result payload is exactly one of:

```json
{"profile":"connect-control/1","result":{"Ok":{"kind":"accepted","data":{"request_id":"logical-1","message_id":"message-1","conversation_id":"conversation-1"}}}}
```

```json
{"profile":"connect-control/1","result":{"Err":{"code":"forbidden","message":"Operation or resource is not permitted"}}}
```

A `result` has one variant only; error `code` is the stable programmatic value.
Peer-controlled error messages must not be logged. The profile and correlation
must match before accepting the outcome. Required known outcome fields remain
mandatory while additional outcome fields and unfamiliar state strings are
forward compatible. Unknown operation/fields yield `invalid-request`; an unknown
application profile is UCTP `error` 501 (`unsupported-application-profile`), rather
than a stock conversation mutation. UCTP error payload includes numeric `code`
and `reason`; preauthentication/current-principal failures use 401/403. UCTP
errors remain distinguishable from application `Err` and uncertain transport loss.

Control outcomes used by the independent client are `identity` (`id`, `alias`,
`capabilities`), `conversation` (`id`, `participants`, `closed`), `accepted`
(`request_id`, `message_id`, `conversation_id`), `history` (`messages`, `next`),
`inbox` (`events`, `cursor`), `request_status` (`state`, nullable `outcome`), and
`health` (`installation_id`, `schema`, `profile`, optional `readiness`).
Complete application result examples live in `contracts/fixtures/results/`;
operation shapes live in `contracts/fixtures/`. Task shapes/versioning are in
[connect-task-v1.md](connect-task-v1.md); a task remains an additive Connect
extension inside this outer framing.

Logical mutation `request_id` is 1–128 ASCII letters/digits/hyphens/underscores,
owned by one stable actor. It is not an envelope ID. Preserve it across timeout,
reconnect and restart; use fresh envelope IDs. Changed input conflicts; identical
replay returns the original immutable result only under current authorization.
Use `request_status` to inspect uncertainty; no `not_found` observation permits
silently changing the logical request ID.

Durable inbox payload is `{"events":[...],"cursor":null-or-string}`. Each event
has `id`, `kind`, `conversation_id`, nullable `message`, and its own opaque `cursor`.
Optional `call`, `session`, `artifact` carry their respective lifecycle outcomes.
A message has `id`, `conversation_id`, `sender`, `sender_participant_id`,
`audience` (frozen participant IDs and membership epochs), `content_type`,
`body`, `body_encoding`, `label`, `attachments`, `in_reply_to_msg`, and
`accepted_at`. Server authority selects the sender. Sends require a `message`
draft with `to` (explicit participant IDs or `"all"`), content type, body,
encoding, label and attachments; optional `msg_id` and `in_reply_to_msg` identify
the message and parent. Body-only sends are rejected. The Rust `send`, TypeScript
`send` and Python `send` helpers construct a plain UTF-8 text draft to `"all"`.
Use `send_message` (TypeScript `sendMessage`) for selected recipients and typed
content. See [addressed messages](../docs/developer/messages.md). Process events in returned order and persist event IDs/cursor only after
handling is durable. The same cursor may replay events. It is installation- and
identity-bound and invalid after foreign identity use; never parse or construct
it. Resume by passing it unchanged to `inbox`. Notifications are only hints;
this reliable-control client polls the journal and does not require a push channel.
Server restart retains journal/receipt state, not a transport session.

Framing vectors in `contracts/fixtures/wire/` include exact byte prefixes and
positive/negative envelopes. They contain synthetic values and repository MIT
license provenance. Media packets have a separate datagram profile and negotiated
bindings; these framing vectors and the independent client prove no media codec,
rendering, WebRTC, SIP or provider behavior.

## Gate 6 participant endpoint selection

The current shared facade also accepts `participant_route` with a nested typed
`request`. Conversation-scoped capability discovery returns `participant_id`,
which is distinct from the Identity ID. The Rust SDK can resolve that appearance
and select permitted endpoints without a provider route ID or destination address.
See the [routing guide](../docs/developer/routing.md) and implemented
[HTTP/OpenAPI subset](openapi-routing.json) for exact fields and prerequisite,
preference, failover and unknown-reachability behavior. Lookup does not allocate
media or provide a reusable authorization lease. Canonical Gate 6 dispatch and
Session/transport admission remain separate implementation tasks.


### Gate 6 attachment links and recipient evidence

ArtifactBegin requires explicit `to`. ArtifactInspect, ArtifactUploadLink,
ArtifactDownloadLink, ArtifactLinkAppend and ArtifactLinkGet share the current
message/file audience engine. Link paths require current credentials and protected
task/grant context; renewal preserves byte mutation IDs. MessageReceipts returns
per-recipient/per-channel evidence with current preference filtering.
MessageReceipt is a self-report of delivered/read, with reading opt-in; it never
implies a human response. ReceiptPreferencesGet/Set affect only the current member.
See [the HTTP contract](openapi-attachments.json) and
[behavior and recovery](../docs/developer/attachments-and-receipts.md).

Schema 33 conversation semantics, canonical create/list/close and shared SDK helpers are specified in [conversation control](../docs/developer/conversation-control.md) and [OpenAPI](openapi-conversations.json). Old create bodies and incompatible candidate state are refused.
