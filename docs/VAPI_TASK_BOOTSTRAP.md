# Vapi task bootstrap

This is protected server composition for G4-019/G4-020. It creates a Vapi
chat **session**, or stages an rvoip-owned voice call, with Connect MCP already
attached and an explicit binding to an existing Connect task. It is not yet a turnkey runtime installer or a
qualification of actual Vapi model/tool behavior. Local tests use a fake private
provider API and the actual Connect MCP endpoint.

Agents perform configured task operations without per-action approval prompts.
The developer establishes the task, allowed communication operations and
recipients. A provider callback cannot choose another task or mint broader
authority.

The installed `connect vapi-task serve` and `reconcile` entry points are
described in [Vapi task host](VAPI_TASK_HOST.md). They reuse this composition;
local command checks and release/provider qualification remain separate.

## Chat-session composition

1. Create the conversation and task through the public client. Keep their IDs.
2. Choose one new private setup directory for this invocation. Compose
   `vapi_bootstrap::BootstrapAuth` with the service, configured private Vapi SDK
   client and that directory. Start an auxiliary listener from the server's
   shared `connect_mcp::McpCapacity` with this authenticator. The standalone
   `RunningMcp::start_local_auxiliary` convenience creates its own capacity pool
   and is only appropriate for an isolated endpoint. An unprepared directory
   denies requests.
3. Call `vapi_bootstrap::prepare` with the actual MCP URL, current backing
   credential **reference**, stable issuance request ID, existing task,
   saved assistant ID, `OriginKind::ChatSession`, operation subset and expiry.
   Preparation writes a fresh nonce and setup secret under private permissions;
   it creates no provider resource, Connect binding or task grant.
4. Call `vapi_bootstrap::create_session` once. The durable dispatch intent is
   saved before the private SDK POST. The file lock is released before network
   I/O so the first MCP discovery/call can arrive before the creation receipt.
5. The callback first authenticates this setup secret. Session/chat headers
   provide correlation only. Connect independently reads the exact session
   through the private provider API and requires its saved assistant, active
   status and exact creation nonce. Only then does current service/store
   authority atomically register the task binding and issue its narrow grant.
   Subsequent callbacks reuse that issuance intent and recheck current authority.

The session request uses `assistantOverrides.variableValues` to carry
`connect_invocation_id`. It uses `assistantOverrides["tools:append"]` to attach
the MCP tool without replacing the assistant's existing tools. The setup secret
is delivered only as the provider tool's configured Authorization header. It is
not an ordinary native/host-session grant and is never a model tool argument.

The nonce is checked in the private session response's
`metadata.variableValues.connect_invocation_id`. A matching assistant or caller
header alone is insufficient. Once verified, the resource ID is frozen in the
setup intent. Another resource cannot reuse it, even with matching metadata.

## Voice composition

Create and accept an audio session through the public task client. Configure
its saved-assistant voice route, then call
`RunningServer::prepare_vapi_task_session` with that session ID, a protected
`Definition` using `OriginKind::Call`, a fresh private directory and the private
provider SDK. The owner-selected task must have created the accepted session;
another task in the same conversation cannot borrow it. The server owns the
new local MCP listener and drains it during shutdown.

Opening native audio uses the existing rvoip attachment and media bridge. The
bootstrap stages the nonce and MCP tool before call creation. `stage_call`
always returns options with an admission hook; it never performs its own POST.
The source-locked, opt-in rvoip hook runs after request preparation and directly
before HTTP send. In one Store transaction, it rechecks current task, backing
credential, authenticated media caller, operation ceilings, original expiry and
exact route, then commits one physical-call admission. Competing attempts and
restart cannot dispatch that same call intent again. Default rvoip call options
retain their existing behavior.

Call callbacks verify `assistantOverrides.variableValues.connect_invocation_id`
through the private provider API before grant issuance. They can arrive before
the creation receipt. Tool results remain tied to the original task; bounded
voice context uses the native caller's history permission. Ending voice leaves
the task and conversation intact, and supported final transcripts enter that
conversation's journal for later recall.

## Advertised callback address

The additive `RunningServer::prepare_vapi_task_session_with_callback` API
accepts an optional operator-selected MCP URL. `None` keeps the original local
URL. The listener still binds loopback and uses the server's shared capacity;
the override changes only the URL saved in the immutable setup and sent to Vapi.
It requires HTTPS or explicit loopback HTTP, exactly `/mcp`, and no embedded
credentials, query or fragment. An invalid URL is rejected before a listener or
private intent is created. It cannot replace another setup on the same session.

An operator proxy must map that URL to the returned local listener and rewrite
HTTP `Host` to the backend address, such as `127.0.0.1:PORT`. Preserve
`Authorization`, provider correlation headers and `Origin` when present. Connect
continues validating host, origin, independently verified provider identity and
current task authority. Merely forwarding TCP to the backend while preserving
a different HTTP Host does not satisfy this mapping. The API does not create a
proxy, DNS record, TLS termination or public ingress.

The [callback component checks](evidence/gate-4/vapi-callback-address.json) pass
through an owned loopback HTTP proxy and simulator, including rejection and
cancellation. They do not establish public reachability or actual cloud-provider
compatibility. A shipped operator setup
and installed Vapi rehearsal remain open under G4-022/G4-033/G4-034. Existing
full-run evidence predates this additive API.

## Tool results and continuation

An unanswered question sent with `connect_send` and `wait_for_reply: true`
stays pending even after the send effect is `succeeded`: committing the question
and receiving its answer are separate observations. Use
`connect_effect_status` when the invocation has `RequestStatus` permission to
retrieve that effect's accepted message reference. Task inbox pages deliberately
omit effect outcome bodies so journal catch-up remains bounded.

`connect_task_wait` returns a bounded pending result and a journal cursor when
there is no new event. A disconnected client can reuse that cursor under current
authority. Preserve the original request and step IDs when inspecting or retrying
the same logical operation; reconnecting is not a reason to send it again.
Business rejection can persist one failed effect without a message or provider
job. Matching replay returns that failure without creating another effect.

An ended provider call cannot authorize later MCP operations. To continue in
chat, prepare a new chat-session invocation on the **existing task and
conversation**, with the needed configured operations. Recall retrieves retained
voice sources and subsequent replies with their original provenance. The new
invocation can use that information for a permitted message without recreating
the call or its media connection. Invocation setup is developer composition;
agents still perform their configured operations without per-action prompts.

## Recovery and limits

Keep the original private directory after an uncertain POST. Do not repeat the
POST, change the nonce or expiry, or create another task to conceal uncertainty.
A matching nonce-verified callback can reconcile the original resource before
the creation receipt. Calling `create_session` again after that observation
returns the independently verified existing resource under current authority;
it dispatches no second POST.

If no callback arrived, protected developer composition can call
`vapi_bootstrap::reconcile_created(service, &provider, &directory, external_id)`
with the exact resource ID obtained from the provider. Retain the original
directory, provider account and intent; do not select a resource by matching
caller or assistant alone. The function privately verifies the original nonce,
assistant and active resource mode, freezes that resource ID, and uses the same
grant issuance intent as a callback. It rechecks current authority before the
lookup and during issuance. Matching retries preserve the original lifetime;
an already revoked grant stays revoked. It returns the verified resource ID,
never a bearer credential. The function is not exposed as a model tool.

This reconciles task tool binding, including a call whose creation receipt was
lost. It does not reconnect audio or resume an ended voice leg. Without an
exact, active, nonce-verified resource, the creation outcome remains unresolved.

Cancellation, grant revocation and backing-credential revocation prevent new
task effects. They do not undo a resource that the provider may already have
created. A saved provider observation is not authority. File contention produces
an explicit retryable setup failure; the original intent is retained.

Call/chat/session nonce verification is available in the connector; standalone
chat creation is not implemented. The voice runtime accepts protected Rust
composition, but a turnkey operator installer, restart reattachment,
full operating-profile measurement and actual Vapi
host qualification remain open. Shared setup admission is described below. A staged or admitted uncertain call is not
silently recreated. Local listeners cannot be reached by a live cloud provider
without separately configured ingress; this component does not add ingress. No live provider requests or public ingress are used by this component's
local checks.

Primary API references:

- [Vapi MCP configuration and connection lifecycle](https://docs.vapi.ai/tools/mcp)
- [Session variable substitution and metadata](https://docs.vapi.ai/chat/variable-substitution)
- [Official assistant override schema, including tools:append](https://github.com/VapiAI/server-sdk-python/blob/main/src/vapi/types/assistant_overrides.py)

Current progress and verification are recorded in the
[Gate 4 implementation plan](connect-gate-4-implementation.html#G4-019) and the historical [chat-session bootstrap evidence](evidence/gate-4/vapi-bootstrap.json).
[Qualified local voice component and retained failures](evidence/gate-4/vapi-call-bootstrap.json)
and [exact-resource recovery checks](evidence/gate-4/vapi-bootstrap-recovery.json)
are recorded separately. These component checks do not close whole-item or
actual host qualification.

The [G4-019 binding item audit](evidence/gate-4/vapi-binding-audit.json) now verifies
the local binding contract, including two overlapping calls to the same assistant,
concurrent first callbacks, independent effects and teardown. It combines those
checks with origin, lifecycle and recovery proofs. The
[G4-020 continuation audit](evidence/gate-4/vapi-continuation-audit.json) now
qualifies the local context/tool runtime with 31 behavior checks, including
bounded pending, disconnect, server restart and a useful follow-up from retained
voice and reply sources. Actual provider/model behavior, installed-host and
complete journey qualification remain separate work.

## Server-wide callback resource budget

Protected voice setup shares the server's `McpCapacity` with its primary MCP
endpoint: 32 starting/live listeners and 32 simultaneous requests total. A
rejected setup at capacity creates no private intent and no provider call.
Closing one endpoint returns its allowance for other task endpoints.

An ended attached call closes its callback listener. Idle or active setup
listeners are also retired after their original grant deadline by the media
lifecycle's one-second expiry scan; existing bounded drain rules apply. This
does not refresh a grant, end the persistent task or delete saved recovery
intent. Schema 16 persists the original setup binding in the Store before media
registration. Retirement releases the process-local entry; a later attachment,
including after restart, cannot fall back to an ordinary call. Creating another
setup for that session is rejected. Continue the same task and conversation with
a new voice session and an explicit new setup. An uncertain original provider
call still requires reconciliation using its original private intent.

The additive, backup-gated migration reserves older task-created sessions with
unknown setup identity. It preserves their history and never guesses a nonce or
adopts a new setup for an old leg. It does not modify ordinary non-task sessions.
The [restart repair audit](evidence/gate-4/vapi-setup-restart.json) records the
original regression and verified repair: 211 Store and 32 Vapi/parity checks,
including ordinary voice-only task calls, pass with strict lint and Rustdoc.
The [full retirement profile](evidence/gate-4/voice-retirement-load.json) now
qualifies 10,000 setups across 313 real-expiry batches, preserves the exact
binding digest across restart and records whole-process memory samples.
Current integrated/candidate qualification and whole G4-030 remain open.

The [shared-capacity audit](evidence/gate-4/mcp-shared-capacity.json) records the
local checks and their exact qualification state. It retains the interim
restaging regression and the native reread timeout; it does not close G4-030.
