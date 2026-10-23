# Original Session invitation sources

Schema90 captures the original inviter and selected target membership epochs in
the same savepoint as a successful general Session invitation. The original
medium, intent and capabilities offer are immutable. The source is bound to the
ordinary operation receipt by a deferred foreign key satisfied before commit.
An infrastructure failure rolls back the source, Session mutation, journal and
receipt together. Exact logical replay does not allocate another source.

Each `(Session, Participant, epoch)` has a current invitation slot. A later
invitation after departure replaces that slot, while retaining its original
source history. An earlier source cannot adopt the later invitation, even when
the membership epoch did not change. Removal/readmission also requires a fresh
source at the new epoch.

The protected Store API `session_invitation_sources(record_id)` enumerates at
most256 target descriptors from an already completed ordinary invitation receipt.
Descriptors retain no authority reader, carry no serialized grant and cannot
supply physical Device/key, response mutation, media or endpoint authority. They
can be reconstructed from durable state after restart, but old process/runtime
handles cannot be adopted by another instance. Task-scoped sources remain excluded
until their original task/grant policy is integrated under G6-055.

`try_session_invitation_read` immediately obtains fresh metadata authority or
returns contention. It verifies the current authenticated target, original
inviter/target epochs, exact latest pending slot, nonterminal Session/open
Conversation, original Session source access and current recipient ceilings.
The target needs `session_get`, `session.accept` or `session.reject`; a role label
supplies none of these. The returned reader fences supported Store authority
writers and exposes a bounded original offer. Drop it after actual metadata IO;
do not retain it while waiting for a human or a reply. Reacquire current authority
for each subsequent phase.

This reader is not physical transport proof or authority to accept an answer,
start a gateway call, negotiate a Connection, send a signal or read RTP. The installed
owned exchange separately binds the exact authenticated physical
Identity/Device/key and current credential, authorize actual writer custody, and
verifies a signed response against its exact scoped waiter. Current mutation
permission and the original invitation slot are checked again at commit.

## Correlated wire boundary

The approved [Connect profile](../../contracts/connect-uctp.md) requires stable
operation IDs on commands. Correlated observations of command families have no
invented command identity: incoming observation `in_reply_to` frames route only
to exact current scoped waiters and do not enter ordinary command execution.
The closed command exception is an operation-bearing native `session.cancel`
asserting this physical caller's original invitation; see below.

The native parameter request origin currently requires an existing confirmed
media Connection. It cannot stand in for an invitation sent before acceptance or
Connection readiness. The installed live-route path forwards `session.invite` and receives correlated
accept/reject observations under a bounded physical waiter. Committed decisions
relay to the original physical caller as described below. Bounded recipient cancellation
observations and original caller command references are described below. Current explicit operation-ID Session
commands continue through the ordinary service policy and journal. Metadata
availability is not a claim of invitation delivery, acceptance or media readiness.

## Owned decision commit

The protected `SessionInvitationDecisionClaim` contains an opaque original source
and a bounded accept/reject observation. Peer admission obtains the full current
credential/Device/key/source and Conversation mutation union in one admission. A
Binding proof, metadata read proof, ordinary Session command proof or changed
decision cannot substitute. `apply_session_invitation_decision` checks the physical
current gate after acquiring SQL custody and before commit. It does not call the
ordinary operation engine, construct a caller command, allocate a caller operation
ID or emit an ordinary operation correlation in the journal.

An immutable source/target decision and original Session result commit atomically
with the presence/capabilities, topology revision and shared Session journal.
Storage failure rolls all of them back; exact replay returns the original result
under current source/epoch/permission and physical authority. It cannot mutate a
later invitation slot. The live worker retains no reader while waiting, deduplicates
active source/target jobs, and drops other physical waiters when one authorized
decision commits. Target lookup and immutable template construction are protected
runtime metadata APIs; actual IO still requires the current exact peer read union.

The wire operation correlation on a forwarded request identifies the existing
server invitation source. A response echoes it solely to match that request. It
is not an ordinary command ID owned by the recipient. No Connection is created or
media enabled by metadata delivery or an accepted observation.

Owned decision commits also enforce the installed host's assurance rule for
`session.accept`/`session.reject` against the exact observed credential. An
identified response cannot bypass a required human consent. The observation
creates no synthetic blocked command or automatic credential upgrade. After
actual consent and explicit Device/key enrollment, logical invitation replay
can obtain a fresh waiter on the new credential; an old wire ID remains invalid.

## Original caller decision observations

A successful signed native invitation registers its actual request ID, logical
operation and exact physical peer as transient metadata. Identity, credential or
Device-ID equality cannot select another physical peer. The runtime records at
most one current correlation per source/physical peer, with bounded capacity and
expiry; no authority reader is retained. HTTP submissions create no native caller
binding. Verified activity renews current route metadata, without extending an
older captured handle.

After an owned accept/reject commits, the original caller can receive the signed
`session.accept`/`session.reject` observation with its own `in_reply_to`, CID/SID
and original caller operation ID. Original by/capabilities/reason values come
from the immutable decision. Each actual write uses a dedicated current caller
read union: original receipt permission and membership epoch, original/current
Session audience, recipient ceiling, target membership epoch and exact current
credential/Device/key. A metadata or Binding proof cannot replace this reader.
The ordinary invitation response holds its mutation fence through ACK delivery;
the later decision reader cannot enter that response batch.

A decision may commit after its caller disconnects. Logical invitation replay
on a newly authenticated physical peer registers the new request correlation
and can relay the stored decision without reinviting or repeating the mutation.
Transient wire correlations do not survive process restart; the durable source
and decision do. One notification attempt per caller request/target bounds
duplicate hints, including uncertain IO. The authoritative journal/snapshot
still governs application state. These observations do not grant media readiness.

## Cancellation observations

Schema91 freezes cancellation facts in the same transaction as the successful
ordinary `session.cancel`. Only original targets still pending in their latest
invitation slots receive facts; an already rejected target receives none. The
original actor Participant/epoch, reason code/text and cancellation time are
immutable. Storage failure rolls back cancellation and its hints together;
logical replay preserves the original facts.

Each actual forwarded invitation has its own final signed wire ID. A bounded
live invitation waiter retains that ID as metadata and can send a signed
`session.cancel` observation with `in_reply_to` set to that exact ID. Its
`operation_id` is the original server invitation source correlation. Two physical
peers sharing an Identity/credential/Device receive their own correlations.
A native or HTTP ordinary cancellation can cause these native observations.

Actual notification reacquires a dedicated current cancellation read proof:
exact credential/Device/key, original invitation source and current inviter and
target membership epochs, current source audience and recipient ceiling. It
validates the immutable actor and reason before the reliable write. No resource
reader survives idle waiting. A cancellation that wins a response/commit race
prevents acceptance; the source may still produce a best-effort cancellation
observation. One bounded notification attempt does not assert remote receipt.

This live path is bounded by the original invitation exchange lifetime (30s),
physical credential incarnation and current route lease. Restart, expired or
retired physical routes do not recover an old wire correlation. The durable
Session snapshot/journal supplies recovery. Native caller command references use the separate owned assertion below;
ordinary logical cancellation commands remain available.

## Original caller cancellation references

`CanonicalClient::cancel_session_invitation(original_request_id, request)` sends
an ordinary logical cancellation command with a signed `in_reply_to` assertion.
The server registers the verified invitation command before its ACK can reach
the caller. It checks the original CID/SID and exact physical PeerAdmission;
a same-Identity/credential/Device sibling cannot adopt it. A logical invitation
replay replaces that physical caller's reference with the fresh request ID.

The dispatch exception is limited to operation-bearing `session.cancel`.
Fresh signature/replay checks, current named cancellation permission, configured
assurance and all ordinary Store membership/source/recipient/revision checks
remain required. Human-consent step-up retains the original command and reference
while reacquiring current authority; no idle resource reader is retained.
Same-physical metadata can survive credential rotation; an old route never
supplies authority for the new command or for delivery.

References are process-local metadata with a60s bound and physical lifetime.
After reconnect or restart, replay the original logical invitation to obtain a
fresh reference, then retry the same logical cancellation. The immutable ordinary
receipt supplies recovery even if cancellation already committed. Changing only
the physical reference does not change logical intent. HTTP does not accept a
native physical reference; its ordinary logical cancellation uses the same
receipt transaction and current policy. Acceptance still closes the cancellation
boundary and requires `session.end`.
