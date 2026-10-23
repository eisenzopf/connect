# Initial telephone admission

G6-044 installs initial telephone agents on the same canonical Core and media
policy as signed native Sessions. A configured agent reacts to an ordinary
committed `session.invite` for its explicit Participant. The caller uses the
public client; the host selects only its configured verified telephone Endpoint
and its own credential. Each configured Identity has one unambiguous Endpoint.
The shared operation belongs to the caller; the reserved Connection belongs to
the invited Participant. Initial calls have their own custody and purpose.

The canonical server's `telephone.initial` list contains
`InitialTelephonePolicy` entries: `endpoint_id`, `endpoint_credential_id`,
`route_credential_id`, `route_reference`, `sip_authority`, and bounded
`timeout_seconds` (1–30). Profiles share the configured canonical SIP adapter.
The route account, tenant and Endpoint secret reference must match the selected
policy. The Endpoint credential must authorize its own connection offer and
Session acceptance; the route credential must authorize SIP dialing. Initial
profiles can be installed without replacement profiles. These are protected
server configuration references, not client routing assertions.

## Original operation and exact transport custody

Schema 65 binds the initial child to the caller's successful canonical invitation,
its original normalized request and an explicit invited Participant. The current
original member epochs and audience, verified telephone Endpoint/revision/Device,
E.164 address, own Endpoint credential and route account are fenced before
reservation. The child ID derives from invitation plus target. Changing its
Endpoint or selected policy conflicts with the existing intent.

Allocation and signaling have distinct once-only handles. The host first reserves
a dormant actual Core outbound Connection. `initial_participant_observation`
checks the exact pending Core ticket and adapter owner, captures the route's
planned SIP Call-ID, and marks the route managed before IO. Those actual IDs must
be recorded before a submitting effect is created. Identical metadata retries do
not allocate or signal again. Raw streams and raw renegotiation cannot bypass
the managed path.

The INVITE writer checks the exact recorded Call-ID, destination, current caller,
own Endpoint and route credentials, original audience and owned runtime. Its
reader covers a bounded write (at most two seconds), not the ringing lifetime.
The original preparation deadline still limits initial signaling.

Core's `activate_initial_participant_with_held` keeps the actual provisional
route and reserves its Session slot before activation. SIP activation retains its
Active-dialog and exact Bound-media guarantee. After ringing, final admission
uses a fresh Core participant revision through
`ActivatedInitialParticipant::commit_staged_participant_with_held`. Only the
expected revision can change: the operation, correlation, Participant, connection
and requested admission remain the same. The adapter reuses its activation
receipt; final admission does not send another INVITE. Drop/failure keeps cleanup
with the original preparation supervisor.

## Observed answer and guarded media

The adapter captures a sticky bounded answer fact from route creation. Ringing,
SDP and codec metadata cannot assert an answer. Observers share one successful
answer claim for the exact route incarnation; a terminal event cannot erase a
previously observed answer. Waiting is bounded and cancellable and performs no
signaling. Its opaque receipt identifies the actual Connection and wire Call-ID.
Current resource checks and historical answer attribution are separate.

Schema 66 stores the captured answer even when old command authority has since
been withdrawn. A savepoint separates that fact from Session acceptance. Under
current original authority, the Endpoint agent executes ordinary `session.accept`
for its **own** Participant using the shared mutation engine. The immutable
`acceptance_record_id` references its completed own-identity control receipt,
Session and original membership epoch. Another actor's receipt cannot satisfy
this binding. A failed acceptance rolls back that decision while retaining the
answer; neither an answer nor a control receipt proves physical media readiness.

The host then obtains fresh own Endpoint and route authority, prepares the
actual SIP controller with the canonical composite media policy, and commits the
checked Core Participant and Connection. Core requires the controller's exact
original binding and policy domain. A pure runtime observer checks the committed
preparation, guarded resource proof, admission receipt, actual controller revision
and bound stream. Only then does Store publish Connection readiness and complete
the initial operation. Original signaling readers do not span these writers.

Every publication and delivery obtains current authority. Ongoing media uses the
Endpoint's own credential and route account, current membership/appearance,
accepted codec and actual binding. It does not retain the initiating caller's
credential as a permanent media grant. Native/telephone pairs hold one merged
fence set and check both source and receiver, identity, epoch, physical generation
and stream incarnation. Unsupported private/handoff combinations fail closed.

The stock agent's receive intent is the original invitation audience, excluding
itself. The host projects each logical source only once an actual eligible Core
source exists. Core still checks current pair authority for selection and every
packet. The successful revision change is recorded as an immutable
`initial_telephone_receive_projections` fact, with original Participant/epoch,
before/after revisions and actual resource observation. Metadata retry is exact
and never triggers another Core write. A changed revision after an uncertain
result stops the call rather than applying a second mutation. Current Connection
stream revision and journal facts advance; the original completion remains a
historical observation. Later unrelated members are not silently added to this
fixed receive intent.

## Cleanup and verification scope

The bounded host owns and drains admitted jobs, refuses new work after closure,
and retains earlier unconfirmed drain results. Session end/cancel notifications
cancel the exact Session's jobs. A busy Store/fence probe pauses packet authority;
it is not evidence that a healthy call ended. Confirmed withdrawal ends the leg.

A notification refused during preparation or the authority checks before any
Core allocation owns no new physical resource. This includes terminal invitation
replay. Such a refusal does not mark the host's drain unconfirmed; it cannot
clear an earlier genuinely unconfirmed job. Errors after Core allocation retain
the conservative drain behavior. Dropping the final Service owner cancels hosted
jobs and the runtime, breaking a job's retained-runtime ownership cycle. Another
Service owner keeps the host alive. Cancellation itself is not cleanup evidence.

Only the original adapter's retained cleanup observation plus actual local
absence can settle physical cleanup. Store checks that proof twice under the
historical binding and records a separate stable compensation effect. Already
fenced retiring rows can become retired without inventing another generation.
Repeated settlement does not emit duplicate retirement or advance operation
revision again. An unobserved submission remains unknown after local absence;
cleanup cannot rewrite its historical outcome to failed. Captured answers remain
confirmed, and cleanup has its own effect. Pre-submission failures can retire only their dormant reservation.
After submission, unknown effects retain IDs and Store capacity; generic workers,
operator assertions and empty restarted adapters cannot assert readiness or
cleanup. Startup's runtime-epoch fencing prevents SQL-only media resurrection.

The local implementation is being qualified with isolated macOS arm64 fixtures:
a configured canonical listener, public signed caller, actual UDP SIP answer,
guarded PCMU/native duplex, replay and cleanup. Store tests separately exercise
synthetic trusted boundary observations; those do not substitute for real
transport proof. See `docs/evidence/gate-6/` for the exact recorded source and
verification scope.

G6-044 remains open for the broader SIP/Telnyx/Vapi/SMS provider matrix. Remaining
work includes early-answer/decline/terminal custody across failures, complete
Task/grant context, runtime route discovery and selected trunk/secret binding,
uncertain physical Core slot retention/reconciliation, process-kill recovery,
additional media/private/fanout/role profiles and the five retired public
telephone workflow migrations. Named remote/provider proofs require bounded
G6-093 configuration and designated endpoints. This local component does not
qualify Linux, browser, remote/NAT, live PSTN/provider, installed-package or
independent acceptance lanes. Publication is separate. The public wire commands
remain in the [canonical HTTP reference](canonical-http.md).
