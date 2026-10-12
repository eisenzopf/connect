# Connect MCP adapter

This crate composes the official Rust MCP SDK with Connect's communication
service. The default local endpoint accepts one regular, revocable agent
credential across that identity's authorized conversations. Tasks optionally
track work within those conversations; they do not require issuing or renewing
an invocation grant. The adapter does not open the database or expose credential
or grant issuance as a model tool.

The transport is opt-in loopback Streamable HTTP at `/mcp`. `LocalAuth` verifies
the bearer against current identity authority before interpreting arguments.
Ordinary tools take explicit conversation IDs and persistent mutation request
IDs. Optional tracking tools select an owned task explicitly. Discovery exposes
only currently permitted communication operations, without administrator
controls or provider secrets. Bearer plaintext is removed before the MCP SDK
receives HTTP context. See [regular agent installation](../../docs/AGENT_ACCESS.md).

For an optional narrow installation, trusted composition registers an invocation
binding, issues an opaque task grant and exports that secret to protected host
configuration. The authenticated principal supplies its fixed task and binding;
model arguments cannot select another task. `LocalAuth` also admits these local
grants with their existing narrow schemas and authority limits.

`FixedBindingAuth` supports an explicitly configured native or host-session
binding. `LocalGrantAuth` supports multiple independently registered native/host
grants on one endpoint; the authenticated grant row supplies each task and
binding. Neither mode accepts a provider grant. `VapiBindingAuth` admits one already verified and registered provider
origin. Its `VapiOrigin` selects an exact call, standalone chat or chat-session
ID. It authenticates the configured narrow grant before matching correlation
headers, and rejects missing, duplicate, malformed or conflicting IDs. A
session binding covers chats within that session; each request must include
both its session ID and a bounded chat ID. The session grant does not authorize
another session or task. Native/host and provider grants are not interchangeable.

Trusted composition must verify the provider resource, register the binding
and export a separate grant to its protected MCP headers. This adapter does
not perform provider lookup or webhook verification, and the default server's
native/host configuration rejects provider origins. The server's
`vapi_task_setup::issue` factory verifies an already-existing resource through
the private SDK and atomically registers/issues under current authority.
Separate [server bootstrap](../../docs/VAPI_TASK_BOOTSTRAP.md) implements
chat-session creation and rvoip-owned voice staging, verifying the first callback
before binding it. Existing-resource setup alone does not qualify resource creation.
Local and installed tests use provider simulators; actual account/model evidence
retains its separately recorded source and limits. Never use one assistant-wide
bearer to select tasks from `X-Call-Id`, `X-Chat-Id` or `X-Session-Id`. These headers
cannot create or replace an invocation binding. See [Vapi's MCP header contract](https://docs.vapi.ai/tools/mcp).

```rust,no_run
# async fn start(service: std::sync::Arc<connect_service::Service>, binding: connect_protocol::tasks::InvocationBinding) -> Result<(), Box<dyn std::error::Error>> {
use connect_mcp::{FixedBindingAuth, RunningMcp};
use std::sync::Arc;

// Trusted composition registered `binding` and issued its grant beforehand.
let auth = Arc::new(FixedBindingAuth::new(service.clone(), binding)?);
let listener = RunningMcp::start_local(service, auth, "127.0.0.1:0".parse()?).await?;
// Install the URI and protected grant in the intended host, then use the tools.
listener.shutdown().await?;
# Ok(()) }
```

The pinned SDK is `rmcp = 3.5.1`. Connect limits negotiated protocols to the known
legacy versions through `2025-11-25`; newer optional facilities are not
advertised. MCP task facilities do not replace Connect's durable task identity.
Stateless requests retain no MCP transport session as application authority.
Every discovery and operation revalidates current identity or invocation authority.

Requests are bounded to 512 KiB, including streamed bodies. Host validation is
restricted to the actual loopback listener authority and localhost port. Present
browser Origins are rejected by default. At most 32 requests occupy admission
across a server’s primary and Vapi task endpoints;
one ten-second deadline covers admission, body decoding and response construction.
Authentication retains its five-second ceiling. Parsed tools stop at 9.5 seconds
to reserve response time; event waits use the remaining budget and leave 250 ms
for authority revalidation and authoritative inbox projection. An incomplete
body receives HTTP 504 at the outer deadline. These are server work limits;
slow reader/network delivery is outside that clock. Server shutdown cancels
admission and handlers and reports a failed drain after five seconds. Dropping
the listener cancels and aborts its owned accept task.

The server's existing `connect serve --config server.toml` command starts the
endpoint when an explicit non-secret configuration is present:

```toml
[mcp]
bind = "127.0.0.1:8091"
[mcp.binding]
kind = "host_session"
adapter = "connect-host"
external_id = "registered-session"
```

The binding must already be registered and each request must present a current
grant for that exact binding. Configuration neither registers an origin nor
issues a grant. The server prints the actual endpoint URI and owns its shutdown
alongside the native QUIC listener. Provider origins cannot use this fixed
native/host configuration. Omitting `[mcp.binding]` selects `LocalAuth`, allowing
regular agent credentials and optional independent local task installations to
share the listener. The URI and configuration never mint authority. A regular
credential retains its named operation and recipient limits plus conversation
membership; a task grant retains its exact registered binding.

Regular discovery includes identity inspection, conversation creation/history,
inbox, context, messaging, file and supported media tools. Optional owned-task
tools provide creation, state, recall, waits, tracked sends and cancellation.
Normal results retain the native `Outcome` envelope, including
`{"kind":"task","data":TaskOutcome}` for task operations. A regular send uses
the native message contract; a tracked question uses `connect_task_send` with
`task_id`, `request_id` and `step_id`. `connect_reply` refers to the original
question message ID. A tracked send's reply reference instead names a received
journal event. Current schemas describe these distinct references.

Narrow discovery projects the tool catalog from current grant operations: status,
source recall, journal wait, effect status, recipient discovery, messaging,
artifact chunks and supported session/call/SIP operations. A tool's presence
does not imply a configured or reachable provider route. Communication tools
require persisted `request_id` and `step_id`, including ledger-backed reads;
the adapter derives task/conversation identity and rejects authority selectors.
Matching mutations reconcile through the same effect ledger as SDK operations.
Sources and file references are untrusted data. File chunks are at most 32 KiB
decoded; lower configured limits still apply. No tool accepts a filesystem path.

Bounded waits return the normal task inbox outcome plus `_meta.connect` with
`task_id` and `pending`; an empty page is not task completion. Keep the journal
cursor and resume under current authority. A pending send/effect uses the shared
typed task outcome. A disconnected mutation must reconcile its original logical
IDs instead of generating new ones.

An abandoned or deadline-expired tool returns `transport-uncertain` when a result
can still be constructed. `_meta.connect` retains the authenticated `task_id`,
validated `request_id`/`step_id` when available, and `reconcile: true`. This is
not proof of rejection or completion. Keep the original mutation inputs and
IDs; inspect a permitted effect handle or retry the matching logical command
under current authority to recover its original receipt. An earlier empty
authoritative inbox page remains the normal pending result with its cursor.
The adapter never invents a page after a timed-out read. Request cancellation
stops the waiter, not a store transaction that may already have committed.

Optional native grants, protected CLI issuance/export, host installation and
provider bootstrap are implemented. The [09b77a3 local package](../../docs/evidence/gate-4/package-09b77a3.json)
passes all 17 installed checks, including 12 Vapi simulator cases. Current full
regression, actual normal-agent model qualification and final Gate 4 acceptance
remain separate; live provider reports retain their recorded source and limits.
`connect_task_cancel` is visible only when the developer explicitly includes
`task_cancel` in both the task ceiling and this invocation grant. It cancels only
the bound task and atomically revokes its grants; replay through that grant is
then denied. A protected owner can inspect the accepted cancellation receipt.
The tool cannot close a conversation or undo an external dispatch. Existing
grants have no cancellation permission. Other administrative task control
remains outside invocation grants. Transport, authority and tools
have dedicated real-client tests in `crates/connect-server/tests/gate4_mcp_*.rs`;
the numbered plan records which candidate checks have actually passed.

[Vapi chat-session bootstrap](../../docs/VAPI_TASK_BOOTSTRAP.md) describes the
protected first-invocation component. It saves creation intent before one
provider POST and requires private nonce verification before binding callbacks
to a task. Its setup credential is distinct from ordinary task grants. Call
staging, voice-runtime composition and the operator entry points are implemented
and locally checked. The guide links installed and live evidence separately;
final current-source acceptance remains pending.

## Shared endpoint capacity

`McpCapacity` owns one server's request and listener budgets. Clone the same pool
for its primary endpoint and task endpoints. The server does this automatically
for protected Vapi voice setup, including when the primary MCP endpoint is not
configured. Embedders using the `RunningMcp::start_local*` convenience functions
get independent pools; use `McpCapacity::start_primary` / `start_auxiliary` for
a group that must share limits.

The pool admits at most 32 starting or live listeners and 32 simultaneous
requests in total. Listener exhaustion returns `io::ErrorKind::WouldBlock` before
binding; request exhaustion returns HTTP 503 before authentication. Neither
path adds a waiting queue or changes authority. A listener slot is held until
its owned accept task exits, including cancelled startup/shutdown. Failed binds
return the slot. Existing body, authentication, tool and response deadlines
remain unchanged. These are admission bounds, not throughput or total-memory
measurements; full Gate 4 operating qualification remains open.
