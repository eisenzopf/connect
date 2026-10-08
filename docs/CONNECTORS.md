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
