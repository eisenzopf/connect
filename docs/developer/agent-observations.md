# Engine agent observations

The normative bounds and authority distinctions are in [the observation profile](../../contracts/agent-observations-profile.md). This API is an upstream primitive for G6-025; Connect's durable projection and host authorization remain service integration work.

Install `Orchestrator::install_operational_event_stream(capacity)` before registering adapters. Consume `OperationalEventKind::AgentObservation { observation }` from that receiver and monitor `subscribe_operational_event_stream_health()`. The envelope supplies a unique local `correlation_id`, current optional Core source binding, capture time and global order. The nested observation exposes the producing AI Participant, provider role, per-route sequence, optional provider references, completeness and any known missing interval. Repeated tool IDs are separate observations. Neither requested tools nor reported results execute transfers.

Inspect `Orchestrator::agent_capabilities(connection_id)` before relying on optional adapter data. Default adapters report false capabilities and unknown remote availability. Vapi parsing and fresh-context support do not prove availability of a configured remote assistant.

To prepare a new private AI leg:

1. Select an authorized history snapshot and retain its watermark, completeness and source labels under the original logical operation.
2. Construct `AgentContextInitialization::new(conversation, session, ai_participant, operation_id, watermark, complete, items)`. Construction validates bounds without IO.
3. Build an outbound `OriginateRequest` for the same Session and AI Participant, with explicit fresh provider model options.
4. Call `Orchestrator::prepare_agent_context(request, &context)`. This returns a prepared request or a typed refusal; it creates no route and contacts no provider.
5. At the actual origination effect, enforce current host authority, including history audience and provider delivery. Use the normal origination path. Preparation is not reusable authorization.

A saved Vapi assistant with an unknown model cannot accept this hook; provide explicit inline model settings or assistant model overrides. The adapter preserves existing messages and serializes the fixed data envelope only for actual call creation. Existing calls use their separate APIs and are not silently mutated by this hook.

Run `python3 scripts/check-gate6-agent-observations.py` to prepare pinned dependencies, check formatting, build one shared feature profile, run the complete affected libraries and real local mock transport suite, and check locked Connect server integration. The report records exact source digests, binary hashes, observed cases, ignored targets and explicit qualification limits. It performs no live provider rehearsal.

## Protected host decoder (G6-043 in progress)

`rvoip_vapi::decode_agent_event(frame, captured_call_reference)` exposes the same
bounded normalizer used by the Vapi adapter for host-owned WebSocket jobs. The
reference must come from the host's captured provider instance. The function
performs no network IO, source registration, authorization or action dispatch.
Its returned metadata cannot establish a Connect Connection or Participant.

Both asserted `callId` and `call.id` must match the captured reference before a
terminal event or any observation is returned. Missing assertions remain tied to
the actual socket; contradictory assertions return `MismatchedSession`. Input is
bounded to 1 MiB. Every normalized item and optional event reference passes the
existing 64 KiB/512-byte metadata checks before any prefix is returned. An action
list over 32 items produces an unavailable observation, never a valid prefix.

`DecodedAgentEvent` exposes an optional reported event ID, typed contents and a
terminal flag; its diagnostics redact content and references. Partial/final and
missing transcript fields, provider speaker roles, requested actions and reported
results remain distinct. A provider's `user` role does not identify a human in
mixed audio. Repeated event/tool IDs are retained as observations; neither an ID
nor model-supplied task/effect arguments confer authority or prove completion.

The shared decoder validates captured call assertions before acknowledgment and
terminal handlers in both public and private jobs, including initial context,
context updates, audio/monitor events and cleanup. Explicitly configured jobs now
project typed contents through their captured observation sources. Host-selected
committed-question provenance is described below. Worker response proof through
public operations remains G6-043 work. The existing observation HTTP
surface remains [openapi-events.json](../../contracts/openapi-events.json); this
component adds no HTTP route and does not qualify the missing live integration.

## Captured public provider sources (G6-043 in progress)

The Store's protected `bind_native_assistant_observation_source` takes an opaque
public provider context and its committed exact acknowledgment. It derives the
Connection, generation, Participant, original audience and correlation from that
instance. A publisher still needs `session.observation.publish`, and publishing
for another identity requires `session.observation.publish.any`. A generic source
cannot replace the captured provider binding under the same correlation.

Publication and observation-page reads check the initial context and every prior
exported update against current history permissions. Revoked history, an
unacknowledged possible update, or admitted provider cleanup stops publication
and hides buffered derived content, even if the native Connection remains live.
Unused updates do not change the instance's acknowledged memory. These checks
retain the provider endpoint, original audience and captured physical generation.

`publish_native_assistant_observation` can reuse the original complete
`AssistantProviderRead` interval. Generic publication refuses a contended reader
immediately; it does not wait behind an authority writer while the caller may
hold that writer's required reader. Composed observation reads likewise reuse a
supplied complete interval and refuse insufficient proofs without upgrading.

## Captured private provider sources and host ownership (G6-043 in progress)

`bind_private_provider_observation_source` requires the original private purpose
and its exact committed provider acknowledgment. Its Connection, generation,
correlation and two-person audience derive from that captured selection. Neither
another instance's receipt nor another Store's retained interval can authorize
publication. Every buffered read and publication rechecks both original
listeners' exported memory, private lifetime and current incarnation. Closure,
uncertain export, stale generation and revoked history hide buffered content.

`publish_private_provider_observation` reuses the original
`PrivateProviderMediaRead`. That read includes the current Conversation identity
scopes needed by observation projection. It never acquires an extra reader
behind a writer while retaining the media interval. An insufficient or foreign
proof is refused; no fallback upgrades it.

The Service exposes `bind_assistant_observation_source` and
`bind_private_observation_source`, returning an owned `ProviderObservationSource`.
The provider job must retain that owner and close it before asynchronous cleanup.
Dropping the owner synchronously closes its activity restriction, including on
binding failure. The Store retains the restriction, not the owner. Publication
and buffered reads check that the captured registry entry and adapter are still
present and that the runtime has not stopped. These checks never reenter SQLite
or wait for authority. Ambiguous adapter/registry contention returns
`authority-busy`, preserving the consumer cursor. Permanent owner closure hides
sensitive content without waiting for provider cleanup SQL. The restriction can
only remove access; current Store authority remains necessary.

The public Service factory and owner lifecycle are exercised against an actual
signed native Connection and public HTTP reads. That ownership probe uses
controlled Store provider receipts; the existing HTTP/WebSocket journey tests
provider transport separately, including foreign-call update acknowledgments
and terminal reports. The actual private live job also installs this factory. Further native-loss
provider integration remains assigned to G6-044.

## Explicit live job capture (G6-043 in progress)

Both `AssistantVapiProfile` and `PrivateVapiProfile` accept optional
`AgentObservationConfig`. Construct it with a dedicated current publisher
credential and explicitly supported observation capabilities. It grants no
publication authority itself; the Store still requires publication permission,
current original memory and audience, and actual host source custody. Ordinary
context credentials intentionally lack publication permission. The configuration
never exposes its publisher credential to the provider or model tools; only the
host-selected proposal names and event subscriptions enter the provider payload.

After an exact committed initial context acknowledgment, the owned job binds its
source and starts a bounded worker. The media/control loop normalizes frames and
uses nonblocking queue admission; it never awaits Store publication. At most32
bounded observations (at most2MiB of inner data) wait per job. The worker obtains
fresh original-memory authority for publication. Busy/storage failures get a
bounded five-second retry; refusal drops the item. The next successful local
sequence reports the exact known missing interval, covering queue overflow and
failed publication. Capture begins after the initial acknowledgment and never
claims complete provider history, including an unknown terminal tail.

All capture/source owners close synchronously before asynchronous provider
cleanup. The worker is drained on the normal job exit; worker failure also closes
its source. Provider end cannot leave buffered derived content readable merely
because cleanup metadata recording is blocked. Closing a source hides every
buffered agent kind, including status metadata; consumers use the durable
Session/journal and context delivery lifecycle to establish actual outcomes.

Typed `ObservationDraft::Agent` preserves optional provider session/event/tool
references, provider speaker role, completeness, requested actions versus reported
results, local sequence and capture time. Repeated provider IDs produce distinct
local observations. The outer Participant/Connection/generation come only from
the captured source. `user` is a provider role, not a human identity. The inner
observation is bounded to65,536 encoded UTF-8 bytes and the wrapped draft to70KiB;
large/unsupported content becomes explicit unavailable data, never a truncated
transcript or valid action prefix. Generic adapter drafts retain their16KiB bound.
Rust, Node and Python readers expose the same typed data and enforce current
route capabilities. The generated [HTTP reference](../../contracts/openapi-events.json)
contains the new closed shapes.

Configured action names install asynchronous client function proposals with no
tool server URL and empty server-message subscriptions. Only the client
`tool-calls` subscription is added; no built-in transfer/end tool, remote action
executor or tool-result sender is installed. This follows Vapi's documented
[client function form](https://docs.vapi.ai/tools/client-side-websdk). A reported
result remains provider data and is never a Connect effect receipt. Parsing and
local fixtures do not qualify a named remote model/account configuration; those
rehearsals remain in their approved Gate6 lanes.

Trusted task/effect/decision correlation, external worker responses using stable
logical mutation IDs, and the actual duplicate-handoff/Session-end race proof
remain G6-043 work. No observation itself executes an operation or establishes
physical media readiness.

## Committed question provenance (G6-043 in progress)

A protected host can call `Store::prepare_task_observation_binding` with a current
publisher credential and explicit task/effect IDs. The resulting opaque
`TaskObservationBinding` belongs to that Store runtime and credential. It verifies
a committed Send effect with `wait_for_reply`, its immutable intent digest, its
accepted message outcome and actual message link. Current task owner inspection,
recipient ceiling and original question visibility are required. The host must
select the question explicitly; Session links and model arguments cannot select it.

Use `AgentObservationConfig::with_work` or the Service's captured-source
`*_with_work` factories to attach that binding. The exact acknowledged provider
memory must contain the question, either in its initial context or a delivered
public update. Every publication and buffered read rechecks current inspection
and original memory authorization. Revocation hides retained observations even
when the native connection and model memory remain independently current.

`ObservationRoute.work` and `SessionObservation.work` are either null or the same
closed object containing `task_id`, `effect_id` and `question_id`. These are
historical question labels. They provide no task grant, decision, semantic answer
assertion or continuation authority; terminal task inspection remains historical.
Provider tool arguments and reported results remain untrusted typed data.

Schema 64 preserves the captured context's work and activity restrictions outside
the transient buffer. Pruning a stale route cannot remove them. A captured context
correlation also remains reserved against generic source registration. A fresh
context may carry the same committed question; provider IDs never become a logical
mutation ID. A separately authorized worker still needs its own current public
operation authority and a retained logical decision before it can act. The actual
worker handoff/replay/Session-end proof is still required before G6-043 closes.

## External worker response through public operations

The prepared-handoff chain fixture contains a worker example in
[`chain/worker.rs`](../../crates/connect-server/tests/support/canonical_native_connections/consultation/groups/preparation/chain/worker.rs).
It uses only `ApplicationClient` and `CanonicalHttpClient`. Its separately
approved decision is persisted before submission and retains the complete
`TransitionCommand`, including original revisions and policy reference. This
example allows one approved handoff per question; its logical operation ID is
independent of provider event, call and tool IDs. Recreating the worker reloads
that same command instead of rebuilding an intent from newer topology.

Only a requested `handoff` observation with matching committed work labels can
prompt this example to consider its approval. Partial transcripts and reported
tool results do not dispatch. The worker inspects the current task and actual
question effect through the public task APIs, then submits the approved command
with its independently granted current Core handoff authority. Work labels never
supply that grant, a target or proof of completion. An inspection credential and
an execution credential are separate; neither is a fallback for the other.
The completion reply uses the question recipient's own communication credential.
A question author cannot answer its own question, and a handoff grant does not
grant management of the entire Session.

The worker reads the actual operation state through `operation.get` and replies
to the exact task question only after Connect reports completion. Repeating that
reply retains its logical request ID. A provider's reported `completed` result
cannot supply this state. Current authority is required again on replay,
including after worker recreation. A Session end that wins before admission
refuses the cached decision instead of causing a later handoff.

For a source created through handoff preparation, public context selection uses
the audience of its confirmed handoff. Session presence alone cannot include an
unrelated future target still held in isolation. This uses the same prepared
source restriction as observation publication; original acknowledgments,
audiences and generation checks remain required. The fixture's provider receipts
and typed provider inputs are controlled; its signed native peers, public HTTP
reads, physical handoff, replay, completion and media isolation are real. The
separate public/private HTTP/WebSocket provider journey exercises live capture.
