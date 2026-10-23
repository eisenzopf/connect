# Same-participant device replacement (G6-023)

Replacement preserves the Participant, Conversation, main Session and retained
remote Connection. A distinct provisioned Endpoint receives a new Connection.
Other devices of the same Participant keep their existing bindings. A QUIC path
migration changes no logical Connection and cannot be submitted as device
replacement on the same Endpoint.

The replacement model prepares a candidate from the exact original binding
incarnation. Its lineage names the original Connection and next binding
generation. Preparation and adapter observations do not publish the candidate,
change roles, or retire the source. A replacement can use the same logical slot
at commit; it does not add a second permanent Connection at capacity. Temporary
physical resources still require the coordinator's bounded reservation.

The host selects an explicit confirmation policy. `MediaOnly` requires actual
current media readiness. `AnsweredDtmf` additionally requires an attributed
answer and the configured exact digits after media readiness. There is no
hardcoded telephone product action or default confirmation digit. No-answer,
voicemail without the configured confirmation, incorrect digits, early digits,
and callbacks naming another Connection or generation do not complete the
replacement. Duplicate confirmation is an observation, not a second effect.
Only the trusted adapter/coordinator may record these observations after matching
its actual IO/callback to the ticket; a wire body cannot be mapped directly to
the model methods. Observations grant no authority.

The owner's final fence rechecks current memberships and verified target
Endpoint, the original binding incarnation, Participant revision, retained
remote and configured confirmation. It stages a complete local model candidate
before publishing anything. The managed coordinator publishes that candidate together with the actual guarded
media programme under current operator/media authority and actual input/output
generation proof. Qualification is recorded separately from implementation. Only the
intended old Connection retires. The Participant role and other devices remain
unchanged. Cancellation, deadlines, stale state or denial before commit preserve
the healthy original route.

A local observation records replaced/admitted/retained Connection IDs, Endpoint,
device, membership epoch, binding generation and one Participant revision.
Exact historical replay requires fresh current authority, binds actor and target
identity/epoch and original intent, and preserves original correlation metadata.
Operation-ID protection shares capacity and namespace with other participant
mutations and handoffs. Old IDs remain tombstoned; an old-leg end event cannot
remove the new binding. History does not assert that a bridge remains active.

Physical cleanup is a later observation. Unknown external outcomes cannot undo
a known local commit or cause redial. Connect's durable journal and restart
reconciliation remain authoritative for complete public operations.

The current patch contains model staging, lineage, confirmation and explicit
provisional resource activation through sealed `PreparedMediaObservation`.
Ordinary initial activation still refuses replacement lineage. The explicit
replacement resource view pins the candidate and original binding incarnations,
current owner/endpoint/generation and same Participant/Session. Current policy
must authorize provisional IO and keep the candidate quarantined. The resource
view retires only a local copy of the source metadata and does not change the
Participant model, require DTMF to have completed, or claim working duplex media.
Fresh packet policy merges only a committed, uncancelled resource observation;
a cached aborted proof, retired/recreated source, revoked Endpoint or foreign
owner cannot authorize it. The actual held policy guard remains owned through
resource preparation; retained proof objects keep no authorization reader.

The Core scene primitive now prepares a sealed swap from an actually isolated
candidate, its exact pending binding and current decoded-input/successful-output
generation evidence. It requires configured confirmation independently. It
retargets only the original Connection and logical stream selections, keeps
Participant selection intent and exclusions, transfers the original receiver
programme, and preserves unrelated devices. The original closed audience is
retargeted to the new controller while the candidate isolation is removed in
the same drained commit. Previously blocked exact receiver selections remain
dormant metadata behind the new audience fences. The primitive copies the
original pause, directional hold, source mute and actual speaking role without
copying physical codecs, handles or transport IDs. Cached restore is disabled.
Only the intended original controller closes after publication; local cleanup
can remain unknown while the new bridge stays active.

A separately authorized local audio path supplies a preparation prompt without
borrowing private call media or inventing a remote source. The default
`MediaPolicy::current_local_audio` returns Unsupported. The host must authorize
the exact `LocalAudioPurpose::EndpointPreparation` operation at enqueue and
actual send under current ownership. Payloads use the current negotiated audio
binding and bounded egress queue; queued objects keep no authorization reader.
Actual send repeats credential, port, consumer, scene and subscription checks,
retains the current grant and delivery guards through bounded IO, and records
output readiness only after successful completion. Failed sends, changed
credentials/generations, closed resources and stale queued prompts cannot lend
readiness to another preparation or to the committed call.

Staged ordinary admission now adopts the actual verified controller binding
incarnation under its final publication fence. It refuses replacement lineage
as ordinary admission. This preserves one real source identity for later moves
instead of constructing a second binding with the same public IDs.

The managed preparation reserves its never-reused Connection ID before physical
allocation. `ParticipantLimits::replacement_preparations` separately bounds extra
temporary resources (default one, zero disables preparation). Admitted connection
capacity is independent: the committed new route inherits the original route's
slot, and only the temporary reservation is released. Filling the admitted slots
does not prevent an authorized make-before-break move.

`prepare_staged_device_replacement_with_held` creates an owned, bounded preparation
with no idle authorization reader. Native allocation consumes its exact reservation
through `prepare_peer_native_device_replacement` and a real authenticated physical
peer. Other transports must explicitly opt into synchronous dormant allocation via
`prepare_device_replacement_connection`. The request contains exact frozen
relationship facts and a trusted adapter context; the adapter returns its derived
authenticated endpoint principal. Allocation cannot signal, publish events or call
Core. Ordinary origination cannot consume the reservation. A separate current
operator grant controls `start_adapter_device_replacement_with_held`; activation
uses the same adapter route and idempotent stable external receipt. A lost receipt
remains uncertainty; Core does not automatically allocate or dial again.

The physical resource lifetime is independent from the caller's preparation
cancellation. Before known commit, cancel/drop/deadline/source loss disposes only
the candidate and returns its temporary reservation. Actual authoritative adapter
liveness is checked as well as lifecycle identities; event hints are insufficient.
After known commit, late cancellation cannot tear down the new route or roll back
the bridge. Normal adapter/Session teardown owns that route afterward.

The candidate must have explicitly authorized muted audio before private
attachment. Activation installs a singleton closed audience. The separate
`current_local_audio` hook can permit host-generated preparation audio; call media
cannot be borrowed to make readiness. `capture_device_replacement_event` accepts
only the actual native peer or a concrete trusted adapter callback context. It
pins source and target credential incarnations. `MediaObserved` evaluates real
isolated duplex IO, never a caller readiness boolean. Answer and DTMF remain
separate attributed observations. The explicit `current_device_replacement_signal`
hook defaults to Unsupported and can reuse a callback's actual held domain grant
without acquiring another reader behind a queued writer.

The final compound fence validates current role/media control, memberships,
Endpoint, source/retained/candidate lifecycles, exact isolated IO generations,
confirmation, revision and owned reservation. One publication replaces the intended
Session/ParticipantSession binding and media programme; it preserves role, other
devices, retained remote and main Session. `participant_replacement` is populated
only by this managed commit. Raw scene outcomes retain no logical receipt. Local
cleanup and fresh-current-authorized historical replay are separate observations.

`scripts/check-gate6-device-replacement.py` requires the managed browser/native/
telephone failure/recovery fixtures, Core regressions, current contract vectors,
source stability and exact executable digests. Runtime results are written to
`docs/evidence/gate-6/device-replacement.json`. Earlier Core/model/resource/raw
scene checkpoints remain historical and do not by themselves qualify the managed
implementation. A task completion must cite the successful complete report.
General wire DTMF/transport/quality stays G6-024; production Store/HTTP/SDK/journal
and public Session projection stay in their assigned tasks. Linux x86_64, the
approved remote/native/browser profiles and complete Gate 6 acceptance require
their own assigned evidence. Publication remains separate.


## Connect operation source and preparation control (G6-041)

The Store now exposes `native_replacement_origin` for an already admitted exact
`connection.replace` command, plus `native_replacement_context` for current
metadata. The weak source pins its installation/runtime, original owner,
credential, digest and expiry without retaining an authority reader. A new
same-owner credential can acquire a fresh source before custody begins; it
cannot lend its grant to a revoked old source or extend the original expiry.
Cancellation, generic worker custody, changed original generations, membership,
topology and target endpoint revision invalidate the source.

The destination is the original provisioned endpoint, or the endpoint frozen
with the asserted candidate at admission. It must belong to the same participant
identity, use a current unrevoked Device, remain verified, support audio and
differ from the original endpoint. Connection/endpoint ownership is immutable.
Caller extension fields cannot replace its provisioned address. Source and
retained IDs resolve to the current installation's original confirmed native
lifetimes; metadata does not assert actual engine liveness or candidate IO.
Debug views redact source and destination details.

`try_native_replacement_preparation_read` additionally resolves the exact opaque
source and retained `NativeMediaOrigin` values and independently current signed
physical credentials under one merged current-authority interval. The original
controller and both physical peers retain separate permission checks. The owned
reader expires within five seconds or an earlier source/operation deadline,
checks again after fence acquisition and grants no candidate publication,
readiness, answer, DTMF or final commit. Original physical audience members remain
in its required footprint; unrelated identities can proceed. It must be dropped
after the bounded Core control action, never retained in a preparation ticket.

Public replacement now queues the bounded, real Core preparation described
below, plus the independently signed target allocation phase. Isolated target IO,
configured telephone callback/DTMF, commit/projection and the complete failure/recovery matrix remain
required before G6-041 can close.

## Durable managed preparation custody

Fresh schema 55 retains `native_replacements` and `native_replacement_steps`.
`prepare_native_replacement` consumes the bounded source read, drops its readers,
rechecks under metadata writer authority and freezes the original source snapshot,
provisioned target revision/Device, source role, candidate logical/runtime IDs,
next binding generation, Core participant revision, original confirmation policy
and a bounded deadline. Reservation does not create a Connection, allocate a
native peer, dial, promote a role or establish media readiness. The configured
telephone path requires answer/DTMF policy; there is no default confirmation digit.
The host-only `TelephoneReplacementPolicy` additionally binds a configured trunk
reference, SIP authority, explicit DTMF digits and a 1–60 second preparation
deadline. Store preparation requires the current operator's independent
`sip_dial` permission as well as `connection.replace`, the verified telephone
Endpoint's matching secret reference and a current route owner. The route owner
is fenced in the same merged source/retained authority interval. The dial URI is
derived only from the Endpoint's provisioned E.164 address and configured SIP
authority; payload extension destinations and confirmation digits confer no
authority. The complete policy and route are frozen in the original immutable
plan, and begin rechecks them. Changed policy conflicts, revocation refuses begin,
and an uncertain preparation cannot reserve another attempt. This is preparation
custody evidence only: production telephone adapter allocation, actual call
submission, attributed answer/DTMF and callback recovery remain unimplemented.
Candidate IDs remain reserved after cancellation/failure/unknown and cannot be
adopted by another intention. Duplicate preparation retains the original IDs,
policy and deadline; changed inputs conflict.

The preparation lease retains metadata only. Immediately before the actual Core
reservation action, `native_replacement_preparation_attempt_read` reacquires
current independent source/retained grants, then
`begin_native_replacement_preparation` rechecks the exact plan and atomically
records the single possibly invoked preparation attempt. Concurrent duplicate
begin never permits a second call. Changed role, endpoint, generation, current
permission or cancelled intention refuses. Generic workers, cleanup and operator
reconciliation cannot take this native custody or manufacture its result.

Only a trusted runtime holding that attempt may capture the exact real owned
Core ticket via `NativeReplacementPreparationAttempt::prepared`. Attribution
must match the original operation, Session, participant/epoch, retained/source/
candidate runtime IDs, target Endpoint/Device, generation and participant revision.
Additional ready fields refuse. `record_native_replacement_prepared` persists
that historical fact idempotently even after cancellation/revocation, without
minting an IO grant or result. Its owner-only progress explicitly reports
`media_ready: false`. A foreign installation cannot record the ticket.

A possibly invoked preparation is never automatically reexecuted. Uncertainty
keeps its effect/ID reservation. Restart fails an unstarted preparation and
retains started/captured work as unknown; predecessor handles do not become
successor grants. Captured facts remain attributable. Backup inspection includes
the reservation and step tables. Older schemas require the documented explicit
backup/reset flow; no automatic migration or downgrade is installed.

The installed custody covers **prepare** and **allocate** as distinct attempts.
Start and commit remain closed. Target negotiation/resources, callback/DTMF,
actual managed commit, durable Connection projection/retirement and their complete
failure matrix still remain G6-041.


## Installed Core preparation worker

For a live original native source and retained route with a current provisioned
Quic target, the public `connection.replace` command now queues one actual
`prepare_staged_device_replacement_with_held` invocation. The installed
`ParticipantReplacementControl` checks the independent current original physical
sources and operator, exact Session/participant/epoch, original role, target
Endpoint/Device and frozen mutation. It grants only `ReplacementPhase::Prepare`.
This hook does not grant later control phases. Native allocation uses the separate
current signed-target boundary below; target media/start/commit and uninstalled
transports remain closed.

The runtime captures the actual owned ticket's binding and appearance, verifies
its role and complete prepared plan, and records the fact through the once-only
Store attempt. The owned ticket and recording receipt retain no authority reader.
A fresh nonblocking source check watches for cancellation, current authority or
endpoint loss; transient writer contention does not by itself cancel the ticket.
Core separately watches its original resource lifecycles and bounded deadline.
The original native media and role remain usable during preparation. This phase
allocates no target peer or Connection and claims no target media readiness.

Definitive preflight refusal before reservation records an attributable failure
for the exact original command. It cannot close reserved or possibly invoked
work. After actual cancellation of an uncommitted Core ticket,
`record_native_replacement_preparation_stopped` records idempotent local cleanup
only if no allocation/start/commit attempt has begun. It can capture this fact
after controller revocation, without restoring that controller's grant. A
preparation receipt cannot claim that a possibly submitted call was undone.
Captured preparation retries only its metadata write; it never invokes Core a
second time. Cancellation releases the owned Core preparation capacity while
candidate IDs remain tombstoned in the journal's durable reservation.


## Signed native target allocation

A separately authenticated target calls the public SDK's
`allocate_native_replacement_target(original_command, original_reply, offer)`.
The SDK preserves the original replacement operation ID, Conversation and main
Session. It selects the command's candidate ID, or the stable default
`replacement_<record_id>`, and sends a fresh signed native `connection.offer` with
`expected_revision: 0` and this application profile:

```json
{"replacement_preparation":{"profile":"connect-replacement-preparation-v1","operation_record_id":"<original replacement record>"}}
```

The offer must contain exactly one duplex audio stream. The marker is an exact
reference, not authority. The Store independently checks the target's current
`connection.offer` permission, signed Device/key and current same-participant
membership, the exact original candidate, current provisioned Endpoint/Device,
original operator and both original physical sources. It refuses foreign,
changed, expired and combined consultation/handoff profiles. The existing target
command interval must cover the entire original footprint; there is no nested
reader or fallback to a smaller authority set.

`begin_native_replacement_allocation_with_peer` atomically records one possibly
invoked **allocate** effect, freezing the complete target intent/credential/key
binding before Core allocation. The runtime consumes the exact actual
`DeviceReplacementPreparation` through `prepare_peer_native_device_replacement`
and retains it on the admitted physical peer. It captures the actual Core ticket
binding/generation and actual live native peer allocation. A second physical peer
cannot take that allocation; identical replay on the original peer returns
progress, while changed content conflicts. Uncertainty never permits another
allocation, including after restart. Captured recording retries only metadata;
it reuses an existing target interval when that interval is already held.

Allocation leaves the candidate's media closed and does not negotiate streams,
answer an offer, confirm readiness, admit a speaking route, change roles or
commit movement. The operation remains `preparing` with no result. At this
checkpoint, the durable reservation/attempt records its actual Core binding;
there is still no candidate Connection row or new public media authority.
Original decoded audio and roles continue. Independent target key/endpoint loss,
controller revocation, physical loss, public cancellation or the original bounded
deadline cancels the owned candidate. Actual native absence must be observed
before `record_native_replacement_allocation_stopped` can settle local cleanup;
a prepare-only receipt cannot settle an allocated candidate, and an allocation
receipt cannot settle a later started/committing phase. No unknown callback or
possibly started target IO is inferred to have been undone.

## Signed native resource start

After allocation, the same target physical peer calls
`begin_native_replacement_media(original_command, original_reply, original_offer)`.
This returns the fresh request ID and scope. It sends the same frozen target
offer with profile `connect-replacement-media-v1`; changing the Device, key,
stream, endpoint, original operation or candidate conflicts before media IO.
The client receives the correlated `connection.answer` followed by the server's
`connection.offer`, then signs `answer_connection_offer` with the exact offered
parameters and returned scope. As for initial native negotiation, an answered
offer has no invented final ACK. Read the original operation status to observe
the resource capture. This phase must not emit `connection.ready` or complete
endpoint movement.

A separate once-only **start** effect freezes the fresh signed target intent
before resource configuration. Metadata planning and historical recording may
retry; resource configuration and negotiation cannot. The resource policy adopts
the actual Core `PreparedParticipantReplacement` binding and appearance, with
next generation and `replaces` lineage. It never reconstructs an ordinary
first-generation binding. Native resource activation requires the signed exact
full-batch parameter confirmation and the sealed Core resource observation.
The actual negotiated audio controller is initially muted at stream revision 2.
The captured observation explicitly records `media_ready: false`.

The provisional policy keeps public publish/receive closed and requires fresh
current original source, retained route, operator and target authority. The
stored media origin contains immutable metadata, with no idle authority reader.
Captured resource proof alone cannot attach a candidate, establish duplex IO,
confirm an endpoint or commit movement. The owned private continuation described below separately establishes attachment,
activation and real input/output readiness. Durable candidate Connection projection
and final movement remain later phases.

A rejected signed parameter answer or local negotiation failure records an
uncertain start and cancels the owned candidate. Historical uncertainty is saved
before awaiting its metadata write, so the owned watcher can finish recording
without executing again. Only actual retirement of that exact local native
runtime ID and generation can settle the failed resource preparation. This does
not infer that a telephone callback or any provider submission was undone. A
captured or uncertain start cannot settle cleanup after movement commit has
possibly begun. Cancellation or authority loss also cleans an already cancelled
owned preparation; committed replacement handles refuse cancellation. Old source
media and role remain intact throughout this preparation and cleanup.

## Private native continuation

A successful signed resource start queues an owned local continuation. Schema 56
records separate once-only `attach`, `activate`, and `readiness` observations.
Each phase requires fresh current target, original operator, source and retained
authority. A scene change additionally resolves every existing native controller
and its authorized source pairs under one complete authority interval; owning a
scene route does not add `stream.subscribe` permission. SQL and authority
admission remain nonblocking under engine fences. The read uses one SQL permit,
then reacquires the complete authority set once and revalidates all facts; it
does not repeatedly re-enter SQL behind status and watch requests.

The target controller uses the same policy object as the managed scene. An exact
weak target registry dispatches its separately typed replacement grants without
constructing an ordinary first-generation Connection. Attachment consumes the
actual Core candidate and authenticated native peer. The explicit sealed muted
activation plan keeps the one-target closed audience muted with an empty receive
programme. The muted mode participates in exact plan identity; ordinary private
consultation activation retains its existing speaking behavior. Cancellation
retires this candidate-only audience rather than retaining a restoration entry.

The scene issues a decoder-only preparation for that exact current one-member
closed muted audience. Its live controller, port, generation and scene group
must remain current. Input is decoded and discarded without ordinary publishing
permission, including for an Observer. Separately authorized local silence uses
the exact negotiated codec and actual guarded writer; enqueue alone proves no
output. Each decode and send reacquires current independent authority. The probe
and queued payloads retain no idle authorization reader. Dropped preparations,
retired consumers, replaced groups or changed credentials invalidate old IO.

Only a trusted event captured against the actual owned native peer and candidate
can evaluate decoded input and completed output. The current signal grant is
separate from public routing. A positive Core `MediaObserved` result under the
configured native `MediaOnly` policy permits recording the historical readiness
observation. It does not create a candidate Connection row, publish the target,
change its role or complete the movement operation. A later movement commit must
revalidate actual current media and endpoint confirmation; a stored readiness
flag cannot substitute for that proof.

Captured local phase facts remain historical after cancellation. Actual
retirement of the exact native runtime/generation can settle local attachment
and activation cleanup, including a possibly started phase whose outcome is
unknown. The same proof refuses after movement commit may have begun. It says
nothing about telephone/provider submission outcomes. Managed movement publication
and the telephone confirmation/recovery matrix remain separate required work.


## Store movement custody and established target projection

Schema 57 gives movement its own once-only `commit` attempt under the complete
current scene grant, after confirmed private attachment, activation and actual
readiness. The expected managed participant observation pins the source,
candidate, retained connection, membership epoch, endpoint, device, next binding
generation, role, configured confirmation and next participant revision. A
captured ready observation must identify that exact target and negotiated
streams. These trusted runtime capture APIs accept no wire ready claim.

The owner retains a known or uncertain capture before awaiting its metadata
write. Retrying a captured write cannot execute movement. A possibly started
commit prevents candidate-only cleanup, generic worker execution and automatic
retry, including after cancellation. Historical recording remains possible
after original controller revocation. A confirmed movement capture alone does
not complete the public operation or create a connection.

A separate atomic projection consumes that exact confirmed capture. It records
the candidate's actual generation and replaces lineage, preserves its participant
and main session, retires only the old connection, advances topology, records
ready/retired journal facts and completes the original replacement operation.
A failed projection rolls back all of these writes. Retrying it creates no
second connection, operation or lifecycle event. Old native cleanup may arrive
before or after projection. Lineage compares immutable native admission
generations, so an old row's later retirement fence cannot invalidate its
already committed successor.

`native_replacement_lifetimes` binds the real replacement root to the independent
target credential, signing key, device, endpoint and original key lifetime.
`native_admission_sources` presents initial and replacement admissions to stream,
data-write, handoff, replacement and session authorization without fabricating
an initial native offer. The replacement root still identifies its original
source; established controls resolve the separate candidate connection. Current
packet reads check the target's own authority. Old operator/source revocation
cannot revoke the target, and target revocation cannot borrow the old source's
permission. Original readiness deadlines do not become the established target's
lifetime. Cleanup retains its own journal capacity reservation.

Store tests qualify these controlled captures, atomic projection failures,
retirement ordering, independent revocation, subsequent generation-three
planning and authorized generation-two stream command admission. They perform
no physical movement. The native service now continues through the actual managed commit, carries
existing signed native resource confirmation across that exact scene commit and
adopts the real committed binding into an independent established runtime owner.
Generation-two and generation-three moves are verified through public signed
clients and actual PCMU delivery. The old operation's completed result is a
historical fact, not a claim that its target remains the current live device.


## Actual native movement and audience revision custody

The final continuation approves `DeviceReplacement` control with the exact
sealed managed plan. Candidate receive pairs are retargeted only from already
authorized source pairs. The candidate supplies its own current named receive
permission and both physical peers retain independent recipient ceilings.
Authority to own a route never supplies `stream.subscribe` implicitly. Core
rechecks actual private duplex stamps and configured confirmation at publication.

The runtime captures the actual managed participant observation and new target
readiness before any metadata await. It retains that capture through retries.
The established entry adopts the actual Core binding incarnation, selected media
profile and resource activation; later generations cannot construct an ordinary
initial binding. Old source retirement, controller cancellation, credential
revocation and preparation deadlines cannot lend or remove the new target's
independent packet authority. Current target loss ends only that exact route.
Actual native absence settles its own lifetime and journal retirement even when
its execution or target credentials have been revoked. Lifecycle inserts include
the original operation correlation for its owner rather than rewriting immutable
journal metadata afterward.

Schema 58 retains immutable `native_replacement_audiences` captures for actual
activation and movement commits. Each capture binds the original root, effect,
runtime and exact connection generations to before/after stream revisions.
The complete scene read issues the snapshot; the actual Core outcome supplies
observed revisions. Unknown/missing/extra controllers and a non-successor revision
cannot be recorded as this audience effect. These plain metadata objects retain
no authority reader and authorize no future IO.

Capture insertion and matching current stream projections share the phase's
transaction. A failed insert rolls back phase confirmation and revision changes.
Retries require the same captured audience and never repeat Core work. A retired
incarnation or a later independent revision is never revived or overwritten.
The retained peer's control revision advances together with the actual audience
change, and the candidate's private activation revision becomes the baseline for
its final movement. Public stream commands remain usable after both generation
two and generation three; Observer media remains receive-only.

The native regression scope includes preparation cancellation and separate
controller, endpoint and target-key loss; rejected signed parameters; actual
movement with old-source retirement; independent established audio; late
cancellation; later endpoint movement; current target EOF/credential/endpoint
loss; and actual durable retirement while the retained peer stays live.
Telephone callback/configured answer-DTMF, the complete unknown/late-event/restart
matrix, durable retargeted-selector replay under later churn and the additional
approved hosting/browser/provider profiles remain required qualification. G6-041
remains in progress.


A known commit also survives delayed projection. The physical fence already
checked the original revision; recording that known result advances the current
monotonic topology instead of discarding later metadata or rerunning the effect.
If target authorization or native confirmation is lost before established-owner
adoption, the owner immediately drains only the new route and records its exact
retirement. It does not leave an unconfirmed connection waiting for a normal
established-lifetime monitor, or restore the retired source. A blocked projection
followed by target-key revocation is an explicit local failure/recovery fixture.

The SIP adapter's replacement seam consumes Core's exact reserved candidate ID
through `prepare_device_replacement_connection`. Its protected context binds the
actual preparation incarnation, verified SIP Endpoint, target principal and
adapter instance. Allocation constructs a dormant outbound route without a
Participant binding, SIP packet or per-route task. Ordinary origination cannot
consume this context. Activation uses the existing retained SIP attempt and
stable Call-ID receipt, so dropping a waiter cannot authorize another call.
After dormant allocation, `prepared_call_reference` exposes the exact planned
Call-ID so the host can record correlation before its single submission begins.
This reference is neither an activation receipt nor evidence of a sent or
answered INVITE. A later successful activation receipt must carry that same ID.
Cancellation before activation is local; cancellation after submission uses that
same route's actual CANCEL or BYE. Resource authorization expiry must not be
reported as physical route absence while teardown is still required.

Only the exact live SIP route translator can mint replacement answer/DTMF signal
sources. `SipDeviceReplacementContext` itself is not a signal source. A bounded
queue carries opaque Core events captured synchronously from actual dialog
observations. Recording an event still requires fresh current signal authority;
old epochs, retired routes and caller-configured contexts cannot supply it.
For DTMF confirmation, capture pins whether the candidate was already answered
and had actual private duplex IO. Delayed observation cannot turn pre-answer or
pre-media digits into a later confirmation. Current readiness and authority are
also checked at observe.
A SIP final answer alone does not establish replacement media readiness.

The protected context installs one held-policy controller for the exact reserved
binding and negotiated audio port. Private input goes through current Core
ingress and decoder probes. Guarded output keeps its packet custody through the
actual exact-session RTP encode/send; queue admission does not assert output
success. Output alone cannot establish duplex readiness. Cleanup cancels and
joins the retained bridge even while idle or backpressured. Raw replacement
stream access and legacy codec renegotiation are refused. Local loopback evidence
in `docs/evidence/gate-6/sip-guarded-media.json` covers one actual SIP Core move,
private preparation, retained duplex after old-device loss and cancellation/EOF.

Core assembles the configured 1–16-key confirmation from attributed callbacks.
`PendingDigits` records a valid prefix; `AlreadyObserved` records an opaque
callback whose sequence or a later sequence has already been observed. These are
internal SPI observations, not new HTTP response kinds. Both still require
current callback authority. A repeated actual key has its own observation;
replaying the same captured event cannot supply another key. A sequence gap or
out-of-order observation cannot join digits across lost callbacks. Invalid
batches discard the prefix, and unmatched keys retain only a suffix that is a
prefix of the configured sequence.

The prefix and completed confirmation retain the actual duplex incarnation,
including controller revision, credential, ports, consumer and subscription
fences. A delayed callback from an older incarnation is refused. A media change
before movement invalidates the completed confirmation and requires a fresh
sequence; the final physical commit checks the captured confirmation stamp with
the target state already locked. Captures never wait on controller mutation state
while holding connection fences in reverse commit order. Transient metadata
contention cannot lend readiness or permission.

These adapter hooks do not install the public Connect telephone workflow.
Independent production target authority and established lifetime, durable
callback submission/receipt custody and the complete telephone recovery matrix
remain required before telephone replacement can be exposed or G6-041 can close.

## Telephone allocation and submission custody

The Store's typed `TelephoneReplacementRead` is a short current-authority
interval, not a signed native target or a readiness proof. Host configuration
selects a route credential and the already frozen telephone policy. The
credential must currently belong to the verified Endpoint's independent route
owner, permit `sip_dial`, and permit the intended endpoint owner and retained
recipient. The original operator and source/retained physical credentials keep
their separate checks. A wrong route identity, changed policy, revoked Endpoint,
expired credential or insufficient recipient ceiling refuses the phase.

The merged interval includes the route owner's identity fence and expires within
five seconds or an earlier backing expiry. It must be dropped after the bounded
control/IO action, before idle answer waits or metadata recording. A writer cannot
pass the held interval, and a new reader cannot nest behind an already held
writer. The route principal and logical target member remain separate facts.

`begin_telephone_replacement_allocation` records one possible dormant allocation
before invoking Core. The trusted capture supplies the actual reserved Core
binding observation and that route's planned Call-ID. The Store compares the
captured binding with the immutable expectation; the runtime must obtain these
fields from Core and the SIP context, not manufacture them from the plan.
Recording the exact capture is idempotent and must finish before
`begin_telephone_replacement_submission` can run. Submission stores that same
Call-ID as correlation before any possible call. These typed telephone APIs use
the existing original-native-call reservation and phase ledger; they do not
construct a `PeerAuthorityGuard` or a native target descriptor.

An opaque attempt owns metadata only, so the runtime can retain it while dropping
its authority interval. Lost results can mark the exact phase unknown. Repeating
that observation is idempotent; neither another phase attempt nor generic worker
custody is granted. A later exact receipt can record historical knowledge after
cancellation or revocation, without restoring the root state or granting future
IO. A different Call-ID conflicts with the immutable recorded capture. Restart
retains the original phase attempts and correlation; an old runtime's capture is
not accepted by its successor.

This Store custody layer does not itself send SIP, establish media, observe DTMF
or install the public telephone workflow. Connecting actual submission to a
bounded current grant, releasing that grant before answer waits, adopting the
independent established target and completing the full telephone failure/restart
matrix remain required work.

## Current permission at actual INVITE writes

The SIP `SipOriginateContext` and outbound INVITE builder retain an optional
`ClientRequestWriteAuthority` policy. It reaches the existing transaction egress
boundary before publication, including each retransmission, candidate and
401/407 or session-timer retry. The callback receives the immutable actual request,
transaction and prepared route plus the exact send future. A refusal never falls
back to an unprotected INVITE. The policy owns metadata between writes and must
acquire current permission anew, retaining the interval through bounded IO and
releasing it before retry timers or answer waits. Generated non-2xx ACKs complete
the existing transaction on its retained route; they do not pass through the
application's INVITE submission policy or count as a new call attempt. A completed
policy refusal before the exact IO future is polled is zero-wire. A lost observer
or an error after IO begins remains uncertain and retains cleanup ownership.
A verified late 487 must be ACKed on the exact retained transaction route before
terminal delivery. Its exact-attempt delivery fact can settle cleanup independently
of the immutable earlier transport-failure outcome; a 200 to CANCEL alone cannot.

`telephone_replacement_submission_write_read` provides a distinct Store interval
for an exact already begun telephone submission attempt. It rechecks current
source, retained route, operator, independent route credential, Endpoint and
policy, then requires the exact durable start attempt, effect, descriptor and
Call-ID correlation in submitting state. Allocation alone, a foreign correlation,
recorded submission, uncertainty, cancellation or a successor runtime cannot
lend this interval. The host must also match the actual request Call-ID and
provisioned destination before polling IO. This API does not install the host
bridge or authorize an idle answer wait.

## Canonical host telephone submission

`Service::install_telephone_replacement_adapter` registers the server-owned SIP
adapter on the canonical Core before the first physical native intake. The host
supplies verified route credentials, frozen callback policy and SIP originate
options; duplicate installation and installation after intake are refused. The
server owns network construction and coordinator shutdown. The canonical
server's optional `telephone` TOML section supplies this hook before HTTP/native
intake. It is not a public route-creation command.

A public `connection.replace` still names only a verified Endpoint. Its secret
reference selects the installed policy. The route account may differ from the
logical Participant identity: the trusted Store resolver binds that current
account to the verified logical membership and Endpoint. Original operator,
source and retained grants remain independent; native peer credentials are not
copied into the telephone account.

Actual Core allocation produces the binding capture and planned Call-ID. Both
must be durable before a once-only start attempt records that same correlation.
Core start retains metadata across ringing; each pre/post control check gets a
fresh short Store interval and releases it before waiting. The start control
reader can wait for the ordinary SQL mutex outside Core registry locks. The
physical write reader remains immediate and revalidates the complete scope set
plus exact durable submission under one SQL mutex interval. Bounded retries reacquire
metadata reads only; Core allocation, start and physical sends are never retried
by this host after an uncertain result. SIP transaction retransmissions retain
the original Call-ID and independently reauthorize each physical write.

The actual INVITE must match the recorded Call-ID and parsed destination URI,
including its parameters and headers. URI formatting can escape E.164's leading
plus; textual spelling alone is not an identity comparison. Permission is held
through the supplied exact IO future, bounded by both the current interval and
the monotonic preparation deadline. Every IO poll rechecks that deadline. The
interval is released before idle answer waits.
Current route or Endpoint loss, public cancellation and the preparation deadline
cancel the same candidate. The old native route stays active. Unknown submission
remains unknown; replay cannot allocate or dial again. A definite preparation
refusal can settle only the exact unstarted reservation. Once Core preparation
has begun, it cannot be reclassified as an unstarted refusal.

The preparation policy applies only until movement commits. Established
telephone ownership uses the separate lifetime described below. Normal server
composition and the complete failure/restart matrix require their own behavior
evidence before G6-041 closes.

## Telephone media resource provenance

Schema 59 records one telephone resource attempt after the exact SIP activation
receipt is durable. `TelephoneReplacementMediaOrigin` contains that captured
provenance without a permission reader. Its current read combines the original
source/retained/operator scope with the independent route credential, logical
membership, verified Endpoint, frozen policy and exact confirmed Call-ID. It
uses the logical member's accepted Session media; it does not fabricate a signed
native target or inherit the old native offer's selected codec.

The SIP context reports the actual bound audio codec and payload type as
metadata. The host applies current Endpoint/Session acceptance before beginning
resources, then initializes the Core-reserved guarded controller, prepares one
audio stream and mutes it. Its sealed Core resource observation must match the
original binding and generation. The captured stream handles, generation,
revision, codec shape and payload type are recorded once. Metadata retries keep
the same capture; ambiguous preparation cannot start another controller.

Resource preparation grants no programme input/output, private prompt,
confirmation, Participant admission or public ready/completed status. Those
policy hooks remain default denied until their own current and journaled phases
are implemented. The current resource policy grants only its exact initialization,
first stream preparation and initial mute. The temporary Store interval is dropped
before idle waits. Route loss cancels the same candidate and preserves confirmed
resource facts and the Call-ID. Abandoning the host continuation leaves the root
unknown; it asserts no remote absence or failed call and permits no redial.

Resource provenance alone does not authorize the private preparation or final
movement phases below. Each phase has its own actual observation and current
authorization boundary.

## Canonical telephone private preparation

Schema 60 adds `telephone_replacement_phases` for once-only attachment, muted
activation and private readiness. Each phase begins against the exact known
telephone resources, runtime epoch and current complete native scene. Its effect
is recorded before Core runs. Actual Core output is retained before recording
metadata; recording retries cannot run Core again. Unknown outcomes retain the
reservation, and late observations add knowledge without restoring a cancelled
or unknown operation. Snapshot digests cover these phase rows.

The SIP candidate remains a telephone route with independent current account
and recipient permissions. It does not acquire a native peer identity. Real
native controllers use their existing current scene views. Activation records
actual successor controller revisions atomically with its effect. Route pairs
require the receiving credential's named subscription permission and the
applicable physical recipient ceilings.

Private decoder input and local silence require the actual Core preparation
operation and a fresh current read of the confirmed private activation. No
reader remains held while waiting for RTP or digits. Ordinary publication and
receive remain refused for this provisional telephone owner. Decoded private
input never enters the main audience. Successful RTP output alone is not duplex
readiness.

Only opaque events captured by the installed SIP adapter may apply attributed
answer and DTMF observations. Current authority is checked separately when each
event is applied. Capture-time media eligibility, exact route/generation,
sequence and replay checks remain Core responsibilities. Early or incorrect
digits and retransmitted INFO cannot satisfy the configured confirmation.
Private readiness requires actual decoded input, successful output and the
configured confirmation; recording readiness leaves the public operation in
preparation until the separate final movement commits.

## Telephone movement and established lifetime

Schema 61 retains the once-only telephone commit attempt, exact observed Core
movement and complete audience revisions. The actual Core compound operation
retargets the existing logical Participant programme to the verified candidate,
then retires only the selected source connection. A separately authenticated
route account may serve that same logical member only through the sealed
replacement binding. Exact connection and stream selectors retain physical
owner and controller checks. Current packet policy remains mandatory.

Known Core output is retained before metadata writes. Recording and projection
retry metadata only; neither may repeat the switch or dial. Projection creates
the actual SIP connection and its independent telephone lifetime, preserves the
original operation correlation and advances the current stream revisions
atomically. An uncertain outcome retains its single attempt; a late known
capture may add facts without another physical operation. Historical captures
cannot overwrite a later generation or resurrect a retired connection.

The established owner requires the current configured route credential,
verified Endpoint and Device, logical membership epoch, present Session role,
accepted negotiated codec and exact current connection generation. It retains
no native source origin or original operator grant. Native/telephone packet
pairs acquire one complete current interval, require each receiver's named
subscription permission and enforce both physical recipient ceilings. An
Observer role immediately blocks telephone publication.

Established in-dialog INVITE writes require their own short current interval,
exact dialog correlation and configured destination authority. They do not
reuse the preparation cancellation token or deadline. Every physical IO poll
checks the interval and actual SIP liveness. Metadata contention is retried
before adoption; it is not interpreted as target revocation.

After commit, old source credential loss, original operator revocation, late
cancellation and preparation expiry cannot end the new connection. Its own
route loss or physical disconnect ends only that connection. Durable retirement
requires exact actual adapter absence at the captured generation. The completed
operation and retained peer remain intact. Canonical configuration owns the
local SIP listener, explicit RTP range, independent account references and
optional trunk/registration. It installs before either intake, drains after
Core shutdown and retains cleanup ownership on failed startup or server drop.
Mixed selector/role/private-context churn and the full failure/restart matrix
remain open qualification work.


## Telephone restart and receipt recovery

Graceful restart during private preparation preserves actual known resources,
phase captures and once-only attempts while the incomplete root remains unknown.
A completed movement keeps its historical receipt/result after restart; it does
not assert a current physical connection. Current connection observations are
retired or interrupted at a successor generation with readiness/runtime cleared.
When server-owner drop retires authority before cleanup can be recorded, the
exclusive successor uses the existing connection-epoch fence. The old telephone
lifetime remains historical, with no fabricated confirmed cleanup or new owner.

Actual late old-dialog INFO and RTP cannot become attributed confirmation,
restore a preparation or publish programme in the new runtime. Matching public
get/replay retains record, result, revision, known captures and original attempt
IDs without another INVITE. A lost command reply uses that same logical operation
ID even when an actual callback was already submitted. No-answer still resolves
to unknown with the old speaking route intact. These local graceful/drop and
discarded-response cases do not replace the process-kill/provider matrix in G6-047.

## Retained participant selection across endpoint movement

Participant selection retains tenant, Conversation, logical identity,
Participant and membership epoch. It does not pin the replaced endpoint's
physical account. Materialization validates the actual current source principal,
connection, endpoint and receiver permissions, including independent telephone
route authority and both recipient ceilings. Explicit physical connection and
stream selections keep their physical scope.

A named stream exclusion suppresses that source even while Participant intent
is retained. Named reselection clears the guarded media exclusion and restores
actual delivery without dropping the retained logical intent. A signed native
control uses its exact original attempt and complete authority interval;
telephone source lookup cannot lend a different command or route permission.

An established telephone source is selected through its own current SIP
lifetime and actual compound-move capture. It cannot be replaced by the retired
native admission that preceded it. Native target allocation uses the new
device's exact signed command and includes the independent source route owner in
the original merged authority interval. A queued route writer cannot require a
nested reader or a second physical attempt.

A telephone-to-native move retains the original logical member, epoch, Session
and retained connection, advances the replacement generation, and requires actual
native duplex readiness. Until commit, the telephone remains the speaking route.
After commit, current native credentials govern the new route. The old telephone
cleanup flag is settled only after actual SIP absence; historical operation
results do not restore that route or permit redial.
