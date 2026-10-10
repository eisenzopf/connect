# 009 · Durable task runtime and invocation authority

Status: Gate 4 implementation contract, owner-authorized October 9, 2026.
Implements T-01–T-07; qualification remains T-A1–T-A8 / A4-01–A4-14.
This decision freezes G4-002/G4-004 interfaces and G4-005 boundaries. It does
not claim the store, grant runtime, MCP endpoint or host journey is delivered.

## Ownership and compatibility

The persistent Conversation remains the home for sources, messages, artifacts
and session outcomes. A task delegates a bounded piece of work inside one
existing conversation. Neither task expiry nor session termination implicitly
closes that conversation. Connect owns task policy; rvoip/UCTP keep their
existing protocol, orchestration and media responsibilities.

`connect-protocol::tasks` defines strict transport-independent typed inputs,
records and transitions. The control extension is explicitly versioned as
`connect-task/1`, carried within `connect-control/1` as the additive `task`
operation with `task_profile` and `command`. G4-013 wires this extension through
the existing public service. Capability discovery must advertise task support
before a client uses it. An old server rejects the unknown operation without
an effect; old control commands and their declared semantics remain unchanged.
Unknown task profiles/fields/actions fail before mutation. An incompatible task
extension requires a new explicit task profile, not reinterpretation of v1.

The initial typed TaskRequest is the extension payload, not a second listener or
an upstream UCTP specification. Pending is an application result with a task
handle. Native media remains the qualified QUIC datagram / SIP / WebRTC path.

## Task lifecycle and authority ceiling

Task creation derives owner identity and creating credential from authenticated
state. Persist immutable conversation/owner/origin, unique ID, allowed
operations and concrete recipient IDs, expiry, lifecycle, revision, timestamps
and attributed terminal reason. Creation requires `task_create` and current
conversation membership. Each delegated operation/recipient must be within
current parent authority. No wildcard recipients/operations are allowed in the
task ceiling. Administrative actions, credential issuance, conversation create
and close are not implicitly delegatable. Existing immutable conversation
membership remains the access boundary; a task cannot add members.

Transitions are `ready → running → waiting → ready`, and `running → completed`.
Any nonterminal state may become cancelled, failed or expired. Terminal states
cannot reopen. Every transition uses expected revision plus stable request ID;
matching replay returns its current-authorized receipt. A new stale transition
returns `task-state-conflict`. A task's metadata is retained under existing
capacity/access rules. Expiry is checked at admission/dispatch even if no
background expiry sweep has run.

Backing-credential revocation, lost membership, expiry or task cancellation
prevents new effects and denies accepted-effect replay. Cancellation races
serialize with admission in one immediate SQLite transaction. If a remote
request was already dispatched, cancellation cannot pretend to undo it: record
and reconcile its outcome, while preventing another dispatch or dependent step.
Inspection requires current actor access; a retained receipt is not a grant.

## Invocation grants and provider binding

Grant issuance is a trusted developer/composition operation, not a model-facing
tool. Bind task, backing actor credential, permitted subset, expiry, adapter and
verified native/provider-chat/provider-call/host-session identifier. Adapter and
external IDs are bounded, server-validated correlation data. Native requests
must authenticate their task-scoped grant; provider requests must additionally
match the registered provider binding. Model-supplied task/cid/headers cannot
create a binding or widen it. Recheck all ceilings on every read, mutation,
continuation page, dispatch and replay.

The provider MCP connection authenticates a configured narrow credential before
interpreting `X-Call-Id`/`X-Chat-Id`. Look up an already verified call/chat binding
established through Connect's durable provider lifecycle. Refuse unknown,
ambiguous or mismatched headers. Discovery exposes only this principal's tool
ceiling; successful tool discovery is not operation authority. Vapi may make
several MCP sessions per call/chat, so an MCP transport session is never the
task identity. Call/chat observations and their configured actor binding must
be persisted before granting task effects.

Mint opaque secrets with a domain-separated HMAC of installation secret,
random persisted nonce, grant ID and bound authority digest. Persist only
verifier/hash plus non-secret metadata; never put bearer plaintext in receipts,
JSON task records, Debug output or logs. A matching currently authorized grant
issuance request can regenerate the same opaque secret after an uncertain
response; replay does not extend expiry. Grant and binding metadata each obey the configured
operation-receipt count ceiling; matching authorized retries remain available at
capacity. These are independent retained metadata limits, not permission to
exceed the shared mutation-receipt or journal limits. Store comparisons use existing
constant-time HMAC verification patterns. Secret wrappers have redacted Debug;
issuance/export requires explicit protected configuration. Credential/grant
rotation and task cancellation deny the old secret even if its hash matches.

Only the currently authorized task owner or trusted backing adapter may refresh
a grant. Refresh issues a new bounded secret within the unchanged task ceiling;
a model-facing tool and an expired grant cannot refresh themselves. Long-running
host adapters obtain refresh through protected composition, not through stored
bearer plaintext or expanded model authority.

## Inbox, reply routing and application effects

Task question effects persist request/reply-to bindings before dispatch. Native
replies use the typed `Reply` command with an explicit question reference,
conversation and stable logical request ID. The native question reference is
the question message ID from the recipient’s own inbox, uniquely mapped to an
accepted task Send effect. Unknown, undelivered or ambiguous mappings fail
before writing. This uses the sender’s current
`send` authority; it cannot supply a task ID, owner or grant. The store verifies
that the question was delivered to this authorized participant and derives its
task. Incoming message, reply binding, task inbox and lifecycle transition share
the admission transaction before ACK. A terminal task retains the incoming
message as unassigned evidence rather than reviving work. Provider adapters resolve only verified
session/task bindings or explicit reply references (e.g. a reviewed SMS task
reply code); sender matching alone cannot distinguish tasks. Unbound messages
stay in conversation history and an authorized unassigned view. Assignment is
an explicit current-authorized, idempotent operation, with source event,
assigning actor and task provenance. An assignment cannot overwrite an existing
verified question reference. Native reply receipts remain the sender’s ordinary
communication evidence under current send authority and conversation membership;
they do not reauthorize an owner’s task or replay its effects. Never auto-split/merge conversations.

Commit reply admission, inbox event and lifecycle transition before ACK. Enforce
unique event admission/consumption per task and single explicit assignment per
source event. Reordered replies cannot advance an already consumed step.
Checkpoint tokens bind installation, actor, task and journal position using
existing authenticated cursor primitives. A notification is only a wake hint.
Disconnect/restart catches up from the journal; task-scoped wait uses the same
bounded cancellation-safe service behavior as the current public inbox.

Effects have a key over task ID, step ID and logical mutation ID, plus canonical
intent digest. Prepare the record before dispatch and compare current authority
before receipt lookup. Same identity/digest reconciles; differing intent conflicts.
State transitions: prepared → dispatched/cancelled; dispatched →
succeeded/failed/uncertain; uncertain → succeeded/failed only from an
attributed reconciliation observation. A lease expiry never re-prepares a
possibly dispatched request. Known remote idempotency keys are preserved;
unknown external outcomes remain uncertain without automatic re-dispatch.

The reference runner claims bounded steps and checkpoints through public SDKs.
A local app business effect and its processed-inbox marker share the app's
transaction, independent of the Connect store. Nontransactional remote effects
need their own durable intent/idempotency/reconciliation. Crash points before
inbox commit, before dispatch, after acceptance, before business commit and
before ACK are all required qualification. Exactly-once universal external
delivery is not claimed.

## Recall and communication limits

Initial task context and each paged recall result have a 32 KiB text budget,
source IDs, timestamps, provenance, continuation and explicit omissions. Older
retained facts can be read within the bound conversation under current authority.
Artifact bytes are separately authorized/limited; no arbitrary server paths or
unbounded base64 tool payloads. A deleted/expired/unavailable source is explicit.
Provider observations and retrieved text are evidence, never grant instructions.

Frozen starting limits, to be measured rather than silently weakened:

| Area | Ceiling |
|---|---|
| Active / retained task records | 1,024 nonterminal / 10,000 total per installation |
| Concurrent reference-runner steps | 16; queue admission fails explicitly |
| Task lifetime | At most 24 hours and no later than parent credential expiry |
| Invocation grant lifetime | At most 300 seconds and within task/credential expiry |
| Step claim | 30 seconds; safe reclaim only before possible external dispatch |
| Unconsumed task inbox | 1,000 events/task; capacity failure is not ACK |
| Scope entries | 1–32 unique operations and 1–32 concrete recipients |
| Logical / task / source identifiers | 1–128 ASCII alphanumeric or `-_.:` |
| Task page | 1–100 items; cursor ≤1,024 bytes; 32 KiB text projection |
| Terminal reason | ≤512 bytes, no control characters |
| Vapi tool wait | ≤10 seconds; earlier deadline may return pending |
| MCP request | ≤512 KiB including framing; narrow file chunks use existing limits |

No scheduled preferences or automatic channel fallback is added. Task expiry
limits delegation, not conversation persistence. SMS additionally rechecks the
reviewed enrollment purpose, active grant, provider keyword suppression and
fresh authority-feed lease immediately before dispatch. STOP blocks SMS without
cancelling unrelated tasks; START cannot revive cancelled effects. HELP/STOP/
START acknowledgements remain provider-owned to avoid duplicates.

## Adapter and package boundary

Proposed thin `connect-mcp` workspace crate depends on protocol/service interfaces
and the official MCP Rust SDK, not SQLite. It supports Streamable HTTP with a
legacy initialization profile accepted by Vapi/Codex; exact pinned SDK version
and tested MCP negotiation are recorded when G4-016 builds it. Do not hand-roll
an incomplete JSON-RPC transport or claim newer optional MCP features without
qualification. MCP native task facilities, if present, do not replace Connect's
durable application task identity and authority.

Server composition owns listeners, verified provider credential/binding hooks,
request bounds and supervised drain. The service owns authority and operation
translation; store owns transactions. The selected host adapter owns its
session/task mapping and declared resume behavior. Reference runner and app use
only public SDK interfaces. MCP auth/response adapters remain small modules;
provider provisioning stays in provider integration code.

Reuse: existing journal/cursor/receipt/context/artifact/worker primitives first;
Thelve managed-MCP projection, invocation preparation and GET-only Vapi recovery
patterns; Parley worker/context/provider mapping; Bridgefu idempotency and
attachment-binding patterns. The owner explicitly authorized reuse of Rudeless
source in this thread on October 9, 2026. Extracted code must record file,
revision, source digest and attribution. This does not require importing the
full Thelve registry or permission product.

## Required behavior evidence

| Stable check | Concrete boundary |
|---|---|
| T-A1 | Two concurrent tasks in one conversation: grant, reply, effect and context isolation |
| T-A2 | Forged cid/header/wrong call, expired/revoked grant/credential and accepted replay |
| T-A3 | Transaction/dispatch/app-effect/ACK crash points; competing workers; one committed effect |
| T-A4 | Duplicate, reordered and ambiguous replies; explicit assignment history |
| T-A5 | Old source outside initial window; unauthorized/expired/deleted source and cursor contrast |
| T-A6 | Voice ends; server/runner restart; useful same-task decision recalled on approved channel |
| T-A7 | SMS queued then STOP; START only under valid reviewed eligibility; cancelled replay denied |
| T-A8 | Slow tool pending ≤10 seconds; disconnected host resumes through status/journal |

Typed contract fixtures also validate the native recipient reply contract and
its distinction from administrative task ownership.

Typed contract fixtures close schema/transition admission only. Each runtime
boundary still requires store/service/SDK/MCP/host behavior evidence. Final
qualification records source/package/host versions and preserves failures.

## Implementation checkpoint · invocation, inbox and effect foundation

G4-008 and G4-009 remain in progress. The store/service now implement protected
grant issuance and discovery, exact registered binding admission, current
credential/membership/ceiling revalidation, explicit reply assignment, atomic
consumption, native reply admission and authenticated inbox catch-up. Service
waits register before reading and recheck durable state after a wake. Inbox
effect events project metadata; separate authorized effect reads own outcomes.

G4-010 now produces the accepted Send boundary transactionally through the shared
ordinary operation engine. Native question/reply and agent-to-agent response
tests use that real boundary. Explicit `wait_for_reply` binds pending completion
to the specific answered question. Effects namespace underlying receipt IDs by
task/step/logical mutation, retain known errors, and derive pending/uncertain
provider observations from durable jobs. Provider messaging dispatch rechecks
the original task and grant even while the account credential remains valid.

Contract revision 3 adds CAS `Claim`/`ClaimGet`/`ReleaseClaim` commands under owner
scopes. Leases are bounded to 30 seconds and 16 active steps; matching replay
does not renew them. A durable nonprepared effect prevents reclamation. Model
grants cannot acquire worker authority. Actual child-process exit before and
after task commit is qualified separately with explicit test-support binaries.

G4-008/G4-009/G4-010 remain in progress. Media start/end task authority at physical
dispatch, integrated provider-origin binding, SDK runner/app transaction and ACK
qualification, recall, actual host/MCP installation and full acceptance remain
open. Component evidence is not gate or host qualification. Current checkpoint
results are recorded in the numbered implementation plan, without changing
earlier foundation evidence.
