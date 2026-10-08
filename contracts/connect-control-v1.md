# Connect control profile v1

Status: experimental, local Gates 1–2. Profile `connect-control/1`, UCTP envelope
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

Payload is `{ "profile": "connect-control/1", "op": "...", ... }`.

| Operation | Inputs | Outcome |
|---|---|---|
| resolve | address: `connect:alias@local` | permitted stable identity and capabilities |
| create | request_id, participants: identity IDs | durable conversation; caller included |
| send | request_id, conversation_id, body: UTF-8 text | accepted receipt |
| history | conversation_id, after: opaque cursor or null, limit | ordered messages and next cursor |
| inbox | cursor: opaque cursor or null, limit, wait_ms | ordered journal events and last scanned cursor |
| request_status | request_id | accepted/rejected/not_found; saved outcome when permitted |
| close | request_id, conversation_id | closed conversation; retained history |
| health | none | installation ID, schema, profile |
| provider_status | conversation_id | deliveries: job/message/connector IDs, state, nullable provider_id |
| call_start | request_id, conversation_id, route_id, destination: E.164 | durable call receipt |
| call_status | conversation_id, call_id | current durable call state |
| call_end | request_id, conversation_id, call_id | durable end intent |

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
64 simultaneous peers per listener; 256 outbound/coordinator queue (upstream).
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

All twelve operations share the same authorization, UCTP profile, request/reply
correlation, and durable conversation journal. Provider credentials and private
call-control URLs stay server-side. A voice route binds an assistant and provider
number to the permitted conversation/identity; a scoped voice grant authorizes
outbound destinations. Caller ID on an inbound telephone call is an observation,
not authentication or permission to act as a Connect identity.

`send` acknowledges durable local acceptance. `provider_status` separately reports
provider delivery; failed/uncertain work is not silently resubmitted. Vapi chat
context is persisted. Telnyx SMS uses programmable messaging and signed callbacks;
live carrier delivery remains deferred pending approval.

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

The qualified telephone path is handset ↔ Telnyx ↔ Vapi. Connect provisions and
controls that route and records its lifecycle; the audio does not pass through
Connect, QUIC, or a Connect WebSocket. A Telnyx SIP → Connect → Vapi audio bridge is
separate, unimplemented Gate 3 work. Local QUIC datagram probes do not prove it.

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
