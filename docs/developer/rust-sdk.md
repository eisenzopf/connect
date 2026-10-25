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
