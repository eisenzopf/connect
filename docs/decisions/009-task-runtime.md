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

First-invocation provider setup may precede the resource creation receipt. The
[Vapi chat-session bootstrap](../VAPI_TASK_BOOTSTRAP.md) saves a protected
creation intent and separate setup credential before dispatch. After validating
that credential and current task authority, its authenticator independently
reads the private provider resource and verifies the expected assistant,
resource mode and saved nonce. Only that verified observation may reach the
existing atomic binding/grant issuer. Headers alone cannot establish identity.
The setup file retains one external resource and original issuance intent;
uncertain creation does not repeat the POST or extend authority. The setup
credential is not an ordinary task grant or a model argument. Protected bootstrap
creates chat sessions and stages rvoip-owned calls. The local G4-019 binding
contract is verified by the audit below; operator installation and actual
provider/model qualification remain separate Gate 4 work.

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

## Historical implementation checkpoints · invocation, inbox and effect foundation

The paragraphs below retain the earlier implementation snapshots, including
what was open at each checkpoint. Current item status and evidence are in the
[numbered implementation plan](../connect-gate-4-implementation.html#tracking);
these historical paragraphs do not override its completion records.

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

G4-011 now implements source-backed recall in the store/service facade. It pages
the existing recipient conversation journal oldest first under history or
context-view authority, with a 32 KiB serialized result and explicit remaining
entry count. Oversized/missing bodies preserve a reference and advance safely.
Current session/artifact metadata replaces stale snapshots; deleted, expired or
unavailable artifacts expose no body or bytes. Existing message timestamps are
explicitly absent. Task/identity/installation/resource-bound cursors do not bypass
current authority. Public wire, SDK, MCP and actual-host recall proof remain open.

G4-012 is in progress. Shared admission now protects one terminal event and one
mutation receipt per nonterminal task under the existing quotas. Task expiry
must be recorded before releasing that room. Terminal transitions use their
own reservation in the same transaction; concurrent tasks cannot spend each
other's slots. Disk/SQL failure and lowered configuration limits remain explicit
errors without an acknowledged state change. Matching terminal-control receipts
remain inspectable under current owner/control scope and ACL; they cannot revive
effects or grants. Physical media dispatch and reviewed task-specific SMS
purpose are still required before G4-012 closes.

The schema-13 G4-012 checkpoint implements task-specific SMS policy. Contract revision 4
adds optional immutable `TaskScope.sms_enrollments` references: trusted source,
grant ID and exact reviewed purpose. Admission checks existing approved evidence
and conversation ownership. Current lease, expiry, active evidence, route match
and STOP gate SMS dispatch only; unavailable SMS does not deny native task recall
or unrelated communication. Empty scope denies task-origin SMS and serializes
identically to existing scopes, preserving old invocation digests. Schema 13 is
a semantic downgrade barrier; it changes no retained task/grant rows. The SMS component is qualified in `docs/evidence/gate-4/task-sms-policy.json`;
the previous capacity checkpoint remains schema-12 evidence. Integrated public
interface and host/provider acceptance remain open.


The current G4-012 media candidate adds schema 14. An end command retains its
own issuer, wire request and optional protected effect in `media_end_intents`.
A new command supersedes the current intent; a cached receipt cannot replace it.
Claims and final dispatch validate the exact generation and current credential,
conversation access and task/grant authority. A blocked old effect remains failed
even if a separately authorized command subsequently ends the same resource.

Physical start admission is recorded once per resource. Final admission runs
after asynchronous SIP preparation and immediately before the provider request
or prepared SIP commit. Repeating admission, including through another database
handle, cannot send another request. Admission is the possibly-sent boundary:
revocation after it cannot promise to erase a remote effect. SIP origination
observes cancellation in flight and attempts connection cleanup; failed cleanup
remains uncertain. Infrastructure errors roll back admission rather than being
recorded as authorization denials. Polling and reconciliation of already dispatched
voice work remain observations and do not borrow a revoked task's authority.

Upgrade preserves old rows and verified backups. Known ordinary queued voice
ends retain their issuer. Old task-end history is ambiguous and remains uncertain;
old SIP ends lack the requesting credential. Both need a new explicitly authorized
end command; migration never substitutes the dialer or a historical task grant.
The additive schema marker prevents an older binary bypassing these boundaries.
The candidate's targeted authority/recovery/migration and server regression checks
are in progress; this does not close G4-012 or integrated Gate 4 acceptance.

## Current core qualification · October 10, 2026

G4-007–G4-012 now pass their complete item audits within the recorded local
macOS arm64 scope. The current records qualify task persistence, current
invocation authority after database lock admission, durable reply assignment
and consumption, effect recovery without blind redispatch, bounded source
recall and current task-specific dispatch authority. Physical loopback SIP,
native Vapi voice fixtures and public task SMS delivery/cancellation checks
are included. They do not attest installed assistant hosts or live providers.

- [Current authority and expiry](../evidence/gate-4/task-authority-audit.json)
- [Reply assignment and restart recovery](../evidence/gate-4/reply-inbox-recovery.json)
- [Effects and uncertain external acceptance](../evidence/gate-4/task-effect-recovery-audit.json)
- [Bounded source recall](../evidence/gate-4/task-recall-audit.json)
- [Dispatch, SMS purpose and STOP/START](../evidence/gate-4/task-dispatch-audit.json)

The 36 implementation items, A4 criteria and T-A1–T-A8 remain the full gate
contract. Component closure does not replace provider bootstrap, installed
hosts, the complete agent/app journeys or final package qualification. At this
core checkpoint cancellation delegation was still absent; its additive design
and subsequent qualification appear below.

## Current public interfaces and runner qualification · October 10, 2026

G4-013–G4-016 now pass their item audits in the recorded local scope. The
[shared interface proof](../evidence/gate-4/public-interface-qualification.json)
qualifies native task control, seven SDK/API/CLI lanes and independent MCP
transport. The [runner audit](../evidence/gate-4/task-runner-audit.json) adds
actual app/provider uncertainty recovery to the earlier durable app boundary
proofs. Failed fixture attempts and unchanged limits remain explicit.

These records do not close G4-017's complete tools/authority decision, installed
hosts, provider bootstrap, full journeys or Gate 4 acceptance. Earlier snapshots
above retain their historical claims; use the numbered plan for current status.

## Explicit bound-task cancellation · additive permission

G4-017's permitted task-cancel tool uses a new explicit `TaskPermission::TaskCancel`
(serialized `task_cancel`). This follows the approved product plan's cancellation
requirement and the recommended default-off design. It is never inferred from
messaging, status, media-end permission or a backing operator credential. The
protected developer must include it in both the immutable task ceiling and the
invocation subset; the backing actor must currently permit `task_cancel`.

This adds a delegatable permission to the existing typed Cancel command. Old
commands, scopes without that entry, grant digests and receipts are unchanged.
Older SDKs reject records containing the new permission; developers using this
feature need the updated SDK. An older server rejects the new scope value before
mutation. No task-profile reinterpretation, automatic migration or widened
existing delegation occurs.

The MCP schema accepts a retained request ID and optional bounded reason. The
service supplies the authenticated task; model arguments cannot select an owner,
conversation or sibling task. Native task clients use the same transactional
admission. Cancellation persists the attributed terminal event/control receipt
and revokes all task grants atomically. An already dispatched effect retains its
observation/reconciliation requirement; cancellation cannot erase it.

The cancelled grant cannot inspect or replay its accepted control receipt. If a
response is lost, the protected owner inspects that original receipt/task under
current owner authority. No tool refreshes a cancelled grant or substitutes an
owner credential. Ending a media session still leaves the task/conversation
intact; cancelling a task still leaves its conversation intact.

Qualification of this additive permission is recorded in the current local
task-tool checkpoint below; the preceding sections retain earlier snapshots.

## Current local task-tool checkpoint

The [G4-017 item audit](../evidence/gate-4/task-tools-audit.json) now qualifies
the complete local tool interface, including explicit bound-task cancellation,
source-backed recall, bounded waiting and lost-response recovery.
[Qualification evidence](../evidence/gate-4/task-tools-qualification.json) retains
the fixed cancellation transaction failure, the corrected recall fixture and
one unresolved SDK/CLI timeout whose unchanged isolated retry passed.
No full T-A/A4 or installed-host acceptance is closed by this checkpoint.

The owner confirmed that agents should get on with delegated work without
per-action permission prompts. Task cancellation is available through explicit
developer configuration and needs no interactive approval at invocation. Task
and conversation boundaries remain separate; cancellation does not undo an
already dispatched effect or end a conversation.

The [Codex export component](../evidence/gate-4/codex-workflow-export.json)
preserves an existing narrow grant and packages the workflow skill. The
[actual runtime discovery check](../evidence/gate-4/codex-runtime-discovery.json)
now finds that enabled skill and exercises real MCP through an isolated installed
Codex runtime, including pending, restart and current-authority denial. The
[actual call-intent check](../evidence/gate-4/codex-call-intent.json) now verifies
full skill injection, original intent on disk before communication dispatch,
exact-session replay after interruption and current revocation through protected
HostSession grants. Its Responses peer is scripted and performs no inference.
G4-018, G4-021 and G4-023 remain open for real-model instruction following,
useful continuation and full host/installation qualification. Healthy-storage
process interruption does not attest power-loss or failed-storage behavior.
The [actual reply-workflow check](../evidence/gate-4/codex-reply-workflow.json)
adds two independent installed runtime sessions on one Connect server. After a
bound human reply and owned server restart, exact saved-session continuation
uses the attributed recalled decision in one follow-up. The unrelated task
stays waiting, conversation content stays isolated and another replay leaves
the entire Store unchanged. Scripted Responses programs qualify wiring and
recovery; real-model and full journey qualification remain open.

## Protected voice bootstrap composition

The voice path installs tools before rvoip creates its raw-audio call. A protected
owner explicitly selects an existing task and that task's accepted session. The
server owns the loopback MCP listener, private intent and lifecycle registration;
the connector independently verifies provider origin, and rvoip owns call
creation, audio bridging and paired termination. Native history permissions
bound context selection; configured provider tool permissions remain an explicit
subset of the same task. No per-action approval prompt is introduced.

A small opt-in rvoip admission hook is recorded in the source lock and tracked
patch. After request preparation and before REST send, Store atomically checks
current task, source, route and expiry and admits the physical call once. Its
resource key is namespaced separately from ordinary voice-call IDs, within the
existing physical-call journal. This requires no schema migration. Staging and
possibly-sent admission are conservative: uncertainty is retained rather than
creating another call. Shutdown persists media unbindings even when an MCP
listener reports a drain failure.

Turnkey operator installation, restart reattachment, aggregate setup-listener
limits and actual provider/model behavior
remain separate unfinished work. The bootstrap guide and numbered plan identify
those limits. The individual component records preserve their scope; G4-019
closure comes from the subsequent binding audit. G4-020's local context/tool
runtime is qualified by the subsequent continuation audit; actual model and
complete journey qualification remain open.

### Exact-resource recovery

Protected composition can reconcile an operator-selected provider resource with
the retained private setup intent. It uses the callback's exact-nonce verification
and atomic issuance path, preserving the original task, operations, expiry and
issuance request ID. It never POSTs a second resource, reconnects media, guesses
an ID from a caller or assistant, or returns a grant bearer. Model tool schemas
cannot invoke this recovery API.

The first verified resource ID is frozen before issuance. If current task or
credential authority changes during lookup, this observation can be retained
without issuing authority. Repeating recovery cannot substitute another resource
or replace a revoked grant. Resource observation and task permission remain
separate; a recovered call binding does not attest an active local audio bridge.

The [local recovery qualification](../evidence/gate-4/vapi-bootstrap-recovery.json)
records five new recovery cases and 21 affected integration checks with strict
lint and Rustdoc results. The subsequent
[G4-019 binding audit](../evidence/gate-4/vapi-binding-audit.json) verifies the whole
local item, including distinct concurrent first calls, with 34 checks and
unchanged frozen inputs. The subsequent
[G4-020 continuation audit](../evidence/gate-4/vapi-continuation-audit.json) passes
31 behavior checks against 93 unchanged frozen inputs. A slow provider effect
survives MCP disconnect; a new chat invocation after call end and server restart
uses the original task's retained voice decision and explicit human reply for
one follow-up. A successful question send remains pending until its answer;
bounded inbox metadata requires a permitted effect-status read for the outcome.
An ended call is not used to authorize later tools or recreate media. Actual
provider/model, installed-host and complete journey qualification remain open in
the numbered plan.
