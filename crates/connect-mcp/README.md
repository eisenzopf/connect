# Connect MCP adapter

Gate 4 implementation is in progress. This crate composes the official Rust MCP
SDK with Connect's task service. It does not open the database or expose grant
issuance as a model tool.

The current transport is opt-in loopback Streamable HTTP at `/mcp`. Trusted
composition registers an invocation binding, issues a narrow opaque task grant
and exports that secret to protected host configuration. Each HTTP request
presents the grant as its bearer token; the admission adapter verifies current
authority before interpreting model arguments. The handler derives its task
from the private authenticated principal rather than an input task/owner ID.
Bearer plaintext is removed before the MCP SDK receives HTTP context.

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
the private SDK and atomically registers/issues under current authority. Resource
creation and first-invocation bootstrap remain open; existing-resource setup
does not qualify those steps.
The local tests use explicitly registered fixture origins; they do not qualify
an actual Vapi account. Never use one assistant-wide bearer to select tasks
from `X-Call-Id`, `X-Chat-Id` or `X-Session-Id`. These headers cannot create or
replace an invocation binding. See [Vapi's MCP header contract](https://docs.vapi.ai/tools/mcp).

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
Every discovery and operation revalidates current task grants.

Requests are bounded to 512 KiB, including streamed bodies. Host validation is
restricted to the actual loopback listener authority and localhost port. Present
browser Origins are rejected by default. At most 32 requests occupy admission;
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
native/host configuration. Omitting `[mcp.binding]` selects `LocalGrantAuth`,
allowing independent protected local task installations to share the listener.
Each request still needs its own exact currently authorized grant; the URI and
configuration cannot select a task or mint authority.

Discovery projects the tool catalog from current grant operations: status,
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

Native grant authentication, protected CLI issuance/export and independent MCP
transport are locally qualified in the numbered plan. Protected host installation,
provider creation/bootstrap, installed hosts and complete Gate 4 acceptance remain open.
`connect_task_cancel` is visible only when the developer explicitly includes
`task_cancel` in both the task ceiling and this invocation grant. It cancels only
the bound task and atomically revokes its grants; replay through that grant is
then denied. A protected owner can inspect the accepted cancellation receipt.
The tool cannot close a conversation or undo an external dispatch. Existing
grants have no cancellation permission. Other administrative task control
remains outside invocation grants. Transport, authority and tools
have dedicated real-client tests in `crates/connect-server/tests/gate4_mcp_*.rs`;
the numbered plan records which candidate checks have actually passed.
