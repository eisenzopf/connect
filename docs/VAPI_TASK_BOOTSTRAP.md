# Vapi task bootstrap

This is a protected server composition component for G4-019. It creates a Vapi
chat **session** with Connect MCP already attached, then binds that session to
an existing Connect task. It is not yet a turnkey runtime installer or a
qualification of actual Vapi model/tool behavior. Local tests use a fake private
provider API and the actual Connect MCP endpoint.

Agents perform configured task operations without per-action approval prompts.
The developer establishes the task, allowed communication operations and
recipients. A provider callback cannot choose another task or mint broader
authority.

## Composition sequence

1. Create the conversation and task through the public client. Keep their IDs.
2. Choose one new private setup directory for this invocation. Compose
   `vapi_bootstrap::BootstrapAuth` with the service, configured private Vapi SDK
   client and that directory. Start a separate `connect_mcp::RunningMcp`
   listener with this authenticator. An unprepared directory denies requests.
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

## Recovery and limits

Keep the original private directory after an uncertain POST. Do not repeat the
POST, change the nonce or expiry, or create another task to conceal uncertainty.
A matching nonce-verified callback can reconcile the original resource before
the creation receipt. Calling `create_session` again after that observation
returns the independently verified existing resource under current authority;
it dispatches no second POST. Without a verified callback/resource, recovery
remains an explicit unresolved outcome.

Cancellation, grant revocation and backing-credential revocation prevent new
task effects. They do not undo a resource that the provider may already have
created. A saved provider observation is not authority. File contention produces
an explicit retryable setup failure; the original intent is retained.

Call/chat/session nonce verification is available in the connector; the creation
function currently implements **chat sessions only**. Call creation, attaching
this setup to the existing voice runtime, a protected operator installer,
explicit resource-ID reconciliation and actual Vapi host qualification remain
open. No live provider requests or public ingress are used by this component's
local checks.

Primary API references:

- [Vapi MCP configuration and connection lifecycle](https://docs.vapi.ai/tools/mcp)
- [Session variable substitution and metadata](https://docs.vapi.ai/chat/variable-substitution)
- [Official assistant override schema, including tools:append](https://github.com/VapiAI/server-sdk-python/blob/main/src/vapi/types/assistant_overrides.py)

Current progress and verification are recorded in the
[Gate 4 implementation plan](connect-gate-4-implementation.html#G4-019) and
[local bootstrap evidence](evidence/gate-4/vapi-bootstrap.json). The eight new
local integration tests pass; this does not close the remaining whole-item or
actual host qualification.
