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
This adds the fourteenth mapped mutation kind. Media I/O, Stream controls,
Connection update/end/device move and consultation/handoff remain in progress.

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
