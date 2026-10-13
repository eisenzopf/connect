# Writing a connector

Depend on `connect-connector-api`, not `connect-store` or `connect-server`.
The crate exports async `Connector` and `VoiceConnector` traits with owned request
and response data. Provider credentials belong in your adapter configuration,
not request DTOs, journal events, SDK replies, or Debug output.

`Connector::deliver` receives target, optional source, body and optional prior
chat context. Return provider ID and optional reply. `VoiceConnector` supports
start/status/end and bounded inbound discovery; discovery must be implemented
explicitly, not silently treated as an empty successful scan.

Return `Failure::Rejected` only for definite rejection without the requested
effect. Return `Failure::Uncertain` when an effect might have occurred. Never retry
an ambiguous creation inside the adapter. Set bounded network deadlines, cap
responses, validate control URLs, and preserve provider IDs for reconciliation.
The server maps durable jobs into these types and commits provider observations.

Inbound observations are untrusted provider data, not Connect user authority.
Report the full requested discovery window or fail it; the server advances its
watermark only after the complete batch is imported. Duplicate observations must
remain safe. Private call-control URLs deliberately do not implement Debug.

The compiled example is deliberately small:

```sh
cargo run --locked -p connect-connector-api --example echo
```

For a real adapter, use a loopback fake provider in tests to verify success,
definite rejection, partial response/deadline ambiguity, body bounds and recovery.
Register it in server composition with an explicit durable route. Dynamic runtime
plugin loading and arbitrary connector registration through the public API are
not implemented. Existing Telnyx/Vapi adapters show the current integration seam.

## Context-bearing adapters

Implement `descriptor()` alongside `name()` and `deliver()`. Use
`Descriptor { conversational: true, ..Descriptor::one_way("your-connector") }`
for an opaque-target assistant. Choose an address contract appropriate to the
adapter. `Worker::with_drivers` registers descriptors; operator provisioning before
worker startup can call `Store::register_connector` from trusted composition.
Registration is not a public SDK operation or an authorization grant.

See [the entry contract](connector-entry-contract.md) and
[the independent adapter test](../crates/connect-server/tests/entry_connector.rs).
The existing Vapi/Telnyx descriptors preserve their validation and policy. New
context adapters do not need edits to conversation authority or provider switches.

### Task context and replies

For a task-origin turn, Connect reconstructs context from the original invocation
grant and the task's current authority. A History or ContextView permission allows
a bounded, source-backed task recall page, with an explicit omitted count and
continuation cursor. A Send-only grant supplies no history. Task turns never
inherit the route's previous provider chat ID; doing so would bypass the local
context ceiling through remote history. The turn still belongs to the same
durable Connect conversation. Its authorized history comes from Connect's
projection rather than an unrestricted remote continuation.

Keep those boundaries in Connect rather than rebuilding task policy in a
connector. Deliver the supplied context as conversation data, preserve source
references, and return the provider ID and answer through `DeliveryResponse`.
Connect derives the answer's task and question from the committed provider-job
effect link. Message admission, task inbox and wakeup commit in one transaction.
A late answer to a cancelled or expired task stays visible as unassigned
communication and does not resume the task.

The [provider task evidence](evidence/gate-4/provider-task-boundaries.json)
distinguishes store and public-client qualification from the remaining Vapi
MCP, voice and installed-host acceptance.

### Task-bound native voice context

The Vapi media adapter uses `Store::prepare_vapi_session_context` to select and
persist the exact context before attachment. Native sessions must be accepted,
linked to the task and currently permitted by the invocation. History comes
from bounded task recall under the grant's ceiling. A media-only invocation
supplies an empty source selection; that does not claim the conversation has no
history. The adapter checks the current route again after provider attachment,
before exporting context. Transcript and outcome observations remain in the
persistent conversation after the leg ends.

The [voice context evidence](evidence/gate-4/task-voice-context.json) records
qualified local SDK/media behavior and its limits. This path does not yet constitute
verified Vapi MCP installation or the full Gate 4 post-call/restart journey.

### Verify a Vapi origin before registering its task binding

`Vapi::verify_origin` uses the pinned private API SDK to match a call, standalone
chat or chat session against protected setup's exact resource and saved-assistant
IDs. Calls and sessions must be active; a session chat cannot masquerade as a
standalone chat. Invalid path identifiers never reach the provider. Lookup
failures return safe errors rather than raw provider bodies.

`VerifiedVapiOrigin` is a read-only observation with private fields. It is neither
an invocation grant nor a substitute for current task authority. Trusted setup
must still register and issue under current authorization, handle uncertain
creation, export only the invocation's credential and revalidate lifecycle.
The [origin verification evidence](evidence/gate-4/vapi-origin-verification.json)
qualifies that earlier lookup component. Protected first-invocation composition
is implemented in [Vapi task bootstrap](VAPI_TASK_BOOTSTRAP.md). Historical
[voice continuity](evidence/gate-4/live-voice-continuity.json) and fresh
[dc73996 scoped text/tool use](evidence/gate-4/actual-vapi-chat-dc73996.json)
retain their actual source and topology. Current owner-review repairs are qualified at 709cf82 in the [numbered plan](connect-gate-4-implementation.html).
The lookup checkpoint remains historical component evidence; the current full review is linked below.

### Register and issue for an existing verified resource

Protected Rust composition can call `connect_server::vapi_task_setup::issue`
with the backing credential reference, stable issuance ID, exact task and
provider origin, expected assistant, operation subset and original expiry.
The resource must already exist. Current authority is checked before the private
lookup and again inside the final binding/grant transaction. A failed lookup
cannot register an origin; a failed grant insert rolls back registration.
Matching replay preserves the secret and expiry. Call, standalone chat and
session chat have separate adapter namespaces.

The returned `VerifiedVapiTaskGrant` supports explicit protected export through
`issued()` and fixed MCP admission through `authenticator()`. Each MCP request
still rechecks current grant/task/parent authority before interpreting provider
correlation headers. Provider origins remain excluded from the public local
task-access exchange. Neither model arguments nor headers can invoke setup.

The [setup evidence](evidence/gate-4/vapi-task-setup.json) retains its historical component qualification.
This existing-resource API does not create provider resources, handle uncertain creation,
install tool configuration on an assistant, or solve first-invocation bootstrap.
Those lifecycle steps and main runtime integration are delivered separately through
[Vapi task bootstrap](VAPI_TASK_BOOTSTRAP.md); the current full review is linked below.

Current owner-review qualification: Runtime 709cf82 passes 42 offline phases, 17 installed checks and 32 actual MCP calls with no errors. Qualification controls d64f1b9 combine the reviewed fixtures at 7bdd534 with all 3 hosted jobs. Local macOS arm64 owner review only; Gate 5, community publication and remote deployment remain separate.

[offline-709cf82](evidence/gate-4/offline-709cf82.json), [package-709cf82](evidence/gate-4/package-709cf82.json), [real-model-agent-709cf82](evidence/gate-4/real-model-agent-709cf82.json), [review-remediation-709cf82](evidence/gate-4/review-remediation-709cf82.json), [qualification-controls-d64f1b9](evidence/gate-4/qualification-controls-d64f1b9.json)
