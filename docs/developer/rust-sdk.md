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
