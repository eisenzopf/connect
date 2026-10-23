# Connect live transition contract

Gate 6 target; G6-006. All transitions use the operation contract and current
audience/authority rules. Handoff, consultation and device replacement are
operations on the six nouns, not new primary communication nouns.

## State and readiness

Legal operation states: accepted, preparing, ready, committing, completed,
cancelled, failed, interrupted, unknown. Admission failure returns a typed error
without accepting work. Legal state edges are defined in the operation schema;
terminal results never regress after duplicate or late callbacks.

Accepted optional transition payload fields remain part of the original business
digest, preserving nested values, array order and explicit nulls. A changed or
omitted extension under the same logical operation ID conflicts. Known typed
fields retain their published normalization and defaults; extension fields cannot
override them or grant execution authority. The Rust transition request represents
these fields in a separate `payload_extensions` map, bounded to 32 KiB.

Mutations of an existing membership or media topology require its expected
revision. Only one conflicting topology transition can hold its execution lease.
Readiness requires permitted participant/endpoint, negotiated compatible media,
verified ingress and egress readiness, and any configured endpoint confirmation.
SDP generation, a SIP answer, WebRTC signaling state or API acceptance alone is
insufficient. A readiness observation must match the current connection generation.

Before commit, cancellation restores the retained usable route and retires only
prepared resources. At/after commit, cancellation cannot promise rollback; report
the committed result and use a new authorized transition. If actual commit is
uncertain, hold conflicting new actions until reconciliation.

## Consultation

`session.consult` names the retained remote connection, explicit private
participant set and desired hold treatment. Connect prepares the private leg and
context, then switches only the authorized audience. Conversation, main Session
and remote Connection IDs remain stable. A separate provider AI session may be
required; it retains the logical AI Participant identity and receives only the
private projection. Never reuse a provider context that leaks consultation into
the public leg.

Cancel/return restores the earlier authorized speaking relationship. If the
remote hangs up, end or explicitly settle the affected engagement and cancel
pending media. Holding a connection does not fabricate a continuing remote call.

## Participant handoff

`session.handoff` names source and target participants, retained remote connection,
candidate connection if already prepared, expected revision and outgoing
disposition (`leave` or permitted `observe`). The coordinator verifies policy,
target admission and media readiness before committing speaking responsibility.
It changes only the intended role/speaking bindings and reports retained, admitted
and retired connection IDs. A model action request is merely intent.

`role_scope` defaults to `conversation`; require an additional
`expected_conversation_revision` and atomically update the Conversation role
defaults and selected Session's responsibility. Explicit `session` scope changes
only that live engagement. Other active Sessions retain their admitted role
assignments. `leave` retires the source's Session presence, not its Conversation
membership or historical access. Ordinary message arrival advances journal order,
not these topology/membership revisions.

Both AI-to-human and human-to-AI use the same operation. The outgoing observer
has no automatic access to private streams or historical content. Failed target
admission leaves the existing usable route intact.

## Target-owned native handoff preparation

A target with its own current `connection.offer` and named `session.handoff`
permissions can prepare independently of any caller operation. Its signed native
QUIC `connection.offer` contains exactly
`"handoff_preparation":{"profile":"connect-handoff-preparation-v1"}` and one
duplex audio stream. This marker cannot coexist with `private_consultation` or
carry a client readiness assertion. The actual Core admission is closed and
muted before publication, with the original Session role preserved, including
Observer. Preparation events and Connection metadata are target-only until an
attributed handoff; existing preparation events are never republished.

The public SDK `prepare_handoff_native_audio` consumes the target's physical
client and returns its immutable signed offer record through `policy_record_id`.
That record is a later handoff's `policy_ref`; no caller operation is required to
prepare it. Its signed completed result confirms `prepared_muted: true`, not
decoded duplex readiness, speaking responsibility or provider memory freshness.
Current physical grants remain necessary for media and replay. HTTP offers
remain unsupported. The native coordinator captures the actual closed admission,
decodes and discards isolated input under separate current authority, and sends
neutral local silence through the guarded writer. Its real duplex observation
is independent of the preparation receipt. Generic prepared human/AI takeover
uses that scene-issued capability, optionally consumes the old private audience,
and unmutes only the exact incoming physical Connection in the atomic managed
role/media commit. Initially Observer targets gain a speaking fence only at that
commit. Dropped capabilities, changed physical incarnations and stale queued
prompts cannot supply readiness. Provider-memory and remaining failure/recovery
qualification remain separate.

A later handoff consumes the current actual source audience, with fresh target
IO and independent current authorization. Prior unselected Observers receive
empty programmes and gain no access to the later audience or its physical
metadata. Completed operation replay cannot restore an earlier audience. An
unrelated audience commit can invalidate a closed target's old IO proof; its
bounded owner can obtain fresh actual output through the same guarded writer.

## Same-participant endpoint replacement

`connection.replace` targets the current participant's Connection and a provisioned
replacement Endpoint or already prepared candidate Connection. It retains the
Conversation, main Session and Participant; a new Connection is expected. The
original remote connection remains unchanged. Browser/phone is one configuration
of this generic operation.

An endpoint confirmation policy may require a configured sequence of 1–16
attributed DTMF keys after answer and media readiness (for example, `1`), with a
45-second default callback deadline. Separate keypresses assemble only within
the same current media incarnation. Replayed callbacks, missing observations and
digits captured before readiness cannot complete a later sequence. Voicemail,
incorrect/no confirmation, no-answer, cancellation or timeout cannot steal the
speaking route. Client-supplied phone numbers do not bypass endpoint provisioning.
The old route remains until commit; it is then explicitly retired. A late event
from its old generation cannot reactivate it.

## Identity and stream invariants

| Action | Stable | Replaced / updated |
|---|---|---|
| Consultation | Conversation, main Session, remote Connection | Private audience and provider consulting leg |
| Handoff | Conversation, main Session, remote Connection | Active participant responsibility and intended media binding |
| Device replace | Conversation, main Session, Participant, remote Connection | Participant endpoint Connection and its physical streams |
| QUIC migration | All logical objects and Connection | Physical network path only |
| Media reconnect in grace | Conversation, Session, Participant | Lost Connection and physical stream bindings |
| Session end | Conversation and authorized history | Live resources terminate; outcome record emitted |

Logical stream references can retain application continuity through an explicit
replacement mapping; transport stream IDs and RTP bindings are never falsely
reported unchanged. Physical-peer datagram handles are not reused.

## Failures and concurrency

Test before/after each prepare, readiness, commit and retirement boundary:
remote hangup, candidate failure, revocation, lease expiry, worker loss, server
restart, duplicate/reordered callbacks and competing controllers. A state conflict
is not latest-wins. Only current authority can reconcile an unknown external state;
a human attestation is recorded as an attestation, not a remote termination command.


## Current consultation admission boundary

The installed consultation admission recognizes actual native
`ready` routes only through the completed offer, confirmed effect and current
unretired same-generation runtime lifetime. A declared ready row cannot serve
as a retained native route. The private set has at least two unique Participants,
excludes the retained remote, and includes the Participant of an explicit primary
Connection. Initial admission creates no media switch. Once the registered private
candidate has completed its signed muted preparation, exact authenticated replay
can queue one actual private audio audience application and silent remote hold.
Native human completion additionally requires actual decoded audio and exact
private preparation. Current configured provider initialization and authorized
restoration are installed and qualified in the G6-039 completion audit. See
[native consultation](../docs/developer/native-consultation.md).

The current internal native invitation registration pins a fresh candidate's
original private context before allocation. Its exact signed offer negotiates
one duplex audio stream and is muted/quarantined before managed scene admission.
The signed native operation result must confirm the exact private profile, parent
record and `prepared_muted: true` before any private application content starts.
This scoped preparation does not complete the consultation or initialize provider
memory. See the native consultation implementation boundary above.

Schema 49 installs the primary's private original-audience floor before possible
audience IO. Actual Core application confirms only the audience effect. Decoded
audio is recorded separately. Schema 50 native human completion additionally binds
the exact current physical scene and verified private preparation into a separate
active lifetime, increments the Session revision once and releases the execution
lease. Established media follows current peer/membership authority independently
of the original execution credential; replay still requires current authorization.
AI provider context readiness remains required before AI completion.
Whole-Session provider transcript/context paths require Connection-bound context
after a private candidate has existed, including after restoration. Known applied audience effects survive actual media interruption, and
uncertainty does not authorize replay or remove the private floor.


## Native authorized return

`session.consult.return` is a new logical operation with its own named permission,
Conversation/Session scopes, explicit primary `connid`, `expected_revision` and
payload `{consultation_record_id, retained_connid}`. The referenced record is an
identifier, never a media grant. The caller must be the original operator or
primary owner in the original membership epoch, with current recipient ceilings.
An independent current return credential works after the preparation credential
is revoked. Replay preserves the exact request, including optional payload fields.

The runtime uses the actual original Core audience handle and a fresh bounded
complete native scene. One exact durable plan reserves at most one native attempt.
An uncertain attempt cannot resubmit. Captured actual restoration may be recorded
after command revocation; that receipt grants recording only. It restores surviving
original speaking programs and removes only the primary private floor. The private
candidate stays isolated permanently, including from an older public context with
the same Participant identity. Completion waits for actual native candidate retirement
and updates the Session topology once. Known departure of either the original
primary or remote requires the exact original native lifetime retirement observation;
a retired row alone cannot authorize return. `remote_continuing` is false when either
original Connection is gone. Restoration cannot revive or substitute the primary.
Surviving peers may make independent current subscription decisions afterward;
hold/ACK/restore alone cannot invent a continuing original call.

The return result contains `consultation_record_id`, `retained_connection_ids`,
`admitted_connection_ids` (empty), `retired_connection_ids`, `session_revision`,
`conversation_revision` (null), and `remote_continuing`. It does not change the
Conversation, main Session, original Participant or retained remote identifiers.
The current [G6-039 completion audit](../docs/evidence/gate-6/private-consultation-complete.json)
records invitation/provider and failure/recovery behavior; broader Gate6
profile, journey and acceptance qualification remain their assigned tasks.


Schema52's internal provider initialization has separate single-attempt creation
and export phases. Private JSON is unavailable until a captured actual fresh
provider reference has been recorded and checked against existing public/private
instances. Native AI audience preparation and begin refuse without independent
confirmed initialization; completion additionally needs actual decoded media and
current exact physical sources. Captured delivery/termination records are historical
observations, not current IO grants or remote cleanup commands. The Store contract
and controlled probes are documented in [private provider context](../docs/developer/private-provider-context.md);
actual provider orchestration and its complete behavior evidence remain required.


The server library's private Vapi bridge composes this contract with a fresh
signed private native Connection and a separate owned provider transport. It
records the provider's exact-message conversation acknowledgment before queuing
native audience work, keeps current history policy on ongoing candidate media,
and records actual provider termination separately from native return. This is
not a new wire/HTTP command or a model tool. Stock automatic configuration and
the remaining failure matrix are not qualified by that composition API.

An unsubmitted native preparation now carries an opaque recording-only
`NativeConsultationPreparation`. Settlement rechecks its exact kernel, attempt,
effect and plan digest under a Conversation write fence. Only durable
`started=0`, no applied observation and an unattempted prepared effect can be
cancelled. It releases only the provisional primary floor; the candidate floor
remains. A possible submission cannot be cancelled with this receipt or retried.
Primary publication is suspended while that provisional preparation is unsubmitted,
so its queued speech cannot later be reclassified as public during settlement.

Provider creation and termination remain independent. An abandoned or expired
unstarted native preparation stays `unknown` while its provider may still exist;
only captured provider termination permits the no-IO failure to settle. Recording
outages roll back the entire settlement and retry metadata with the same receipt.
This receipt grants no new media IO, even when the execution credential is gone.

The private PCMU profile also bounds locally queued provider audio to 1280 bytes
and 200 ms from receipt of its earliest fragment. A metadata/authority stall
discards expired complete and partial frames and emits silence while awaiting
fresh speech. Current audience/history authorization is still required for every
actual forwarding interval; queue age is not an authorization substitute.

An installed protected consultation initializer now accepts ordinary public
signed or HTTP `session.consult` admissions. It selects only host-configured AI
peers and application/provider profiles. Caller context authority and the
independent native delegate remain separate; the host cannot substitute owner
authority. Configured jobs are bounded, once-only and owned through shutdown.
Their original handles drain before installation retirement; incomplete drains
retain those handles. No new HTTP/wire command or model tool is introduced.

Schema53 records configured AI Session invitation and acceptance as two ordinary
authorized child receipts, their journal events and an immutable parent binding
in one transaction before private physical IO. Both `session.invite`/management
and independent `session.accept`/recipient authority are required. The original
consultation command and digest remain unchanged. Only the exact recorded
two-revision delta is recognized for that parent's execution/completion; an
unrelated topology mutation still conflicts. The result reports the resulting
Session revision, including these child mutations. The existing signed native
candidate admission remains mandatory. The configured join profile is voice
with PCMU support; public stock host configuration remains G6-044 integration.

Cancellation during provider initialization does not undo an already submitted
authorized creation/export. Captured late responses record only their exact
observations; a current cancellation forbids the next phase. If no native audience
attempt exists, initialization settles as `cancelled` only when every possible
provider is known closed. Lost creation response and unconfirmed termination
remain `unknown`, including on cancellation/replay; no new creation or export is
authorized. Captured creation/termination recording retries never repeat IO.

The invitation revision lookup on current media reads binds the original complete
intent and digest, owner, tenant and runtime directly to the immutable child delta.
It caches only the SQL program. It does not cache current permission results or
load unrelated operation effects/journal history on each packet interval.

Completed private work retains its primary, candidate and held remote against
competing consultation admission while the context is active or uncertain.
Disjoint private receiver sets remain independent. Actual return frees the original
primary/held receiver for new work, without converting the old private candidate
into a public context. Independent returns preserve the other current private
pair's speaking programs and exclusions. Candidate discovery uses only its own
current parent registration and exact native incarnation; another completed
context's old execution credential cannot authorize or obstruct that lookup.
