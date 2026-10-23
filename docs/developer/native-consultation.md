# Native consultation

G6-039 is complete for the installed canonical runtime and its qualified local
native/designated-provider profiles. `session.consult` admits durable intention. Once a trusted
coordinator registers a fresh candidate and its signed native preparation finishes,
an exact authenticated replay can apply the private audio bridge and silent remote
hold. Actual decoded audio is recorded separately. Native human consultants now
complete after verified private preparation and actual decoded media, with a
separate active private lifetime. AI provider initialization, configured invitation
and authorized return are installed; see the current
[private provider guide](private-provider-context.md). The [completion audit](../evidence/gate-6/private-consultation-complete.json)
records current behavior and its verification limits.
Original operation IDs and status/cancellation semantics are preserved.

The retained remote and an optional explicit primary must belong to the current
Session and current authorized membership epochs. The private set contains at
least two unique Participant IDs. It excludes the retained remote Participant;
if a primary Connection is explicitly supplied, that Participant must be in the
private set. A private role label grants no permission or media access.

Native Connection negotiation records `ready`, rather than the older adapter's
`active` state. Transition admission now recognizes that native state only with
a completed original offer, a confirmed effect and an unretired native lifetime
matching the same generation and current runtime. A declared ready row alone is
refused. This is recognition of a current local observation, not a new media grant.
Actual control must still obtain fresh endpoint, native origin, private-audience
and source/target authorization before IO.

The initial production case uses real signed QUIC native audio, then submits the
consultation through the public HTTP client with an explicit `session.consult`
grant. It verifies accepted intention and exact replay with zero native effects,
continued original audio, cancellation before preparation and refusal of a new
consultation after actual remote EOF. Controlled Store cases separately verify
private-set refusal before admission and refusal of unproven ready rows.

The coordinator composes the upstream private-consultation planner,
exact held audio audience policy, current native source origins and durable
single-attempt effect custody. A consultant must be prepared muted; the audience
fence and activation commit together. Saved handles retain metadata, never old
command readers. Captured results must record actual outcomes across storage
failure without another Core call. Return/cancel needs fresh restoration authority;
remote departure must not be described as a retained continuing call.

Audio isolation and durable content provenance must agree. Transcript, observation,
recording and context sources must use the private original audience rather than
ambient Session membership. Provider initialization uses the configured endpoint
and private projection; app prompts remain app/provider configuration. These
requirements, failure/recovery cases and provider integration are not yet qualified
by this admission boundary.

## Current source and planning readers

The trusted coordinator can obtain `NativeConsultationOrigin` only from an
already accepted complete original command and a current `session.consult` grant.
The opaque origin pins its record, digest, expiry and original credential ID in
the current installation runtime. It holds a weak kernel reference and no bearer,
owned command grant, SQL lock or authority reader. It cannot cross a runtime,
borrow another owner's record, change the original command or continue cancelled
work. It creates no effect and is not an execution lease.

`try_native_consultation_read` accepts the original retained native remote and a
complete selected private set, with one native Connection per declared private
Participant. It checks original membership epochs and exact primary/remote
bindings. It resolves current native Device/key/credential, verified Endpoint,
Session/profile/generation and bilateral recipient permissions, merges all
needed readers, acquires them once and resolves the entire selection again.
Private receive views additionally require each original receiver's current
`stream.subscribe` permission. No private-to-remote receive view is issued.

These planning readers expire after at most five seconds and the earlier original
source expiry. The coordinator must drop them after its bounded interval. They
hold the Conversation and affected identities stable during planning; the idle
origin alone leaves writers free. A revoked consultation or receiver credential,
a changed native generation, incomplete/duplicate/wrong selection, cancellation,
a foreign kernel or predecessor runtime refuses fresh planning. Declared ready
rows remain insufficient. Configured hold media still needs its actual configured
source and cannot silently fall back to silence.

The controlled Store tests prove these reader boundaries without Core IO.
Before IO the coordinator separately persists and reserves its exact single attempt,
binds the actual Core-issued plan, prepares the private consultant muted and installs
its durable private content provenance. The planning reader cannot substitute
for that custody.


## Durable native audience attempt

Fresh schema 46 records the selected native incarnations and complete bounded
Core audience descriptor before an execution attempt. This is an internal Store
boundary, not another public command. The original `session.consult` intent,
logical operation ID, expiry and owner remain unchanged. The effect correlation
uses the globally unique operation record ID. Native worker execution does not
require giving the ordinary caller `operation.execute`.

Preparation consumes and drops planning readers before acquiring its metadata
writer. An identical prepared plan can return its same unstarted lease; a changed
plan conflicts. Begin consumes that lease and fresh complete readers, validates
current receiver revisions and records the one possible submission. Only one
caller receives the execution token. The returned readers are capped by both
their original five-second interval and the lease deadline. Repeating begin or
replaying status never authorizes another Core call.

`try_native_consultation_scene_read` additionally takes all outside native routes.
The complete selected scene must match current native lifetime observations.
Outside Connections retain their original receive programmes. They receive only
the exact private exclusion fence, not private source views. The descriptor pins
all receiver revisions, private membership, closed private routes, declared silent
remote treatment and activation of the prepared consulting source. Unknown optional
descriptor fields remain in its hash; descriptors and selected bindings each have
a 32 KiB limit. The service must still match the actual scene-issued plan and its
controller pins rather than treating these stored fields as a Core grant.

Only an actual returned Core application outcome may consume the attempt into
`NativeConsultationApplied`. This receipt retains no command, credential grant,
selected native origin or readers. Recording retries persist the same observation
and confirm the same effect, even after recording failure, a role writer,
cancellation or membership removal. Each matching still-live original generation
can advance its native stream observation; a retired/replaced or later observed
slot is never revived or overwritten. Journal attribution uses the original actor
epoch, so readmission cannot inherit the old operation's private progress.

This audience-application observation is not consulting audio readiness or overall
operation completion. The native human completion path additionally requires actual
decoded media and verified distinct private preparation. AI context/provider
readiness remains a separate boundary. The native coordinator now calls this attempt API.
Generic worker, provider observation, attestation/reconciliation and cleanup
hooks cannot claim its effects. The dedicated native coordinator owns its later
restoration and readiness work.

Before begin, cancellation is definitive and prevents any native attempt. After
possible submission, cancellation or an engine error remains unknown until actual
outcomes establish what happened. Restart/lease expiry distinguishes an unstarted
plan (known failure with no native attempt) from possible application (unknown);
it never resubmits either. Administrative status inspection preserves a live foreign
worker lease; it does not classify that active work as abandoned. Predecessor tokens cannot record into a successor.
Controlled Store cases exercise these custody boundaries; the native cases below
exercise actual audience application and selected failure paths.

## Original transition intent

Fresh schema 47 retains optional canonical transition payload fields in the
immutable original request and digest. The signed mapper and both canonical Rust
clients preserve their nested values, array order and explicit nulls. Exact
HTTP/signed replay shares the original receipt; changing or dropping a field
conflicts without adding an effect or journal event. The extension map cannot
override known typed fields and is bounded to 32 KiB. This preservation does not
enable private Core execution.

## Quarantined native candidate preparation

Fresh schema 48 adds `register_native_consultation_leg`, a trusted coordinator
registration before any candidate allocation. It pins the complete original offer,
consultation record/digest/current credential, installation runtime, verified
Endpoint revision/Device, explicit primary Connection and original private
Participant epochs. One candidate is reserved per consultation. Registration
creates no Connection, engine effect, readiness or operation progress, and cannot
convert an existing public Connection into a private context. Repeated exact
registration is idempotent; changed intent, Endpoint incarnation or source
authority refuses. This registration is not yet called automatically by
`session.consult`; invitation/provider initialization remains coordinator work.

The registered offer carries `private_consultation` with
`profile: "connect-private-audio-v1"` and its `operation_record_id`. This reference
is not a grant. An unregistered reference, changed original offer, wrong Endpoint,
revoked original consultation credential or cancelled parent refuses before native
allocation. The candidate still supplies its own signed, currently authorized
physical offer and answers the real native confirmation exchange. The installed
preparation profile accepts one duplex audio stream. Other media cannot enter
through later Session upgrades or native controls.

The service mutes the actually confirmed original native batch, carries signed
resource confirmation across that actual gate revision, and uses Core's private
managed admission. `PreparePrivateAudio` requires a separate exact held-policy
decision; ordinary managed admission cannot imply it. The private preparation
uses the global native offer record ID internally, preserving the caller's
original wire operation ID. Its self-contained audience fence precedes scene
publication. The candidate remains muted and quarantined pending the later
private audience commit.

The completed native offer result contains `private_consultation` with the exact
profile/consultation record and `prepared_muted: true`. A client must validate
this signed operation result before initializing or sending private application
content. A native answer or ACK alone cannot establish private preparation. This
result is not decoded consulting readiness or completion of `session.consult`.

Candidate lifecycle journal entries and snapshots retain the original private
audience and hide its Connection from excluded Session members, including after
readmission. Native receive views permit only the explicitly selected physical
primary, excluding another Connection of the same logical consultant Participant.
Observation sources cannot widen the candidate's private original audience and
recheck the parent source. The primary additionally gains the private source floor
before audience IO, as described below. A provider's fresh private memory is not
initialized by these protections.

Seven controlled Store cases cover reservation, exact intent, current source and
Endpoint revision and original credential deadlines, original audience visibility, observation floors, physical
context selection and refusal to reuse a public slot. A production signed native
case negotiates a fresh private candidate alongside a public Connection of the
same logical Participant. It observes muted preparation, excluded remote snapshot
visibility, private-ingress/public-egress silence and continued retained audio.
Cancellation retires only the candidate; the original call remains usable. The
AI provider initialization remains open; the schema51 authorized-return implementation is described below. Native human
completion follows the separate established-lifetime boundary below.

## Actual private audience application

Fresh schema 49 installs the primary's original private audience before the one
possible Core submission. Both primary and candidate native receive views select
the exact other physical Connection. Another public Connection belonging to the
same logical consultant remains outside the private bridge. Observation publication
retains the original private Participant epochs. Legacy whole-Session provider
transcript/outcome and task context paths refuse with `private-context-required`
while the primary floor is active. A new Connection-bound provider initialization
path is still required; an existing public provider context cannot supply it.

The owned worker resolves the complete current native scene and calls Core's
private planner. Held callbacks match the actual `AudioAudiencePlan` and controller
pins, using current source/receiver readers capped at five seconds and both original
and controller credential expiries. They perform no SQL or grant reentry. Blocking
ledger work runs off the async executor. Preparation installs the floor, begin
consumes the one attempt, and the exact Core audience application activates the
candidate and holds the remote atomically. Configured-media hold without an actual
configured source fails before any audience effect.

The actual returned handle is retained immediately for later recovery. Its sealed
commit carries existing native confirmations across the exact changed routes.
After dropping all readers, the worker records the captured applied observation;
bounded recording retries never repeat Core IO. The confirmed effect describes
the audience commit only. Waiting for actual decoded consulting audio retains
metadata rather than command readers. Decoded audio sets an internal historical
flag. A separate native human completion path uses that observation and the exact
confirmed private preparation; it does not infer AI provider readiness.

An actual remote/primary/consultant departure, timeout, cancellation or lost native
confirmation records the first failure reason and leaves the operation unknown,
preserving a known confirmed audience effect. Replay cannot submit another attempt.
The primary floor remains through uncertainty. An unstarted definitive cancellation
can remove that floor; it cannot make a prepared candidate public.

The production cases send real native audio in both private directions and check
silence at the retained remote and old public context, including primary audio that
was audible there before the switch. Other cases exercise remote EOF after actual
application, missing configured hold before application, and cancellation of muted
preparation. Cleanup explicitly ends the Session and requires client/server shutdown.

An AI root currently remains `preparing` with no result after decoded media, under
the bounded execution lease, until fresh private provider context readiness exists.
Configured invitation/provider selection and authorized return/restoration are
installed. Current failure/recovery behavior is recorded in the completion audit.

## Established native human lifetime

Fresh schema 50 records `native_private_lifetimes` independently of the execution
credential and lease. The native human completion path requires a captured applied
receipt from this kernel, its recorded decoded observation, no recorded media
failure, the exact registered private candidate's confirmed `prepared_muted` result,
and all three original physical incarnations still current. Both the Participant
and Identity must be human. An AI or external label cannot borrow this readiness
path; configured provider initialization must establish its own boundary.

The atomic commit records the private lifetime and original audience, increments
the Session topology revision once, writes the normal transition completion with
retained primary/remote and admitted candidate IDs, journals Session/operation
progress and releases the execution lease. A storage failure rolls back all those
local changes; replay of the captured observation performs no Core IO. Historical
completion retries cannot reestablish a retired or restored physical context.
The graph commit takes the current Conversation and member Identity writers,
waits for current media readers to release, and resolves the physical scene again.
Its metadata-only historical retry does not retain that graph writer.

Preparing media reads still obey the original consultation source expiry. After
establishment, fresh reads follow the current native peer credentials, keys,
Devices, verified Endpoints, presence and original membership epochs. Metadata
origins retain the physical credential deadline separately from the preparation
interval. Revoking or expiring the original execution credential cannot revoke
those independent peer grants, and it cannot authorize replay. Current physical
revocation, replacement, departure or membership loss does stop private delivery.
Outside peers may disconnect independently without replacing either private slot.

The primary/candidate content floors persist through completed private work and
uncertainty. Another consultation cannot adopt an occupied private slot; an
authorized return or other explicit decision is required. The lifetime schema
reserves a monotonic restoration marker for the later captured restore path;
there is no public shortcut that sets it or removes a private floor.

Controlled tests cover missing decoded evidence, AI refusal, original execution
expiry, current physical revocation, wrong preparation correlation, changed native
generation, transactional recording failure and idempotent completion. The actual
signed native case observes completed status, exact replay, refusal of a competing
consultation before reservation, bidirectional private audio after execution-key
revocation and continued audio after the old public context disconnects.

The retained selection also pins each physical credential/key, public-key digest,
Device descriptor, Endpoint and original physical deadline. Current native offer,
private subscription and bilateral recipient permissions remain ceilings on
completion and continued media. Narrowing those grants cannot qualify a new active
context or widen one already established. Native invitation registration excludes
generic execution/cleanup claims before the audience attempt exists; if generic
execution already owns the root, a new native invitation is refused.


## Authorized native return (schema 51)

Submit `session.consult.return` with a fresh logical operation ID, the original
Conversation/Session/primary scopes, current Session topology revision and payload
`{consultation_record_id, retained_connid}`. It requires its own named permission,
current original operator or primary-owner epoch and current recipient ceilings.
The old execution credential is unnecessary. The root reference cannot authorize
Core or substitute a newly admitted physical Connection.

The service retains the actual original Core handle and captured application
receipt as metadata. It resolves a complete current native scene, retains its
bounded readers through one exact scene-issued restoration, and records the
captured result separately after dropping readers. Prepared and started phases
are durable; unknown submission cannot execute again. Only confirmed actual
restoration removes the primary private content floor. The candidate remains
permanently private, is closed by its native lifetime worker, and must be observed
retired before the return operation completes. The result explicitly reports
`remote_continuing`; remote EOF settles the old engagement without inventing a
continuing call. An ended original primary is also admissible only after its exact
native generation has a current retirement observation. Restoration never revives
it or substitutes another primary. Surviving Connections retain their own identities
and may make independent current subscription decisions. The committed Core group
keeps its bounded saved-program identity even after both private origins depart;
actual restoration consumes that identity once, and scene teardown clears it.
A pending original consultation becomes cancelled after confirmed restoration;
an already completed consultation remains historical completion.

Metadata recording outages retain captured receipts for authenticated replay,
which resumes recording/observation without repeating Core audience application.
Legacy whole-Session provider memory remains unsuitable after a private candidate
has existed, even after return; Connection-bound context is required permanently.
Provider initialization and configured invitation selection are described in the
current private provider guide. Disjoint private groups and independent returns are qualified by the
current completion audit. Original-primary loss and the
controlled admitted/prepared/started/captured-unrecorded return restart boundary
are recorded in the [primary recovery evidence](../evidence/gate-6/private-primary-recovery.json).


Schema52 adds the [private provider context Store boundary](private-provider-context.md).
It separates actual fresh provider creation from private export, binds both phases
to the exact private Connection and configured Endpoint, and requires independent
provider initialization before AI audience preparation/begin and completion. Private
transcripts retain original audience epochs and Connection provenance; public provider
projections cannot inherit them. These controlled Store probes do not qualify an
actual Vapi call or provider audio. Later designated native/provider fixtures
qualify the explicit no-tools controller and configured invitation; they do not
qualify a live provider account or close the remaining failure/recovery matrix.


The later server composition in `private_vapi::PrivateVapiBridge` owns a fresh
signed private native peer, actual provider creation and acknowledged context,
and a bounded PCMU forwarder. Current opaque-memory policy gates both the
forwarder and native candidate packets. The primary remains available for
independent return when only the context grant is revoked. See the private
provider guide for the exact local fixture scope and remaining stock profile,
invitation and failure/recovery work; earlier Store-only evidence stays historical.


## Independent private contexts

Completed consultation work retains its private context until actual return.
A competing consultation cannot borrow that context's primary, private candidate
or held remote. Rejection precedes receipt/effect reservation. Current floor
ownership, rather than the terminal state of the generic operation, enforces this
boundary. Definitive unsubmitted settlement or authorized actual return releases
only the original primary/held receiver; the old candidate remains private.
Disjoint receiver sets can consult and return independently in one main Session.

Candidate discovery matches the requested current operation and its exact
registered physical incarnation with `native_consultation_candidate_matches`.
It does not reconstruct an unrelated completed context's execution grant.
All current native keys, Devices, Endpoints, membership epochs and receiver
permissions remain required for actual media. The six-peer production case
qualifies bidirectional audio isolation, signed snapshots, separate returns and
public cancellation of a new intention on the returned original routes.
See the [completion evidence](../evidence/gate-6/private-consultation-complete.json).
