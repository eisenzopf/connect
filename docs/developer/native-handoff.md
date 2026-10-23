# Native handoff implementation

G6-040 is implemented and qualified in the recorded local macOS arm64/native
QUIC/PCMU scope. Public `session.handoff` admits durable intention. The
native coordinator executes both the known private-pair human profile and
independently prepared human/AI targets through actual managed Core role/media
controls. An incoming Observer remains isolated until the same atomic commit
promotes its participant role and actual audio speaking fence. Its Store/Core and actual native failure/recovery evidence is recorded in
`docs/evidence/gate-6/native-handoff-complete.json`. Other transport, provider
context, remote/performance and acceptance qualification remain in their assigned tasks.

The local native QUIC fixture verifies observe and leave, both audio directions,
strict observer non-publication, continued audio beyond the original bounded
reader interval, observer EOF recovery and stable remote/main Session IDs. It
checks both Session and Conversation role scope through public clients, including
the unchanged Session-scope defaults and the single Conversation default revision.
Copied-policy and unsupported-preparation refusals retain a public problem and
no native effect, while the original private duplex and silent remote hold remain
usable. The four handoff scenarios and two refusal cases use 27 strict audio
probes with zero audio allowed outside each specified audience.
It uses provider-free audio endpoints. It does not qualify live models, opaque provider memory, other media or
remote/browser/Linux profiles.

`Store::native_handoff_origin` accepts an already admitted exact
`TransitionCommand` under a current `session.handoff` credential. Its immutable
`NativeHandoffOrigin` pins the operation record, complete original digest/request,
original expiry, current credential ID and installation runtime. It retains only
a weak kernel reference and no owned authority reader, transaction, media grant
or execution token. Status acceptance cannot manufacture incoming readiness.

The target can now prepare its own physical Connection independently of the
caller's handoff operation. `CanonicalClient::prepare_handoff_native_audio`
consumes a fresh native client with its own current named `session.handoff` and
`connection.offer` permissions. Supply the native QUIC offer with exactly
`"handoff_preparation":{"profile":"connect-handoff-preparation-v1"}`, one duplex
PCMU audio stream and the normal `CommandScope` (original Conversation/Session,
fresh Connection ID, logical operation ID, expected revision zero). The returned
`PreparedHandoffNativeAudio` owns audio IO and exposes `policy_record_id()` for
the later caller's `policy_ref`. No caller ID, transferable grant or ready flag
is accepted in the preparation marker. Mixed private/preparation profiles fail
before allocation. This is a closed, muted preparation, including for Observer;
it does not change the Session role or claim actual duplex readiness. Its
completed result contains the same profile and `prepared_muted: true`.
Preparation journal recipients stay target-only, including after handoff.
Current Connection metadata may include only the exact captured handoff members
after a confirmed attributed commit. This preparation does not initialize model
memory. The server separately owns a bounded readiness worker: it installs a
scene-issued decoder-only capability, discards decoded input without publishing
samples, and generates neutral local silence through the exact negotiated codec
and guarded native writer. Current own physical credentials authorize each IO
attempt; no reader is retained while idle. EOF/Session/server retirement stops
the worker. Enqueuing output is not proof: only actual decoder and successful
writer completion mint the sealed duplex observation used by handoff.

The native source requires an opaque identifier in `payload_extensions.policy_ref`.
An optional `payload_extensions.consultation_record_id` names an existing known
applied private context. In the canonical wire payload these fields remain
preserved extension fields. HTTP `TransitionCommand` uses its explicit
`payload_extensions` map. Both references remain in the original business digest;
changing them under the same logical operation ID cannot acquire a source.
A policy reference alone is not consent or approval. The native reader resolves
it to the exact incoming signed native offer operation record ID. The credential
bound to that physical offer must independently and currently permit
`session.handoff`, `connection.offer` and `stream.subscribe`, with current recipient
ceilings for the proposed routes. This is explicit target preauthorization; it
does not claim a new human consent gesture. Default collaboration credentials
do not include `session.handoff`. The caller must hold its own current named
handoff grant; it cannot lend that grant to the incoming peer. A copied offer
reference or model request alone cannot authorize the target.

`Store::native_handoff_context` rechecks the original owner's current named grant,
request, topology, original membership epochs, captured generations and current
resource assertions. Cancellation, a generic worker's execution lease, another
installation kernel or a predecessor runtime cannot supply native source custody.
Before any execution attempt, a fresh same-owner credential may acquire a new
source for the same original intention; it cannot revive a cached old source or
renew the original operation expiry.

A referenced private context must belong to the same Conversation/main Session
and exact retained remote. Source and target Participants must be the original
two private members. Omitted physical assertions select their exact original
incarnations. An explicit separate prepared source/target may belong to the same
original member epoch, with its own generation frozen at handoff admission. The
selected source, incoming and retained remote must have current native lifetimes
and Session appearances. This allows a separate safe public AI instance without
publishing the original private model memory. A returned context is unsuitable.
The lookup does not borrow the consultation's completed/revoked execution grant.

`Store::try_native_handoff_read` takes that origin and the complete current native
Session scene. It refuses missing, duplicate, foreign, stale and retired source
bindings. It rechecks current signed physical credentials, verified Endpoints,
Devices, signing keys, membership epochs and bilateral recipient ceilings. The
owned reader expires within five seconds or the earliest source/operation expiry,
fences relevant writers, and resolves the complete footprint again after fence
acquisition. `NativeHandoffRead::into_selection` drops the readers; the resulting
selection is metadata only and must reacquire current authority before reuse.

The proposed route map permits incoming and retained remote to hear each other.
With `observe`, every current outgoing Participant Connection may receive those
two sources under its own `session.observation` and `stream.subscribe` grants;
none publishes back into the new pair. With `leave`, those outgoing Connections
receive no handoff source. Unrelated private groups and held remote legs cannot
be taken by the handoff. Unaffected outside routes retain current bilateral/private
scope restrictions. These are planning permissions, not actual route changes.

A private AI Connection cannot become the incoming public speaker. Select a
separate safe public native/model instance. An unselected original private
candidate is quarantined from all handoff media, including subscription grants.
Current physical closure facts remain required even when its private provider
is unavailable. Private history and ordinary private media restrictions remain
intact; role changes never share that history. These controlled Store tests do
not claim live model memory qualification or decoded incoming readiness.

`Store::prepare_native_handoff` drops planning readers, obtains current writer
fences, and stores one exact selected-origin snapshot and plan digest. The plan
pins the scene/audience and participant revisions, selected runtime ports,
current source role, outgoing disposition, complete stream revision map and
exact required route/group sets. Preparation reserves one native effect and
worker token. Generic worker claims, effects, cleanup and reconciliation cannot
adopt this native attempt or manufacture a result.

`native_handoff_attempt_read` reacquires current caller/target/physical authority.
`begin_native_handoff` consumes a prepared lease plus its fresh exact reader and
atomically marks the single possibly submitted attempt. Duplicate begin, stale
roles/revisions/generations, cancellation and changed target grants refuse before
execution. An error after begin records uncertainty and keeps the reservation;
it never authorizes another Core call. Restart fails an unsubmitted preparation,
keeps a possibly submitted attempt unknown, and cannot borrow predecessor tokens.

The trusted native runtime may turn an actual paired Core media/participant
observation into `NativeHandoffApplied`. Attribution must match the original
operation, Session, selected Participants/epochs, retained/admitted ports, exact
role/disposition/retired set and next audience/participant/stream revisions.
`record_native_handoff_applied` records that captured fact after cancellation or
credential revocation without acquiring a live IO grant. It updates only matching
native incarnations at the captured previous stream revision and persists a
bounded attributed operation observation/causal ID. Unrelated scene ports and
private provider state are absent from the public observation. Re-recording is
idempotent. A captured fact survives restart even when current role projection
must remain unresolved.

`complete_native_handoff` projects an already recorded outcome in one transaction.
Session scope changes only the selected engagement. Conversation scope atomically
updates the source default to observer, the target default to the captured source
role and the selected Session; other Sessions retain their admitted roles.
Observe preserves the source presence as observer. Leave marks that presence
left and the captured outgoing Connections retiring; physical cleanup remains a
separate pending result. Conversation membership and historical access remain
intact. Result IDs, role/default events, Session topology and completion are
recorded once. Replays emit no additional events. Capacity failure rolls back the
role/result/event transaction and allows the same captured fact to be retried.
Newer topology, Conversation revision, membership epochs, physical loss or changed
selected generations remain untouched; completion returns pending rather than
overwriting them. A late cancel after captured application requires reconciliation
and cannot report cancellation before commit.

Native leave cleanup may be observed before role projection. Its exact original
native lifetime, completed offer and confirmed effect can identify the retired
outgoing tombstone after the normal one-generation advance and live-field clear.
That captured leave fact may complete without reviving the port. A missing
retirement marker, later generation, different runtime, lost observer or unhealthy
incoming/retained peer remains pending. Immutable offer, endpoint and native
origin bindings cannot be reassigned to make another port inherit this fact.

This is fresh schema 55. Existing incompatible local state needs the documented
backup/reset flow; there is no automatic historical migration or downgrade path.
The canonical signed/HTTP dispatcher queues an owned runtime job. It uses the
actual private audience handle, actual Core participant/audience revisions and
an opaque `ManagedAudioHandoffPlan`. The same exact plan is approved separately
by the participant and `Handoff` media-control leases under the bounded Store
read. The caller's current bearer membership is separate from each signed
physical peer's native binding. A controller needs no native media Connection.
Native confirmation follows the actual audience commit; only then are the
attributed result and current role projection recorded. Runtime retries retain
captured metadata and an opaque actual handle, not command readers or IO grants.

Current audio after capture follows a distinct handoff lifetime. Each peer keeps
its own current credential, Device/key, Endpoint, membership/presence and native
incarnation checks; pairs require the same captured group, exact permitted
routing and bilateral recipient/subscription permissions. Incoming target policy
and outgoing observation grants remain current. A lost observer cannot retire
an otherwise healthy retained peer. A pending un-attributed attempt denies media
without falsely asserting definitive physical retirement. Known pending role
projection uses the actual captured role until the durable role transaction
finishes. Native role reconciliation preserves the receive-only observer role.

A consumed private context cannot be restored or reused as a private media grant.
Its retained provenance and private history remain unchanged. The new exception
is purpose-bound audio for the captured handoff group; it grants no new private
history, messages, files, provider memory or other media access.

Authorized `operation.get` status and transition replies expose `record_id` as
the stable Store reference. The typed native SDK retains that optional field,
including the target's own offer reference used for `policy_ref`. The extension
is pinned in `gate6-operation-record-reference.patch` with a source-lock digest
and documented in the generated wire/HTTP references. An identifier is never a
grant or a new consent gesture.

`fail_native_handoff_preflight` records a bounded definitive refusal only for the
exact accepted owner command under a current named grant, with no worker, native
custody or effect. It records an attributable `problem`, a failed intention and
one operation event; replay changes nothing. It can report a changed target
policy without pretending that policy remains executable. It cannot turn a
prepared or possibly submitted attempt into a known failure. The coordinator
distinguishes missing actual decoded audio (`not-ready`) from a definitive
unsupported profile or policy refusal. Status and transition replies preserve
the durable optional `problem` through the typed SDK and HTTP surface.

Core now also exposes `plan_staged_prepared_audio_handoff` for an independently
prepared one-member closed target audience and an optional prior outgoing
audience. Both exact scene-issued contexts are consumed together; unselected
old private ports retain a closed quarantine. Actual decode and guarded local
output after preparation remain required. This engine extension grants no
permission, creates no provider and makes no claim about opaque model memory.
`prepare_staged_audio_endpoint` now captures only the actual one-member muted
admission of this scene, under separate current preparation policy. Its owned
capability keeps decode-only input separate from normal publication, including
for Observer. Dropping it invalidates captured decoded proof, old readiness plans and
queued local prompts. A cloned scene handle is metadata and cannot replace
current IO or current permission. Consuming it changes the mute gate only in the handoff
commit. A media-only unmanaged commit cannot promote an Observer; the separately
authorized managed participant commit updates both its logical role and audio
consumer fence together.

The public coordinator chooses this exact independent capability when the
incoming signed offer uses `connect-handoff-preparation-v1`, with an optional
original private audience. Store custody requires `activate_muted` to name only
that incoming runtime Connection; missing or outgoing/remote assertions refuse
before any effect. A private source cannot omit its original context reference.
The existing original-private-human profile remains available with that context.

The actual public native test qualifies human-to-AI and AI-to-human takeover,
including initially Observer targets of both kinds, observe with Session scope
and leave with Conversation scope. Four scenarios use 23 strict audio probes:
the original duplex stays usable through preparation, incoming/retained audio
flows only after the attributed commit, outgoing Observer never publishes, and
leave retires its physical port. Each completes one attempt and replays the same
result while retaining the original remote Connection and main Session.
This is native endpoint/role qualification, not a claim about opaque model memory.
A later distinct handoff in the same live Session consumes the actual current
post-handoff audience, selected by its committed source membership and audience
revision. Receiver pins alone do not identify that audience: they can include
quarantined ports. Core still checks the opaque scene/group/controller identity
under current independent authority. Store custody requires exact empty receive
programmes for prior members omitted from the new audience. Earlier Observers
retain no automatic access to the later pair; current Connection metadata follows
the latest applied receiver programme without republishing preparation events.
Replaying either completed operation observes its original result and cannot
restore the earlier audience.

An unrelated audience commit can invalidate a still-closed target's earlier IO
proof. Its bounded preparation owner remains available to obtain fresh actual
output under the same independently current physical grant. Old proof never
substitutes for current duplex IO, and a consumed/retired preparation fails the
writer's exact group/port checks. The native repeated-transfer fixture prepares
two targets before either transfer, then exercises human-to-AI-to-human in one
Session with the same remote incarnation and twelve strict audio probes.

A readiness-only planning miss keeps the accepted handoff in a bounded owned
wait. Each retry rechecks the original intent and independent current grants;
no reader or role fence remains held while waiting. Actual late target input
can complete that original request without a readiness replay. A competing
request for the occupied transition footprint is refused at admission before
ledger effects. The actual native test cancels an unready request through public
HTTP, revokes a separate controller credential while another intention waits,
and checks both original audio directions before recovery. A fresh same-owner
grant replays that same second intention without extending its expiry; real
input then completes one native attempt. Stale topology requests are refused.
Cancellation, stale topology, policy denial and uncertainty are not readiness retries. The complete Gate 6 qualification matrix remains in its assigned tasks. A model tool
request, declared ready row or generic receipt cannot stand in for actual decoded
duplex readiness, current policy or the actual role/media commit.
