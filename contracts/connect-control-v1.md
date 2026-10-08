# Connect control profile v1

Status: experimental, local Gate 1. Profile `connect-control/1`, UCTP envelope
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
