# Gate 6 public SDK contract

G6-059 freezes the shared public contract on October 25, 2026. The inventory
and current canonical boundary vectors are in `contracts/sdk-gate-6-api.json`
and `contracts/uctp-vectors/gate6-sdk-api.json`. Declaration-only Rust,
browser/Node TypeScript and Python proofs are in `contracts/sdk-api/`.
They define the implementation target for G6-060 through G6-070. Compilation
qualifies contract usability; SDK runtime behavior, packages and Gate 6
acceptance require their own evidence.

## Handles and authority

Client, Conversation, Participant, Session, Connection, Stream and Operation
handles contain stable public IDs, their parent scope and a client reference.
Constructing a handle from a saved ID grants no authority. A snapshot is a
versioned observation; it cannot authorize a later call. The server checks
current identity, credential, device, membership, endpoint and source authority
on each command and replay.

Ordinary applications select contacts, Participants and discovered endpoints.
They do not read the Connect database, copy internal route IDs, call providers
manually or receive provider keys. A private consultation and a device move
remain in their original Conversation unless the application explicitly creates
another one. Stream helpers hide negotiated physical handles. Validated raw
canonical request/status/event access remains available for advanced callers;
raw access does not bypass policy or enable an older command dialect.

## Operations, uncertainty and cancellation

Control mutations accept a saved logical operation ID. Intent, scope, message ID,
revision assertions and optional payload extensions must be preserved before the
first write. Any generated IDs must be saved through an application-owned durable
intent adapter before I/O. Wire attempt IDs and signatures may change; the
logical ID and intent may not. A changed intent under the same ID conflicts.

An observable Operation distinguishes accepted, preparing, ready, committing,
completed, cancelled, failed, interrupted and unknown. An ACK is not media
readiness. Completion has the command's actual meaning: accepting a message is
not recipient consumption, and creating an invitation is not remote acceptance.
A successful transport or advertised codec is not evidence of usable media.
Typed operation results preserve the authoritative source and scope of their
outcomes rather than translating every result into a boolean.

Waiting has a finite deadline and local cancellation. Stopping a wait, dropping
a handle or losing a helper never implies server rollback or remote hangup.
Requesting cancellation is a separate durable decision with its own saved ID and
the original operation as its target. Reconciliation requires attributed evidence;
it cannot invent successful audio or an unobserved provider outcome.

After uncertainty, reconnect with configured trust and recover the original
Operation by saved ID and scope. Query status first. Unknown, an inaccessible
receipt or a timed-out query does not authorize a new mutation ID, a changed
revision, a second dial/message, or silent fallback to another command family.
A retryable error is not permission to repeat a possible effect. Original
receipt expiry and tombstones survive reconnect; refreshing credentials does not
renew old authority.

Awaiting a mutation returns its observable `Operation<T>`, after submission or
an authoritative receipt. It does not wait for the workflow to finish. Explicit
`wait` returns `Completion<T>` with the current snapshot and typed value.
`waitUntilReady` returns an observed Operation snapshot, without asserting decoded
media. Every request/wait has a finite timeout and optional local cancellation.
This resolves decision 016's illustrative chained-await shorthand:

```typescript
const creation = await client.conversations.create(input, savedMutationOptions);
const { value: conversation } = await creation.wait(boundedWait);
const sending = await conversation.send(privateMessage, savedSendOptions);
const acceptance = await sending.wait(boundedWait);
```

`referenceJson()` produces `connect-operation-reference/1` JSON with the original
`operation_id`, expected `command` and original optional `scope`. It contains no
credential or authority. `operations.restore(saved, client.operationKinds.<kind>)`
validates that schema, IDs, scopes and command-kind match before producing the
typed local Operation. SDK-owned kind witnesses cover all 22 mapped mutations;
foreign witnesses refuse. `recover` accepts an already typed reference. Neither
restoration method performs I/O or submits work. The first application action is
`status`, with current authorization. A saved expected command is intent context,
not a claim that the current status response echoes command type.

A canonical `operation.updated` supplies ID, state and revision with optional
result/problem and opaque record correlation. Optional expiry/cancellation
observations remain absent unless the authoritative source actually supplies
them. Stored operation problems are preserved structured JSON; canonical command
errors separately preserve their numeric code/category/reason. Cancellation and
reconciliation receipts preserve target ID, state at decision, disposition and
optional evidence. The decision reference uses its own recorded scope: current
Store decisions retain cid/sid but omit the target connid. A cancellation receipt
is not proof that every possible effect was rolled back.

## Required application outlines

Every language must express the same following workflows through public handles:

| Workflow | Required intent and outcome |
| --- | --- |
| Private send | Saved operation/message IDs, explicit nonempty Participant audience, typed acceptance and attributed delivery/read outcomes |
| Consultation | Existing Session, private Participant selection and explicit retained Connection; readiness and return use the original public result/reference |
| Handoff | Explicit source/target Participants, retained Connection, outgoing `leave` or `observe`, selected Session/Conversation role scope and the required revisions |
| Device move | Existing Connection and retained Connection, exactly one discovered endpoint or prepared candidate, observable preparation/commit/retirement |
| Recovery | Restore the original Operation on a fresh client and query its status; do not submit a replacement mutation |
| Incoming invitation | Typed attributed event, current scoped snapshot and explicit accept/reject decision; no copied internal route IDs |
| Cancellation | Demonstrate local wait cancellation separately from a durable cancellation decision |
| Capability refusal | Unsupported control/media/runtime roles return a structured refusal without fabricated readiness or a legacy downgrade |

The 17 shared wire vectors exercise the corresponding current canonical shapes.
They are contract boundary fixtures. Rust, browser/Node TypeScript and Python application outlines and their
result/error declarations type-check. Six unsuppressed negative cases per compiler
target reject mixed IDs, wrong revision families, ambiguous move targets, empty
private audiences, participant provisioning and wrong recovery result types.
Executing installed packages is separate G6-070 work.

## Command inputs and outcomes

Private Conversation sends carry only cid, original message ID and explicit
nonempty Participant audience. Session lifecycle commands carry cid/sid and
exclude connid. Invitation `medium` is `voice`, `video`, `voice+video`,
`screen-share`, `text-chat` or `mixed`; Stream media kinds remain audio/video/
screen/data. Empty invitation targets permit the canonical single-Participant
Session. Conversation creation uses the configured tenant and canonical policy;
the identity-list convenience assigns the ordinary participant role. Advanced
canonical requests retain all supported explicit fields.

Participant admission requires explicit identity, kind and role. Participant
mutations compare the Conversation revision. Session controls and invitation
decisions compare topology revision, as do consultation, return, handoff and
device move. `SessionSnapshot.revision` observes state_revision and cannot
authorize a Session control command. Session decision results expose
`topologyRevision`/`topology_revision` for the wire DecisionOutcome.revision. Native stream controls/subscriptions compare the negotiated Stream
revision. A journal Connection resource revision cannot stand in for it.

Consultation/handoff/move results retain every retained, admitted and retired
Connection, plus topology revision and any Conversation revision. The current
`TransitionCompletion.session_revision` wire field is a topology revision. The
SDK adapts public IDs into scoped handles without granting authority. A move also
identifies its resulting Connection; consultation retains the original Session
and SDK-managed return correlation. Physical route IDs stay internal.

Session update includes participant leave, role change, medium upgrade and codec
renegotiation. Connection update includes mute/unmute, hold with explicit
direction, resume, quality interval and codec renegotiation. Unsupported actions
refuse; local capture pause remains a separate media lifecycle operation.

## Snapshots, events and errors

Session snapshots use the current Rust `session_control::SessionView`: `id`,
`conversation_id`, `state`, `media`, `intent`, `capabilities_offer`,
`codec_preferences`, `is_main`, `revision`, `topology_revision`, `created_at`,
`updated_at` and `ended_reason`. Appearances, Connections and Streams are separate
scoped facts. An initiator/counterpart pair cannot substitute for the Participant
set. Shared numeric revisions must remain exactly representable; media clock
64-bit values use lossless typed/binary or explicitly encoded integer forms.

A current-source audit finds that TypeScript's SessionSnapshot interface and
Python's canonical event snapshot validator already use this SessionView shape.
The older blanket finding in the implementation plan is stale for those two
paths. Python's generic legacy `session` result validator still uses the older
shape; the remaining facade migration must be handled in G6-060/G6-062/G6-063,
without a compatibility alias. Schema agreement here is not complete SDK parity.

Events preserve original actor, source, audience and scoped IDs with audience-bound resumable cursors. Specialized incoming invitation events carry public Session handles; kind-specific payload validation follows the current protocol event models and shared event matrix. Advanced/extension payload data remains JSON. Notifications are hints. Actions use
current snapshots/status and authority, not cached presence, caller ID or a
notification. Cursor loss/refusal and resubscription are explicit. Closed or
foreign cursors must not silently select a broader audience.

Structured errors preserve safe canonical codes and context. Configuration,
trust/authentication, protocol, timeout, local cancellation, transport uncertainty,
capability, conflict and application failures remain distinguishable. An error
must preserve the original Operation reference when acceptance is uncertain and
must not expose credentials, provider keys or raw private payloads.

## Media and client ownership

Media frames are bounded ephemeral I/O rather than journal mutations. A Stream
handle carries declared format, direction, source timing/provenance and ownership
rules. `Stream.refresh` returns available owned/negotiated format, direction and source clock metadata; unknown metadata stays absent. Source clock units are lossless (`bigint` in TypeScript, integer in Python/u64 in Rust). The handle distinguishes observation, local source, local sink and local duplex ownership. Codec support, negotiated transport, current source clocks and observed
readiness remain separate facts. Slow consumers, helper death, clock loss and
source replacement must be explicit and bounded. Local close/pause cannot become
a claim of confirmed remote termination.

Browser device capture/render is distinct from Node/Python programmatic I/O and
packaged codec utilities. Node/Python are not promised arbitrary desktop capture.
Clients own helper processes, readers, writers, timers and callbacks; bounded
shutdown reports incomplete cleanup explicitly. Ending one logical Connection
must not retire another one sharing a physical peer.

## Roles, runtimes and provisioning

Rust provides native control/media and a supported trusted composition boundary.
Browser TypeScript uses canonical WT or WSS/WebRTC with feature detection and
separate capture/render. Node and Python use managed helper assets or an explicit
remote HTTP control facade. HTTP/CLI/MCP are control/status surfaces; media frames
do not appear in their control responses. A transport switch after accepted or
uncertain work starts with original-ID status recovery.

The approved Gate 6 minimum targets are Rust 1.97.1, Node 22 and Python 3.12.
The Python package currently declares 3.11; G6-063 must reconcile its manifest
and qualification with the approved 3.12 target.
Qualification pins and browser/media roles come from decision 015, not from a
claim that every newer runtime is already qualified. Helper/server/profile
mismatches fail explicitly; helper assets must be managed compatible package
assets, with a caller-supplied executable only as an advanced override.

Trusted provisioning uses a separate administrative client/capability for
identities, endpoints, scoped credentials and policy. Ordinary participant clients,
browsers and agent tools receive neither that capability nor provider secrets.
Server embedding does not require applications to use private Store internals.

The selected package candidates remain `connect-client`, `@eisenzopf/connect` and
`rudeless-connect` (Python import `connect_sdk`). The read-only registry preflight
is recorded with its date and response digests in the preparation evidence.
A 404 is not reservation or namespace ownership. Publisher credentials and the
release version remain unverified and must be checked in G6-069 before packaging
or any separately authorized publication. Current npm/Python manifest names are
migration work, not an alternative compatibility promise.

## Verification and implementation handoff

`check-gate6-sdk-contract.py` checks canonical schema/refusal vectors, current
source boundaries, all 13 Session fields and 22 command-kind witnesses across
languages. `check-gate6-sdk-outlines.py` checks the same full application outline
in Rust, browser TypeScript, Node TypeScript and Python 3.12 typing, including
unsuppressed negative compilations. Reports bind inputs and logs by SHA256.
The evidence is `docs/evidence/gate-6/sdk-contract-final.json`.

G6-060 implements the Rust handles, async Operations and current typed event/result
mapping; G6-061 implements native programmatic I/O. G6-062/063 implement the
TypeScript/Python contracts and managed helper ownership; G6-065 migrates control
consumers. Transport connect/trust/configuration and platform capture/frame APIs
compose through their existing approved profiles. None of this contract proof
claims current runtime parity, receipt availability after expiry, media readiness,
Linux/remote/provider qualification, package ownership or publication.

The revision 2 Store-source audit corrects the original contract's assignment of
Session command preconditions to the state revision. Both `session_control.rs`
and `session_decisions.rs` compare and increment `topology_revision`. Declaration
proofs retain distinct state/topology types and use the latter for these calls.
The original freeze evidence remains historical; the correction and new evidence
are recorded in `docs/evidence/gate-6/sdk-session-revision-contract.json`.

The existing-Session `inviteParticipants`/`invite_participants` helper uses the
same `session.invite` command with an explicit topology revision and nonempty
targets. It preserves original intent/capabilities and uses current media.
A newly invited Participant can join an already accepted Session. The application
outline skips only terminal Session states and submits under current server
appearance/authorization checks; global Session state is not an invitation grant.
Membership receipts project currently authorized metadata. A same-ID replay can
observe a later role/revision; crossing a membership epoch refuses the old receipt.

Decision receipt `target` is the observed target Operation ID. It carries no
invented command or scope for that target; a decision restored from saved JSON
can only report fields its authoritative receipt provides. Typed cancellation
and reconciliation refuse nonempty payload extensions until those fields are
part of the durable decision identity. Mutation server failures keep the original
ID and uncertain acceptance instead of implying that repeating under a new ID
is safe. Actual effect reconciliation still requires attributed host evidence.

Revision 3 follows the G6-060 native implementation audit. `Connection.update`
returns `ConnectionUpdateReceipt`, including the scoped Connection, native
revision/generation and typed codec, gate or quality outcome. Stream selection
returns `StreamSelectionReceipt`: `bindings` is explicitly nullable when the
concurrent historical capture was unavailable. A known empty capture is an empty
collection. Neither result invents Stream handles or grants local physical
custody. The Rust/TypeScript/Python declarations and original-operation kind
witnesses use these results consistently.

Subscription inputs accept nonempty sets of explicit Stream or Participant/media
selectors. Connection codec changes may target a stream subset. Connection end
retains the application's reason and optional native StreamRevision assertion.
All three declaration outlines compile a receipt/lifecycle journey. The real
public Rust crate separately compiles
`crates/connect-client/examples/native_receipts.rs`; the contract checker audits
four receipt/parameter field sets against those actual implementation sources.
This correction changes target declarations, not server wire behavior or SDK
physical ownership. Evidence is recorded in
`docs/evidence/gate-6/rust-sdk-contract-alignment.json`.
