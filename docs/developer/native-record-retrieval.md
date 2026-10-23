# Native record retrieval

G6-046 installs signed `recording.vcon-fetch` for the canonical native peer. The
request needs Conversation/Session scope and a bounded UUID and/or local URL
selector. When both are present they must select the same record. Selectors
are not authority, and arbitrary URLs are never fetched over the network.

After terminal outcome storage commits, the listener independently multicasts
`recording.vcon-ready` to currently authorized original Session participants on
their registered physical bindings. A Conversation subscription is not required.
The hint contains UUID/revision metadata only; it carries no bytes, full-record
digest or read grant. It follows Session terminal metadata in the durable source
queue. Each delivery rechecks the recipient's captured membership epoch and
current `record_get`, source policy and stored revision. Deleted, changed or
unavailable records cannot produce a stale live hint. Failed readiness storage
rolls back outcome bytes, audience, journal and snapshot release together;
retry uses the original UUID. Catch-up independently reconstructs its correlated
hint from the authorized journal.

Participant admission checks the current credential, Device/signing key,
`record_get` permission, Conversation membership and actual Session source access. Ordinary
new collaboration credentials explicitly include that permission; membership
or a role label alone cannot add it. Store derives source facts under the same
held command authority. Core's backend read then reserves the exact current
record. Revision/body/manifest mismatches between policy derivation and protected
lookup refuse the view. No authorization reader is dropped and reacquired behind
a queued writer.

The supported metadata format starts at schema 77. This fresh schema boundary
does not silently reinterpret older private canonical records as the public
read format. Shared Session metadata requires its frozen original membership
epoch or an explicit current Session history share. Each appearance additionally
checks the credential's recipient ceiling for that Participant's Identity.
Connection lineage requires its original owner epoch. Sharing Session metadata
does not grant another Connection's private content.

[Explicit tenant reader roles](tenant-record-readers.md) use a separate typed
viewer and current tenant policy. They do not manufacture membership or inherit
erasure authority. Both paths retain credential scope and recipient ceilings.
Tenant readers fetch known selectors; this role does not grant Conversation
snapshot discovery.

Capture diagnostics are a separate source from the useful Session outcome.
They require permission for every other source and zero whole-record omissions;
a caller cannot learn hidden activity counts through the shared outcome. Core
also enforces each source's retention/state and dependency projection. Unknown
record policies and source IDs do not become blanket read grants.

The source lease retains the original Store authority. Its validity predicate is
nonblocking and performs no storage IO: authorization is protected by the owned
Identity/Conversation fences, and Core retains exact record custody. The native
dispatcher retains the composed lease through the actual response writer, bounded
by original credential/key lifetime, the dispatch deadline and the earliest
retention cutoff of the projected sources. Core exposes the original projection's
IO deadline, including visible metadata, content actually exported and authorized
URL lifetimes. The protocol installs that bound in the retained lease so a writer
blocked on QUIC flow control wakes at source expiry. Hidden sources and content
already represented as unavailable do not shorten an authorized metadata view.
Credential or membership changes cannot authorize a stale future request.

A cutoff can retire a partially written control frame while the caller's
credential remains valid. The SDK rejects that incomplete frame; copied bytes or
an old digest cannot authorize a retry. Guarded reference downloads retain the
same projected-source deadline in addition to their reference and authority TTLs.

`recording.vcon-fetched` carries the caller's projected bytes and their exact
SHA-512 content hash. A conditional hash names that view, not the full backend
record. A narrowed credential can receive a different digest for the same UUID;
a previous full-view digest cannot recover unauthorized sources. The fetch-only
handler is explicitly composed with Connect's handler, without stock Core
command fallthrough.

`CanonicalClient::fetch_record` accepts `CanonicalRecordSelector`. It verifies the
correlated signed batch, terminal acknowledgement, inline bound, canonical vCon
schema, exact UTF-8 byte digest, optional condition and selected UUID. Errors use
the SDK's sanitized canonical error mapping. Returned body bytes confer no
future access. The current inline limit is 128 KiB; larger views use the optional
authenticated HTTPS endpoint and the original TLS pin/current bearer. See
[HTTP record retrieval](http-record-retrieval.md#authenticated-https-references)
for reference, download and expiry bounds. Without that endpoint, larger views
return explicit delivery-unavailable.

The local native backpressure cases use signed peers with 4 KiB stream and
connection receive windows. They stop after the length prefix of a response over
80 KiB, so actual frame output remains blocked. Credential revocation and exact
record deletion wait for completed output; receive-stream cancellation, peer
closure, credential expiry and frozen source expiry release the retained guards.
The positive case verifies the complete EdDSA/ES256 signed frame against the HTTP
SDK's exact view. Its large body is an explicit trusted metadata fixture attached
to a real terminal Session; it does not qualify captured media.

Public discovery uses `conversation.get` snapshot items of kind `record`.
`CanonicalClient::conversation_records` and
`CanonicalHttpClient::conversation_records` return one bounded page, its next-page
token and journal cursor. Continue until `next_page` is absent. Discovery requires
both `conversation.get` and explicit `record_get`; it does not require or grant
`session_get`. Each hint independently requires current Session source access and
its frozen original membership epoch or an explicit current Session history share.
A narrowed credential does not inherit permission to see other Participant or
Connection topology through discovery.

A pending outcome has only its Session ID and `pending` state. A ready item adds
UUID and positive record revision. A deleted/missing materialized record is
`unavailable`, with no UUID, storage URL or digest. Cached snapshot pages recheck
current authorization and state, suppressing stale ready items; obtain a fresh
snapshot when a cached item changed. No record body, canonical/manifest digest,
private source/count or recorded-audio claim enters these hints.

The producer atomically commits bytes, source epochs, intent completion and
`record_ready` journal entries. Journal capacity refusal rolls all of that back;
the same stable intent remains pending for retry. Current journal projection
requires record permission and the same source/membership checks. Changed or
deleted ready revisions are suppressed. Finite/live canonical subscriptions emit
`recording.vcon-ready` before its cursor event, carrying only UUID/revision,
`encrypted:false` and an empty signer list for this unsigned metadata policy.
These are hints: a later fetch authenticates again and computes its own projected
digest. Fresh snapshots and journal cursors recover discovery after server restart.

HTTP record body retrieval uses the same public SDK selector and projection checks
through an explicit closing HTTP/1.1 socket-custody profile. See
[http-record-retrieval.md](http-record-retrieval.md). Public redaction/deletion and
named retention workers have separate recorded qualification. Captured-media
links and the remaining auxiliary behaviors remain G6-046 work.
The discovery runtime test obtains selectors without a privileged local record
index, reconnects after a full server restart and independently discovers from the
invited participant over bearer HTTP with record-only read rights.

The selector can use a live HTTPS URL issued by this server plus its content hash.
This is a fresh control read: current source authority and its projected hash are
recomputed, including recipient attenuation. Conversation/Session scope and an
optional UUID must match the same resource. Expired, foreign or unknown URLs do
not cause network fetches or fall back to a supplied UUID. A changed condition
returns a correlated conflict; obtain a fresh authorized view explicitly.
