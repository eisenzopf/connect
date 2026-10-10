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
