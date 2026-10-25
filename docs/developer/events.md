# Snapshots, journal events and transient observations

These APIs read the same durable conversation and source audiences as messages,
files, sessions and operations. A transport wakeup is a hint. It never accepts a
mutation or proves delivery. Reads have fresh transport exchange IDs and do not
create logical mutation receipts.

`conversation_snapshot` accepts a conversation ID and `SnapshotQuery` (`page`,
`limit`). Its first read captures the authorized conversation topology, membership,
session appearances, connection references and owned operation status in one SQLite
transaction, together with the installation journal high watermark. Message bodies
and file bytes remain in the separately paged history and artifact APIs. No physical
stream facts are invented before the media coordinator supplies them.

Ordinary native offer and stream-control completions now record bounded binding
metadata in the same transaction as their operation result. Stream identity is
scoped by Connection ID plus Stream ID. `negotiated` describes captured parameters;
it does not establish media flow. `active` and `paused` describe captured control
state. A confirmed `retired` Connection projects its recorded streams as `ended`.
Core termination and its command receipt can precede the separate durable
retirement observation; callers wait for the current Connection snapshot.
The Stream resource observation version is independent of the native stream
revision used by control commands. Unchanged streams retain that version.

A snapshot omits a projection if its captured generation/revision cannot match
the recorded Connection state. Concurrent descriptor unavailability cannot turn
an older capture into a current one. Each Connection retains at most 32 current
bindings and 64 current/terminal summaries in total. Eviction affects only
terminal snapshot summaries; original operation and journal identities remain.
Every Stream page rechecks the same current audience, recipient ceiling,
membership and private-leg restrictions as its Connection. This metadata never
grants physical stream ownership or media permission.

Drain `next` until `has_more` is false before treating the snapshot as complete.
Every page retains the original snapshot ID, revision and watermark, even if a role
or operation changes while it is paged. Fetch subsequent events after that watermark.
Pages recheck the credential, its recipient ceiling, membership epoch and each
source's current audience. A removed recipient or superseded connection cannot
regain access through a cached page. Roles never grant permissions.

The snapshot cache holds at most 32 snapshots, two per stable identity, at most
8 MiB of encoded topology per snapshot and 64 MiB of encoded topology overall.
These are payload accounting limits, not a process RSS claim. Every snapshot ends
within 30 seconds and no later than credential expiry. Expiry, eviction or server
restart returns `resync-required`; request a fresh snapshot. No read transaction,
WAL snapshot or indefinitely leased history is pinned between requests.

`conversation_events` accepts `EventQuery`: `after`, `limit`, `lease`,
`lease_seconds` and `wait_ms`. Without `after` it returns a snapshot boundary.
Otherwise it reads newer authorized journal facts in cursor order. Apply each event
idempotently by `event_id` and persist the cursor only after actually processing it.
Re-reading a cursor can return the same event ID. There is no private-event omitted
count. Audience and capability checks run before counting a delivered page item.

The default page size is 100, subject to a lower configured installation ceiling;
the protocol ceiling is 500 items and 512 KiB encoded. A live catch-up response is
also bounded to 256 event references. Leases last at most 30 seconds and no later
than credential expiry. Long polls register their wake hint before reading and
read again after a hint or timeout. They hold no database transaction or payload
queue while waiting. A physical native peer closing cancels its pending read.

An expired live lease, restart-bound lease or position lost to retention returns
`resync_required: true` and a fresh authorized snapshot reference. Follow that
boundary explicitly; an empty page is never substituted for known retention loss.
A retained cursor remains usable across credential rotation for the same stable
identity and membership epoch, subject to all new permissions and recipient ceilings.
Live leases and cached pages bind their admitting credential and scope. A restored
backup cannot make a cursor newer than its own journal appear successfully caught up.

New event tokens use authenticated encryption and fixed-size plaintext padding.
A client cannot recover installation sequence numbers or infer filtered counts from
their digits or token lengths. Purpose, identity, conversation and membership epoch
are bound inside each token. Tokens cannot be substituted for another inbox,
conversation, snapshot page or observation stream. Existing inbox/history token
formats are independent APIs; these are not compatibility aliases.

New model writers retain their actual kind, original revision and observed time,
source, frozen recipient epoch, typed data and authorized operation correlation.
Operation updates preserve the state recorded by that event; looking up a current
operation cannot relabel a prior accepted event as completed. Operation status is
private to its currently authorized owner, including when the room is shared.
Only the journal cursor orders facts. Wall times are diagnostic observations.
The original eight journal insertion sites are mapped in
`contracts/gate6-event-writers.json`. Schema 27 stamps original resource revisions
and source provenance inside the insert transaction and refuses missing metadata.
Initial state revision zero is a defined stored version. Session snapshots expose
both `revision` (stored state changes) and `topology_revision` (transition
preconditions). Upload chunks can advance file state revision between journalled
file lifecycle facts; a resource revision gap is not evidence of lost messages.
Core request correlation is bound to the authenticated writer and visible only to
that identity; task invocation IDs are not mislabeled as core operations.

`prune_conversation_journal` is protected local maintenance, absent from public
operation bodies. It keeps at least 30 days and deletes at most 1,000 rows per call.
Any deletion advances a durable conservative retention floor, which survives backup.
Ordinary membership filtering does not advance that floor. Message history, effect
facts, operation receipts and digest tombstones are separate from journal retention.

## Original Session start

On the first verified public native flow, schema93 commits the Active phase,
original `SessionStarted` snapshot and per-Participant journal facts together.
Acceptance or an offer/answer alone does not start the Session. Storage refusal
rolls back the whole transition; retrying the captured metadata sends no extra
media. The original start remains unchanged after codec/topology changes or end.

The unsolicited signed `session.started` hint has CID/SID and no Connection scope,
operation ID or reply correlation. Its payload includes original `started_at`,
`participants_present`, observed `active_connections` and actual negotiated
capabilities. A current authorized admitted Participant can receive it without a
media Connection or subscription. Every recipient writer rechecks current
credentials, Device/key bindings, source access, epoch and recipient ceiling.

Recover missed hints through `conversation.subscribe` catch-up. The original typed
hint precedes its cursor-bearing `session_started` event and has the subscription
reply correlation. Current authorization still applies after the Session ends.
Participant joined/left, last-Connection loss and reconnect grace remain separate
unfinished lifecycle work; packet silence does not establish loss.

## Canonical physical delivery

The [schema-34 canonical subscription continuation](canonical-subscriptions.md)
uses these same pages through per-physical handlers and the authenticated guarded
writer. Its receipt-aware batch contains at most 63 journal facts,
each with at most one typed Session start/end, closure, receipt or record-readiness hint, the event and the final ACK, with eight
live Conversations per peer. Hints precede cursor-bearing facts; see
[canonical receipts](canonical-receipts.md). Idle waits retain no authority reader. Exact snapshot and
subscription query proofs fence the current selected source owners through local
submission; a source-set race retries the same cursor under fresh authority within
the original bounded wait. Production canonical listener installation remains open.

## Original Conversation closure

Schema 87 stores the original `reason_code`, `reason` and `closed_at` when a
`conversation_closed` journal row commits. Every authorized recipient sees the
same typed `conversation_closed` data. Closing a room or replaying its operation
cannot rewrite that fact, and a later credential restriction can hide it.

Canonical catch-up and live subscriptions emit `conversation.closed` before its
cursor-bearing `conversation.event`. The hint carries the original closure time,
Conversation scope and subscription reply ID. Process the journal fact and cursor
for recovery; the hint alone grants no access or delivery guarantee. The delivery
patch also admits existing `recording.vcon-ready` hints under the original bounded
subscription and current authority checks.

## Original Session termination

Schema88 reserves the original terminal Session snapshot, reason, time and known
actor Participant/epoch when its journal transaction writes. This small fact
survives release of the larger pending record snapshot and later record retention.
It does not claim that media ever became ready or that a provider confirmed an end.
Runtime paths without a supplied numeric reason record0 (unspecified); missing
reason text is `unspecified`, and an unknown/non-Participant actor remains null.

`session_ended` journal data contains the original shared Session view. Native live
subscriptions and finite catch-up emit `session.ended` immediately before that
cursor-bearing fact, with the original reason/time, Session scope and subscription
reply ID. The vCon handle arrives separately when asynchronous finalization becomes
ready. Current `session_get`, original/current Session source access, membership
epochs and recipient ceilings still govern each read and actual signed delivery.
An exact end replay does not publish another terminal fact.

The TypeScript and Python Session snapshot shape follows the current shared
Session view: media/intent/capabilities, main-Session marker and both revisions.
The old initiator/counterpart snapshot shape is not accepted as that current view.

## Transient observations

`session_observations` accepts a conversation/session and `ObservationQuery`
(`after`, `limit`, `wait_ms`). It returns generic transcript, speech, action-request
and typed agent observations with participant epoch, connection generation, source correlation and
server-observed time. Transcript completeness is interim/final/incomplete. Speech
is started/stopped. An action request names a declared action, its provider tool-call
ID and untrusted arguments; it does not schedule a worker, grant authority or execute
an operation.

Trusted adapter composition obtains a `SessionObservationSource` after verifying
its actual route. The handle is not serializable or constructible by request bodies.
Registration binds an existing current connection, generation, endpoint, participant,
source correlation and explicit frozen route audience. Every selected recipient must
have current original session access. The source owner is included. This permits a
private audience that excludes another member of the same public session.

Publishing rechecks current credentials, membership, endpoint verification,
connection generation, runtime and session state before duplicate handling. A stale
or revoked source cannot publish late observations. Queued items and advertised
route capabilities recheck current reader and source authority before delivery.
Unsupported speech, transcript or action capability is explicit; missing speech
notifications never prove silence. A publisher's local event key matches only the
same source and content within the retained transient horizon; changed content
conflicts. Provider event/tool IDs inside agent metadata are data: repeated frames
receive separate local sequences and observations.

The transient buffer retains at most 256 references and 2 MiB of encoded observations
for at most 30 seconds, with at most 128 registered source routes. Each draft is at
most 16 KiB encoded for generic data; typed agent drafts allow 70 KiB with a
65,536-byte inner observation. Oversized agent content is explicitly unavailable,
never a valid-looking truncated prefix. A slow reader receives an explicit resync/incomplete boundary
when its authorized source data has been evicted. Hidden sources do not produce a
private gap count or resync flag for an unauthorized reader. Restart discards live
observations and invalidates their cursors. Only separately selected policy-approved
observations enter durable context; this buffer never creates a Message implicitly.

Agent metadata preserves optional provider references, provider speaker role,
partial/final/unavailable completeness, requested actions versus reported tool
results, capture time and known local sequence loss. `capture_complete` is always
false. A provider `user` role cannot identify a human in mixed audio. Neither model
arguments naming a task/effect nor a reported tool result bind or complete a Core
operation. The current Session/journal and public operation checks remain
independent of these best-effort observations. See [agent observations](agent-observations.md).

These source handles and maintenance methods are backend Rust capabilities, not
HTTP/MCP model tools. Current control operations and the Rust client share one
service/store path. Canonical envelope dispatch is G6-037; physical graph/stream
and provider normalization integrations are G6-038/G6-043/G6-044. Storage fixtures
for these boundaries are not physical/provider behavior qualification. The Rust, TypeScript and Python helpers use the same current service, with a
closed [HTTP reference](../../contracts/openapi-events.json). Complete current
qualification is recorded separately; no checklist status follows from this guide. Fresh state is schema 31; older candidates are preserved
and refused unchanged.

## Participant arrival and departure

Schema94 emits `session.participant.joined` after verified public native readiness,
with immutable identity/kind/Session role/display name, original joined time and
via-Connection. Accepted presence alone does not emit it. Additional Connections
for the same appearance do not duplicate it. A terminal last Connection or an
explicit authorized departure produces `session.participant.left`. A new ready
appearance after departure joins again on the same live SID; old facts remain.

The source-ordered bounded metadata worker independently authorizes each actual
recipient write. Recipients need no media Connection or subscription. Recover
missed hints from the journal; finite catch-up puts each typed hint before its
original event. The event retains a stable fact ID and original subject/recipient
epochs. Every hint envelope has a fresh ID. Mutation-operation correlation is
owner-only journal metadata; unsolicited hints have no operation/reply scope.
Full reconnect grace and private promotion/restart aggregation remain separate
unfinished lifecycle work until verified.
