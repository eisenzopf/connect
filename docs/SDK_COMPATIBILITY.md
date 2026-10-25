# Gate 6 public SDK contract

This is the G6-059 contract candidate. It refines decision 016 using the current
canonical service. The handle/method inventory and refusal vectors are in
`contracts/sdk-gate-6-api.json` and `contracts/uctp-vectors/gate6-sdk-api.json`.
They define a target for G6-060 through G6-070; they do not certify current SDK
implementations or installed-package parity. G6-059 stays in progress until the
same complete application outlines are checked in every language and the final
surface review is complete.

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

The final ergonomic await syntax remains under review. The target must retain
both the Operation reference and bounded convenience waiting; an example must
not silently return an unobservable completed value or wait forever.

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

The shared wire vectors exercise the corresponding current canonical shapes.
They are contract boundary fixtures. Type-checked Rust, browser/Node TypeScript
and Python application outlines and their result/error declarations remain
required before this candidate is finalized; executing installed packages is
separate G6-070 work.

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

Events are attributed hints with audience-bound resumable cursors. Actions use
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
rules. Codec support, negotiated transport, current source clocks and observed
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

The current minimum declarations are Rust 1.97.1, Node 22 and Python 3.11.
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
