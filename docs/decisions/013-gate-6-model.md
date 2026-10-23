# 013 — Gate 6 conversation model and lifecycle

Accepted implementation direction: 2026-10-15. Owner approved both Gate 6 plans.
G6-004–G6-009 specify this decision; runtime evidence is separate.

## Six nouns and their bindings

Conversation owns durable messages, Participants and Sessions. Identity is a
durable tenant-scoped entity. Participant is its conversation-scoped appearance;
kind is human/AI/system/external and role describes application responsibility.
Conversation membership, Session presence, role and effective authorization are
different dimensions. Device identifies a physical/software endpoint; an Endpoint
is a provisioned reachable binding with capability, verification and freshness.

Participant role is the Conversation default. A Session appearance snapshots its
role at admission and can carry an explicit Session-local override. Changing the
Conversation default does not silently rewrite other active Sessions. Handoff
normally transfers Conversation responsibility and the selected Session role;
an explicit Session-only mode is available. Neither role scope grants permission.

A Session contains live appearances of permitted Participants and Connections.
One Participant can use multiple Devices/Connections and concurrent Sessions.
A Connection carries negotiated Streams. Logical stream relationships can survive
replacement through explicit lineage; physical RTP and datagram bindings change.
IDs are stable bounded opaque identifiers; they are not source-of-authority fields.

## Lifecycles

- Conversation: explicit persistent creation is Connect's default; idle activity
  continues/resumes it. Only explicit authorized close ends it. An ephemeral
  policy may close after all Sessions end and the declared idle timer expires.
- Session: invitation is pending until acceptance and required connection/media
  readiness. A new Session starts once its admitted live participants are ready.
  Existing Session invitations add participants under explicit admission policy.
- Reconnect: losing the last Connection starts a default 30-second reconnect
  grace while no explicit end occurred. End after grace and no in-flight permitted
  negotiation. Explicit Session end terminates immediately and cancels grace.
- Membership leave/removal: revokes conversation access and increments membership
  epoch on re-admission. Session leave preserves conversation history access.
- Handoff: changes participant responsibility/media audience; Session and retained
  remote Connection remain. Device replace keeps Participant and changes its leg.
- Server restart: persists intent/history but does not resurrect network media.
  Interrupted or uncertain effects require truthful reconciliation.

The baseline's immediate last-connection end rule is replaced by the explicit
grace rule above. `session.update` topology changes use expected revision and
fenced transitions, never last-writer-wins bridge mutation.

## Ownership

Shared schemas, transitions and media primitives belong in UCTP/rvoip. Connect
owns durable application state, authority and orchestration. The app owns task
goals, prompts, scheduling, UI and business approval decisions. No new primary
Task, Approval, Handoff or Consultation noun is introduced.

See [canonical mapping](../../contracts/connect-uctp.md),
[audiences](../../contracts/connect-audiences.md),
[transitions](../../contracts/connect-transitions.md),
[operations](../../contracts/connect-operations.md),
[events](../../contracts/connect-events.md),
[media](../../contracts/connect-media-v2.md) and
[records](../../contracts/connect-records.md).

## Checked participant engine integration

The G6-019 engine primitive starts with
`Orchestrator::enable_participant_control` on an empty initiating Session.
`change_session_participant` accepts a typed `ParticipantMutation` with a logical
operation ID, expected Session revision and target Participant. Its policy resolves
the authenticated actor, target membership and endpoint from current authority;
payload-selected roles and identifiers do not establish permission. A policy lease
holds that authority stable for a synchronous commit and must not call back into
the Orchestrator while holding the same authority lock elsewhere.

Admission reserves both participant and connection capacity before adapter
signaling. Pending candidates count against the six-connection default together
with admitted routes. Signaling acceptance does not mark the Session active:
negotiated duplex stream readiness, the exact connection lifecycle generation,
current endpoint verification and configured endpoint confirmation are checked
before the mixer and Session projection commit together. The policy is acquired
again after asynchronous preparation. Dropping or cancelling preparation retires
only its candidate and releases its reservation.

The mixer bounds frame collection per tick and holds exact-generation delivery
leases across that tick. Explicit
departure removes only the target Participant's routes; Conversation membership,
other Participants and concurrent Sessions remain. Speaking permission is explicit
policy independent of the Session role. Session role changes preserve Conversation
defaults and other Sessions' admitted roles. Raw join/leave, takeover and bridge
helpers cannot take over a checked participant route. Terminal route loss preserves
membership and lets the owning Session apply its reconnect policy.

A successful result and its retained receipt describe a local engine transition.
Correlated joined/connection-added/left/removed/role-changed observations are
available through the bounded process-internal participant event stream. Matching
replay returns its original observation after checking current authority; it does
not accept a physical route again. Changed intent conflicts. Receipts and retired
connection IDs remain bounded without eviction that would permit reuse. These
engine receipts are process-local; Connect journal admission, durable operation
recovery and public audience-filtered events remain G6-035/G6-036/G6-038 work.

Local route retirement does not assert a confirmed provider hangup. Adapter cleanup
runs under the bounded lifecycle supervisor, and uncertain cleanup retains its
quarantine. Explicit Session end cancels candidates, stops its mixer and fences
later admission. The engine fixture qualification uses encoded/decoded PCMU audio;
remote transport, browser, video/screen/data and live provider qualification remain
separate tasks. Run
`CARGO_TARGET_DIR=target python3 scripts/check-gate6-rvoip.py --component orchestration`
for the scoped engine tests and recorded source/binary digests.

## Durable membership policy

G6-031 implements serialized invitations and membership changes with a current
tenant/credential/epoch check and revision-bound receipts. Original content
audiences remain immutable decisions; explicit history shares record their own
source references, target epochs and provenance. Removal fences current access
and logical presence; readmission preserves the Participant ID with a new epoch.
Role labels do not change permissions or Session role snapshots. The common
policy applies to replay, source projections, file reads and queued dispatch.

Model context is an audience intersection, including after asynchronous voice
preparation. A saved selection or opaque provider chat reference is not a grant.
The [membership implementation guide](../developer/membership.md) documents the
facades, current projection rules and the boundary between durable policy and
later canonical Session/event/record integration.
