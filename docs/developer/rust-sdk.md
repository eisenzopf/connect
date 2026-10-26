# Rust public SDK implementation

G6-060 is in progress. `connect_client::sdk` provides owned signed native
control through `Client`, `Conversation`, `Participant`, `Session` and typed
`Operation<T>` handles. It composes the configured canonical transport and uses
one bounded request owner. The remaining Connection/Stream, transition,
event/capability, raw/admin and media surfaces are still being implemented.

Creating a handle from an ID grants no authority. Each refresh, mutation and
original-operation status query checks current server policy. Mutations require
an application-saved logical ID and return an observable Operation. Explicit
bounded waiting returns its typed completion; receipt completion does not prove
recipient consumption or media readiness. The compiled starting example is in
`crates/connect-client/src/sdk.rs`.

| Public control | Required precondition |
| --- | --- |
| `Client::create_conversation` | Configured tenant, explicit identity list and policy |
| `Conversation::send` | Original message ID and nonempty Participant audience |
| `Conversation::add_participant` | Identity, actual identity kind, role and Conversation revision |
| `Participant::update` / `remove` | Conversation revision |
| `Conversation::invite` | Saved new Session ID, targets, medium, intent and capabilities; empty targets permit a solo Session |
| `Session::invite_participants` | Nonempty targets, current media, original intent/capabilities and topology revision |
| `Session::accept`, `reject`, `cancel_invitation`, `update`, `end` | Explicit topology revision |
| `Session::offer_connection` | Saved new Connection ID and nonempty typed stream offers |
| `Connection::end` | Saved operation ID, original reason and optional negotiated `StreamRevision` |
| `Connection::update` runtime variants | Negotiated `StreamRevision`, explicit targets/hold direction or quality interval |

Session updates include role changes, participant leave, additive media upgrades
and codec preference changes. Codec reconciliation references remain separate
from the local policy receipt. An accepted Session can still have a newly invited
Participant; current actor appearance and authorization determine acceptance.

`SessionSnapshot.revision` observes the provenance `state_revision` counter.
The migration trigger updates that counter for selected Session-row changes,
including topology changes. A media upgrade can update media and topology in
separate writes, so these counters can differ. Session controls use the distinct
`TopologyRevision` type. The Rustdoc negative example rejects a state revision
passed to `Session::end`, and the production test proves refusal of its wrongly
cast numeric value when the counters differ.

Membership receipt reads project currently authorized metadata. A valid same-ID
replay can observe a later role and Conversation revision, while a new membership
epoch refuses the old receipt. `receipt_snapshot` is an observation from the
receipt read, not a permission grant. Fresh reads use a shared bounded snapshot
reader with fixed page/byte ceilings and explicit resync refusal.

Saved references contain original operation ID, command and scope, without
credentials. Restoration validates schema, command kind and scope locally; it
does not submit another mutation. Status is queried on the original ID. Local
cancellation leaves an already started exchange owned until it settles, while
cancelled queued work stays unwritten. Explicit close retains its join handle
across timeout/cancellation so cleanup can be awaited again.

The owned driver multiplexes command exchanges on one signed peer. Status and
durable decisions can complete while an earlier command still awaits its reply;
each reply binds its envelope ID and exact resource scope. One shared limit of
16 covers queued and active exchanges. Waiting for that capacity is bounded by
the caller's deadline and cancellation. Each active reply batch retains the
canonical 128-frame/2 MiB limits and its original transport/authority deadline.
An incomplete exchange deadline or invalid correlated frame retires the peer
and returns the original logical IDs for all affected mutations. Local caller
cancellation keeps a submitted exchange owned until it settles or the peer
retires. This control driver is a prerequisite for native negotiation; native
offer/answer and media I/O are still being implemented.

`docs/evidence/gate-6/rust-sdk-multiplex.json` records signed interleaving,
capacity, failure and cleanup checks, plus production listener concurrency with
24 distinct messages and interleaved status reads under EdDSA and ES256.

Current local verification is recorded in
`docs/evidence/gate-6/rust-sdk-participants-sessions.json`: three production test
functions, each covering EdDSA and ES256, plus the client library suite and
Rustdoc checks. These are functional control checks. Complete SDK parity, decoded
media, a fresh external crate, restart/incompatible-server qualification, remote
and Linux profiles, packaging and Gate 6 acceptance remain open.

`Operation::cancel` requests a separate durable decision with its own saved ID;
`reconcile` additionally supplies an outcome and evidence reference. A decision's
completion describes that decision, while its receipt separately preserves the
observed target state and disposition. `already_terminal` cannot undo a completed
message or Session action. The target field is an Operation ID, without an
invented command/scope. Saved decision references omit target Connection assertions
and preserve known Conversation/Session assertions for status recovery.

The host validates reconciliation evidence; a caller-provided ID cannot manufacture
an outcome. Nonempty decision extensions currently refuse before I/O. Server
failures keep uncertain acceptance under the original decision ID. Additional
control verification is in `docs/evidence/gate-6/rust-sdk-decisions.json`; positive
backend reconciliation and cancellation of actual pending native work remain open.

`Session::offer_connection` submits a new native Connection with typed stream
IDs, media, directions and codec preferences. It returns an observable operation
after the initial signed answer. The owned driver validates the later server
parameter offer and signs its exact same-root confirmation. Native negotiation
has no final ACK; `Operation::wait` reads the separate durable result. Status and
durable decisions remain available while a parameter offer is pending.

The builder supports PCMU, PCMA, mono Opus, VP8 video/screen and reliable ordered
text/JSON/binary parameters. It preserves the original codec preference list,
including unsupported preferences before a supported fallback. An offer with no
supported codec for a stream refuses before I/O. Other transition preparation
profiles require their own authorized workflows. The driver bounds owned native
Connection metadata to 64 entries and refuses cross-Connection hop-handle
collisions before signing confirmation.

`Connection::refresh` reads current authorized journal metadata. Its resource
revision has its own `ConnectionRevision` type. The journal does not provide a
negotiated stream revision, so that optional field remains absent. A completed
offer exposes its original `receipt_stream_revision` separately.
`native_negotiation` observes this client's locally signed parameters; it does
not prove media flow or remote consumption. A restored operation can recover a
Connection receipt without gaining another physical peer's hop-local handles.

`docs/evidence/gate-6/rust-sdk-native-offers.json` records the mixed negotiation,
pending status, parameter refusal, replay, shared-peer and fresh-client checks.
Connection end adds the fifteenth mapped mutation kind. Stream controls,
Connection update, device move and consultation/handoff remain in progress.

`Connection::stream_by_id` returns a scoped Stream handle. For a native RTP
binding owned by this exact client, `Stream::send_rtp` accepts a validated complete
`RtpPacket`. Marker, CSRC, extension, padding, sequence, timestamp and payload
bytes stay intact; the driver supplies the peer-local stream handle. The local
direction and negotiated payload type are checked before submission. Success
means admission to the local transport queue, without delivery or consumption
confirmation. A cancelled or expired queued send may remain uncertain, and the
SDK does not resend it.

`send_clock` and `receive_packet` keep sender clock reports separate from RTP.
Clock I/O requires negotiated clock support. Callers supply their actual source
counters and timestamps; these reports do not establish synchronized playback
or media readiness. Packet read cancellation leaves queued packets available.
Cloned Stream handles consume one shared queue for that stream.

One peer owns a 64-packet outgoing queue and receive ceilings of 32 packets per
stream, 256 packets and 4 MiB overall. A slow stream drains its older packets,
then receives `MediaPacket::Gap` with the local drop count before new packets
resume. Other streams and signed control requests continue. Network sequence
gaps remain distinct from local queue loss. `Client::media_stats` reports queue,
overflow, malformed packet and unknown-handle counters. Closing the client
clears queued media and wakes packet readers.

`Stream::refresh` reads recorded native binding metadata from the authorized
server snapshot. Ordinary offer and stream-control captures commit atomically
with their operation. `negotiated` records bound parameters; `active` and `paused`
record captured controls. A confirmed retired Connection exposes known streams
as `ended`, including after normal restart. Core termination and its receipt
can precede this separate retirement observation. A missing or mismatched capture is
unavailable; the SDK never substitutes its local negotiation cache. Stream IDs
are scoped by Connection. Resource observation versions are independent of
native-control revisions and cannot be used as `expected_revision`.

Schema112 stores at most32current bindings and64current/ended summaries per
Connection. Every snapshot page checks current Connection/Session visibility,
membership epoch and recipient ceilings. This metadata grants no media I/O.
Pre-alpha existing state requires a fresh schema112 database. Transition-specific
captures, reliable data I/O and the remaining typed controls are still G6-060
work. RTP/clock evidence is in `docs/evidence/gate-6/rust-sdk-media.json`;
authoritative metadata evidence is in
`docs/evidence/gate-6/rust-sdk-stream-metadata.json`.

`Connection::end` returns `Operation<Connection>`. It accepts an optional
negotiated `StreamRevision`; a `ConnectionRevision` or `StreamResourceRevision`
is a distinct type. A completed, scoped `ended: true` result establishes actual
local native termination. The separate offer worker can journal `retired` later,
so callers wait for that resource observation when they need ended Stream rows.

The owned driver clears only that Connection's receive queues and wakes its
pending packet readers when it verifies completion. This happens even if the
caller cancels its local wait. Later queued sends recheck the binding and refuse
it. Unknown outcomes do not manufacture termination; use the original operation
for status and recovery. A restored completed operation can return its scoped
Connection handle without acquiring physical ownership. Same-intent end replay
remains subject to current server authorization.

Retired negotiation metadata keeps its hop handles reserved within the existing
lifetime bound of 64 Connections on the physical peer. A later offer cannot reuse
those handles to interpret an old queued datagram as a new stream. Ending one
Connection leaves other Connections, media and signed control usable. The exact
verification scope is recorded in
`docs/evidence/gate-6/rust-sdk-native-end.json`.

`Connection::update` implements typed hold/resume, source mute/unmute, quality
settings and direct codec renegotiation on an owned native Connection.
`HoldDirection` describes this client: `Sendonly`, `Recvonly` or `Inactive`.
Mute/unmute require nonempty explicit Stream IDs. Hold/resume can target explicit
IDs or all current streams. The server still enforces its supported media profile
and current authority. Quality intervals are integer milliseconds in 100–60,000.

The returned `ConnectionUpdateReceipt` contains the captured native model revision
and typed gate observations or quality interval. Gate changes advance that native
revision; quality settings retain it. Snapshot resource revisions remain distinct.
`GateState::Active` describes captured control state and does not establish flow;
`source_muted` and the client hold direction remain separate observations.

Newer receipts update only matching locally owned bindings. A saved receipt can
advance a previously unknown original outcome, but it cannot install a foreign
hop handle or overwrite a newer known control revision. Original-intent replay
and fresh-client recovery remain current-authorized server reads. Cancellation
of the caller leaves a submitted control exchange owned by the driver.

Local send/read direction checks reflect known controls. Pausing receive clears
queued packets for those streams and wakes their readers; resume exposes the
local `Gap` before accepting more packets. `MediaStats::control_drops` separates
known-binding direction/control drops from queue overflow and malformed input.
Other Stream queues continue. These local facts never replace server admission
or prove remote playback. Evidence is recorded in
`docs/evidence/gate-6/rust-sdk-runtime-controls.json`.

`ConnectionUpdate::CodecRenegotiate` preserves ordered original preferences and
optional exact stream targets. Codec reoffers bind the original request, logical
operation, Conversation, Session, Connection, Participant, revision, supported
codec parameters, unchanged stream set and both endpoint directions. The driver
confirms only that validated preparation. Reliable data streams retain their
logical identity and privately retained native port handle; the public data
observation exposes no RTP handle. Selected RTP bindings can change.

Confirmation keeps a bounded pending parameter proof. The old local binding stays
in place until the matching completed receipt is known. Unknown or cancelled
waiting preserves the original operation reference; status recovery can pair its
captured receipt with that proof. Fresh or historical receipts alone cannot
acquire hop ownership. A recorded failure releases pending preparation while
retaining its reserved hop identities. At most one unsettled replacement per
Connection is retained; another codec mutation receives `Conflict` with
`NotSubmitted` certainty until the original outcome settles.

Binding replacement preserves unrelated queues and known logical mute/hold
state. It reports discarded old packets as a `Gap` and counts them in
`binding_change_drops`. Gate observations have separate per-stream revision
floors, so out-of-order observations for different streams are retained and an
older codec receipt cannot undo later controls. Native model revision, resource
revision, stream generation and Connection generation remain distinct counters.
The HTTP reference describes the captured `NativeCodecBinding` and
`NativeCodecResult` shapes; these observations grant no media admission.

This direct update path does not complete the SDK's remaining transition,
subscription, server-initiated reconciliation, event, data, device, configuration,
raw/admin or external-package journeys. The exact codec qualification scope is in
`docs/evidence/gate-6/rust-sdk-codec-updates.json`.

`Session::consult` returns `Operation<ConsultationResult>` with an explicit source
Connection, retained Connection, private Participant set, hold treatment and
`TopologyRevision`. Admission, private preparation, actual decoded audio and
completion are separate boundaries. A completed topology receipt supplies scoped
Connection handles for observation; it does not acquire their physical media.

A consultation's saved operation reference retains its original source and
retained Connection and observed host-issued `OperationRecordId`.
`Operation<ConsultationResult>::return_to_retained` uses that saved record and
submits a new logical return operation under current host authority. It never
substitutes the logical operation ID for the
Store record ID or resubmits the original consultation. If the record has not yet
been observed, recovery reads it under current authority; that read and return
submission share the caller's deadline. A failed read reports that the new return
mutation was not submitted. Later observed record IDs enrich saved references,
and a changed host record ID is refused. A saved record grants no permission: the
server checks current return authority even after the old execution credential is
revoked. Caller cancellation still leaves a started exchange owned by the driver.

Transition admission and status carry the host's original `command_type`,
`expected_revision` and `record_id`. The SDK checks these before decoding a
transition receipt, including after restoring a saved reference. Invalid or
missing consultation continuation context is refused locally. Current host
permission and intent checks remain authoritative.

The returned Session exposes `consultation_return_receipt()` with the captured
retained/retired topology, original consultation record and `remote_continuing`.
Remote EOF can yield a settled restoration with `remote_continuing: false` and a
retired remote Connection; completion alone does not assert a continuing call.
Read current Session topology before choosing a return revision:

```rust
let original = session.consult(input, begin_options).await?;
let saved = original.reference_json();
let restored = client.restore(saved, client.operation_kinds().session_consult)?;
let consultation = restored.wait(read_options.clone()).await?.value;
let current = consultation.session.refresh(read_options.clone()).await?;
let returning = restored
    .return_to_retained(current.topology_revision, return_options)
    .await?;
let session = returning.wait(read_options).await?.value;
let observation = session.consultation_return_receipt();
```

The consultation/return qualification evidence is recorded in
`docs/evidence/gate-6/rust-sdk-consultations.json`. The subsequent handoff and
Connection replacement APIs are described below. Subscriptions and the remaining
complete SDK journeys remain G6-060 work.

`Session::handoff` takes explicit source/target Participants, source and retained
Connections, optional candidate, outgoing disposition and role scope. A
Conversation role change requires its separate `ConversationRevision`. A typed
consultation reference contributes its original host record ID; an unobserved ID
requires an authorized recovery read within the same deadline. The caller's
`policy_ref` and other extensions keep their original values, including explicit
nulls and array order. Typed absent optionals are omitted from the wire.

`HandoffResult` records the retained/admitted/retired topology and, when captured,
the native role change. Participant authority revision, Session topology and
Conversation revision remain separate observations. `cleanup: pending` does not
claim that every outgoing resource has finished retiring. An old completed result
cannot install foreign media or repeat the role change.

`Connection::replace_device` chooses either an Endpoint or an existing candidate
Connection with `MoveTarget`. Its `MoveResult` reports the captured target handle,
generation, Participant/membership epoch, Device/Endpoint and topology. The
server records the retained/admitted/retired IDs with the committed move;
preparation, readiness or the SDK request cannot substitute for that result.
Source cleanup and target packet admission retain their current independent
physical authority. A recovered observation does not acquire the new Device's
transport or local hop handles.

Both methods return typed original-ID operations even when work is still pending
or records a failure. Unknown and interrupted states remain observations, and a
cancelled caller leaves a submitted exchange owned. Saved transition references
retain the intent context needed to validate historical results. Device movement
uses Session `TopologyRevision`, independently of stream, resource and generation
counters. The exact current qualification scope is recorded in
`docs/evidence/gate-6/rust-sdk-movements.json`.

The movement qualification uses SDK controllers with independent public Canonical
native media owners. Restoring a controller operation does not grant the target
Device's transport custody. Complete transition reconciliation, both-ends SDK
media and external-package/remote profiles remain separate G6-060 work.

`Connection::subscribe` accepts nonempty `StreamSelector` values: an explicit
logical Stream ID, or a Participant with optional nonempty media kinds.
`Connection::unsubscribe` accepts nonempty explicit Stream IDs. Both take a native
`StreamRevision` and a stable `MutationOptions.operation_id`; a snapshot resource
revision or a topology revision cannot replace that assertion. Selector order,
extensions and explicit null values remain part of the original request.

These calls return `Operation<StreamSelectionReceipt>`, including when the
original result is pending, unknown or failed. Saved references use the distinct
`stream_subscribe` and `stream_unsubscribe` operation kinds. The receipt records
the original Connection generation, resulting native revision and whether actual
resources changed. A routing change may advance the revision without a reoffer;
a no-op can retain it. Replaying a historical result does not reset newer local
parameters or repeat the selection.

A resource change requires a same-root signed preparation and matching captured
completion before this peer installs its receive bindings. Local publications
are preserved, new bindings are receive-only, and native hop handles remain
positive, unique and reserved across uncertain or retired preparations. Opaque
data ports remain private; their observations expose no RTP I/O. Removing a local
binding drops its queued packets; unrelated queues stay bounded and available.

`bindings: None` explicitly records unavailable concurrent capture metadata;
`Some(vec![])` records a known empty set. An unavailable capture never installs
unproven local parameters, and its preparation remains reserved pending explicit
reconciliation. This is a remaining reconciliation limit, not a claim that the
recorded effect failed. Fresh-client recovery can inspect captured parameters
without taking the original physical peer's media custody. Current qualification
and unresolved scope are recorded in `docs/evidence/gate-6/rust-sdk-selections.json`.


An owned native Connection handles server-initiated receive-resource offers for
its previously issued selectors. Before signing, the worker reads the original
server-created `stream.reconcile` operation under current authority and matches
its original `subscription_record_id`, exact scope and native revision. The
internal proposal ID and signed callback envelope ID remain distinct. A bounded
job shares the worker's command capacity; media and other control exchanges stay
live while it polls for captured completion. Confirmation alone does not change
local parameters. Unsupported receive formats receive a signed refusal.

`Connection::native_reconciliation()` exposes the latest local callback progress,
original logical/host/parent identity and `bindings_adopted`. `Recorded` describes
the journal; adoption is separately false for unavailable captures. Use
`progress.operation(&client)` to observe that original operation, save its
reference, or restore with `operation_kinds().stream_reconcile`. This kind is
read-only recovery, not another direct mutation API. Current-authorized recovery
can settle a retained matching preparation; fresh Clients acquire no transport
or media custody. A failed internal read or deadline preserves original recovery
and the old bindings without closing unrelated control work.

Resource-changing signed callbacks are qualified in
`docs/evidence/gate-6/rust-sdk-auto-reconciliation.json`.

A completed routing-only reconciliation can emit a signed `operation.updated`
journal hint on the original physical peer. The hint contains operation IDs and
counters, with no result or bindings. The SDK verifies the originally issued
selector and reads that operation under current authority before advancing its
local native revision. Stale, foreign, malformed or mismatched hints do not adopt
parameters. Coalesced pending hints cannot overwrite an unsettled preparation's
original recovery identity.

When a recorded routing-only result has unavailable binding capture, its exact
installed predecessor can advance to the recorded native counter: that operation
changed no physical parameters. No binding set is invented and
`bindings_adopted` remains false. Unavailable resource-replacement captures still
retain their original preparation pending explicit physical reconciliation.
Delivery is advisory; durable cursor catch-up, wider backpressure profiles, other
automatic negotiation families and the remaining SDK/package/profile scope stay
open under G6-060. Routing-only commits can preserve both native ports and
revision; the SDK still observes the new journal identity after a current read.
All 166 client, 43 protocol, 181 targeted Store functions and 23 selected production
functions pass in the recorded local scope. Verification and earlier failures
are recorded in `docs/evidence/gate-6/rust-sdk-routing-reconciliation.json`.

## Native reliable data

An owned negotiated `text`, `json` or `binary` Stream provides `send_data` and
`receive_data`. Use caller-persisted message and operation IDs. JSON uses a typed
`serde_json::Value` and `application/json` with `utf8`; binary preserves bytes
through canonical base64 and `application/octet-stream`. Both use
`reliable_ordered` with a maximum decoded body of 64 KiB. `DataPayload::Text`
preserves UTF-8 text with `text/plain` and `utf8` encoding.

```rust,no_run
# async fn data(stream: connect_client::sdk::Stream, target: connect_client::sdk::ParticipantId) -> connect_client::sdk::Result<()> {
use connect_client::sdk::{DataPayload, MutationOptions, NonEmpty, RequestOptions};
use std::time::Duration;
let wait = RequestOptions::new(Duration::from_secs(10))?;
let send = stream.send_data(
    "saved-message".parse()?,
    NonEmpty { first: target, rest: vec![] },
    DataPayload::Json(serde_json::json!({"sequence": 1})),
    MutationOptions { operation_id: "saved-data-send".parse()?, request: wait.clone(), extensions: None },
).await?;
let saved = send.reference_json(); // Persist original scope and identity.
let admission = send.wait(wait.clone()).await?.value;
let _ = (saved, admission); // Core queue admission, not recipient consumption.
let received = stream.receive_data(wait).await?;
let _ = (received.message_id, received.from, received.payload);
# Ok(()) }
```

Recover a full Connection-scoped send with `operation_kinds().data_send`.
Conversation `message_send` references use a different scope. A fresh Client can
observe a saved data operation under current authorization; it cannot use that
receipt to send or receive on another physical peer's Stream.

Cloned Stream handles consume one ordered queue. Queues hold at most 32 messages
per Stream, 256 messages and 1 MiB of decoded body bytes overall. On overflow,
the affected binding discards its pending bodies and returns `ResyncRequired`
until a new binding generation or format is negotiated. `Client::data_stats`
reports overflow, malformed/unowned frames and discarded old-binding bodies.
Other control and Stream queues continue. Physical signaling framing has a
separate bounded queue; its terminal transport errors remain observable.

Receive cancellation and deadlines consume no queued message. A submitted send
that loses its caller wait keeps the original uncertain operation; recover it
under the same IDs. Connection end or a changed binding prevents old queued
bodies from being consumed; Client close wakes receivers and releases queues.
Debug output redacts message bodies.

The local functional checkpoint passes 173 client tests and 21 selected production
functions, including both owned SDK peers, decoded concurrent audio, journal
recovery and all three data formats. Verification and earlier failures are in
`docs/evidence/gate-6/rust-sdk-reliable-data.json`. G6-060 remains in progress;
full SDK packaging, event streams and operating-profile qualification remain open.

## Durable event streams

`Conversation::events(after, options)` returns a lazy typed `EventStream` over an
actual native live subscription. Without a cursor, `snapshot().await` or the first
`next().await` first drains a complete authorized snapshot, retaining one revision
and journal watermark across all pages. `EventSnapshot` exposes typed resource
views and all original `SnapshotFact` values. Those observations grant no native
transport or Stream ownership.

```rust,no_run
# async fn journal(room: connect_client::sdk::Conversation) -> connect_client::sdk::Result<()> {
use connect_client::sdk::RequestOptions;
use std::time::Duration;
let mut events = room.events(None, RequestOptions::new(Duration::from_secs(10))?);
let initial = events.snapshot().await?;
if let Some(event) = events.next().await? {
    // Apply idempotently by event_id, then persist event.cursor after processing.
    let _ = (initial, event.event_id, event.cursor);
}
events.close().await?;
# Ok(()) }
```

Resume with a typed `Cursor` under current authorization. Each event retains its
original scoped IDs, revision, source, audience epochs, observed time and typed
journal data represented as JSON. The signed outer wrapper must match that fact.
Hints accompanying a batch are advisory; only journal facts are returned. Batches
enter the queue after their correlated ACK. Repeated immutable event IDs are
suppressed even when an opaque cursor token changes. Changed facts under the same
ID refuse the source. Debug output redacts payloads and cursor tokens.

The worker renews its 10-second lease using the last complete ACK watermark while
preserving queued facts. `EventStream::cursor()` reports the last returned fact
or explicit snapshot/resume boundary. It does not confirm application processing:
persist each event's cursor only after your own work succeeds. A resync, invalid
scope or current-authority refusal clears unreturned facts and keeps the previous
application boundary. Restart-bound live leases are not reused when resuming a
retained durable cursor.

Each `next` uses the configured finite timeout. Timing out or dropping a wait
consumes no fact; initialization and partial framing stay owned. Reader close or
its cancellation signal stops delivery and renewal independently of other readers,
media and control. `close` is local: the existing server lease expires within its
10-second bound, and no unsupported unsubscribe command is sent. Client close
wakes all readers. Physical signature or framing failures still retire the peer.

There is one reader per Conversation, at most eight per Client. Each reader holds
at most 256 queued events and 2 MiB of encoded journal payload, plus bounded batch
and replay metadata. Active/retired reply tags are limited to 64. Overflow returns
`ResyncRequired` and clears unreturned facts without advancing the returned cursor;
other healthy control remains usable. Close the failed reader and choose an
explicit retained cursor or fresh snapshot. SDK queue bounds are payload limits,
not process memory or scheduling guarantees.

The recorded local functional checkpoint passes 182 client tests, six Rustdoc
examples, two upstream delivery regressions and 19 selected production functions,
including live renewal with independently decoded audio and durable cursor recovery
after restart. Verification and earlier failures are recorded in
`docs/evidence/gate-6/rust-sdk-events.json`. Incoming
invitation and receipt convenience APIs, remaining SDK surfaces, complete external
packaging and operating-profile qualification remain open.

## Incoming invitations

`Client::incoming_invitations(options)` creates a lazy `InvitationStream` on the
same owned native peer. The worker retains signed invitations that arrive before
a reader attaches. A callee obtains the scoped Session handle, original invitation
operation ID, inviter and target Participant IDs, medium, intent and capability
offer from `IncomingInvitation`; it needs no copied Conversation or Session ID.
Its `operation_id` is the stable server invitation source correlation carried on
the received wire envelope. It is distinct from the caller's private mutation ID
and from your decision mutation ID. `source` identifies `session.invite` and that
same original source correlation.
Debug output omits the intent and capability offer.

An invitation is an original metadata hint. It can become stale after cancellation,
a decision or membership removal. Refresh `hint.session` before deciding, then use
`Session::accept` or `reject` with the fresh topology revision and your separately
persisted decision operation ID. These ordinary mutations enforce current authority.
Receiving or refreshing a hint never creates a Connection, accepts a Session or
acquires native media custody. The journal and current authorized snapshot govern
actual state.

There is one active invitation reader per Client, including its clones; a second
reader returns `Conflict`. Each wait has its configured timeout. Dropping a wait,
local cancellation or timeout consumes no pending hint. Close or drop detaches the
reader; a replacement can receive retained pending hints. Reader cancellation does
not cancel ordinary control, media or another durable event source. Client close
clears retained bodies and wakes the reader.

The inbox holds at most 32 invitations and 1 MiB of encoded invitation payloads,
plus 256 immutable logical identities with four bounded cancellation correlations
each. Identical original invitations with a fresh wire ID are suppressed. Changed
metadata under the same identity fails the inbox. A signed cancellation removes a
pending hint only when its exact known wire reference and logical scope match.
Overflow clears retained bodies and reports `ResyncRequired`, leaving control live.
That failure persists for this physical Client: reopening a reader does not pretend
to recover missed notifications. Use authoritative discovery and current snapshots
to reconcile pending work; a fresh peer alone is not a promise of notification replay.

The local functional checkpoint passes 189 client tests, seven Rustdoc checks,
254 contract vectors, the independently compiled public invitation example and
19 selected production functions. The two new production functions cover both
signing algorithms, all six media intents, explicit decisions, stale cancellation
and membership refusal, and joined cleanup. Evidence and failed candidates are in
`docs/evidence/gate-6/rust-sdk-invitations.json`. Full SDK/package and operating-profile
qualification remains open.

## Trusted administration

`AdministrativeClient` is a separate trusted backend interface. Its explicit
constructor takes the signed native configuration, Tenant ID and an
`ApplicationClientConfig` for the existing local HTTP provisioning API. Both
transports must use the same supplied credential. No endpoint discovery,
credential substitution, transport fallback or automatic retry occurs. The server
enforces current administrator and target authority; constructing this client
does not grant that permission.

`validate_provisioning` accepts the complete current provisioning command JSON,
normalizes and bounds it before IO, and returns `TrustedProvisioningCommand`.
Persist both `to_json()` and `reference_json()` before submission. The command's
logical ID must equal `MutationOptions.operation_id`; nonempty canonical payload
extensions are explicitly unsupported by the closed provisioning schema. All
11 existing command families are supported: Identity, Participant, Endpoint and
record-reader role provisioning, Endpoint verification, signing-key enrollment,
and Identity/Endpoint/credential/key/Device revocation. Debug output omits bodies,
addresses, key material and credentials.

`provision` returns an `Operation<serde_json::Value>`. Its completion and restored
status use signed `operation.get` with the original unscoped ID and exact recorded
command kind. Typed outcome shape, scoped IDs and safe counters are checked before
returning JSON. Current authorized metadata may reflect later verification or
revocation. A saved reference performs a read; it never resubmits provisioning.
To retry uncertain submission, reuse the separately persisted full original
command, logical ID and intent. Changed original intent remains a server conflict.
An inaccessible or absent unscoped receipt may be refused; that refusal is not
proof that a mutation never happened. Malformed or incomplete completion reads
retain the original key, report `ReadOnly` and preserve any validated observed
record; they cannot claim `NotSubmitted`. Keep the full original intent and recovery
key. Administrative clients expose no ordinary Conversation or media mutation facade.

The application worker owns at most 16 active/queued submissions and serializes
its bounded HTTP transactions. Cancelling or timing out before queue admission
submits nothing. After queue admission, local cancellation is conservatively
uncertain; an unwritten cancelled command is skipped, while started HTTP IO remains
owned until its response or the configured transaction deadline. Native status
reads remain independent of a held HTTP response. Structured refusals retain the
original operation key; transport loss, malformed completion or server failure
preserve uncertainty. `close` stops both workers, wakes queued requests and joins
both owners. A cancelled/timed-out close retains joins for a later close attempt.

The HTTP application configuration currently selects the protected loopback HTTP
profile explicitly. Native trust/network configuration retains its existing
explicit Local/Remote rules; this facade does not infer a remote HTTP trust policy.
The local functional checkpoint passes 196 client tests (seven new administrative
functions), eight Rustdoc checks, 254 contract vectors, the separately compiled
public administrative example and 22 selected production functions. Three new
both-algorithm functions cover all 11 command families, current authorization,
restart/replay and actual committed uncertainty with held HTTP response/no queued
effect. Evidence and failed/superseded candidates are recorded in
`docs/evidence/gate-6/rust-sdk-administration.json`. Full SDK/package and
operating-profile qualification remain open. Publication remains separate.


## Advanced canonical access

`Client::raw()` returns a cloneable facade over the same owned signed peer.
`validate(Value)` checks the current envelope schema and its one MiB wire bound;
it does not authenticate caller-supplied signatures or grant authority.
`ValidatedEnvelope::to_json()` exports original JSON; Debug omits private bodies.

`request(envelope, options)` preserves the complete payload, resource assertions,
logical operation ID and expected revision. It replaces the physical envelope
ID, timestamp and signature on each explicit call. Persist the original intent
and logical ID before I/O. Successful batches contain every original signed,
correlated response through the terminal ACK, including optional fields.
Canonical refusals use structured SDK errors. Cancellation preserves the same
owned exchange and mutation uncertainty as typed requests; status is an explicit
read through `operation.get`, with no hidden mutation replay.

Finite Conversation snapshots, message history and journal catch-up are available
without decoding every fact into a typed helper. Catch-up requires explicit
`live:false`; advanced callers manage its cursor and pagination. Use
`Conversation::events` for owned live leases and typed Connection helpers for
native negotiation. Auth/step-up, offer/answer/ready/ICE, binary record transfer,
server-only replies and unknown top-level intent fields refuse locally. These
lifecycles cannot be submitted as standalone finite requests. Raw results never
create native media ownership. Mutations require a logical operation ID even
when a wire family permits untracked advisory use. No fallback or older dialect
is selected.

The current checkpoint passes 200 client tests, including four new signed raw
boundary tests. Real-server restart/current-authority/journal qualification is
pending in `docs/evidence/gate-6/rust-sdk-raw.json`; G6-060 remains in progress.
