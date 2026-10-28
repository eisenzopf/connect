# Node capability observations

`NodeClient.capabilities.refresh(options)` reports local interface support for
the selected Node facade. The managed helper obtains the original Rust SDK's
capability snapshot, validates the profile and selected transport, then the Node
adapter intersects that snapshot with features exposed by its own public API.
The runtime role is `node`, the current selected transport is `quic`, and the
profile is `connect/2`.

The current typed Node facade advertises `original-id-operations`. It does not
yet advertise the helper's native programmatic media, durable events, incoming
invitations, raw canonical or application receipt interfaces. Separate low-level
clients do not imply that those interfaces exist on `NodeClient`.

```ts
const support = await client.capabilities.refresh({ timeoutMs: 2000 });
if (support.features.has('original-id-operations')) {
  await client.capabilities.require('original-id-operations', { timeoutMs: 2000 });
}
```

The feature set is immutable at runtime, including the set supplied to `forEach`
callbacks. `require` validates its feature name and performs a fresh helper
observation. An unavailable feature raises a structured `unsupported` error
with `read-only` certainty. Invalid names and already-aborted requests cannot
write helper input. Profile, helper runtime, transport or duplicate-feature
violations fence the original helper through the shared protocol-error path.

These observations imply no server grant, media readiness, device permission or
successful audio. Current authorization and negotiated readiness remain separate
checks on each actual operation. Refresh does not contact the server itself;
it still uses the bounded, cancellable managed-helper exchange and fixed client
configuration. The earlier initial-create deadline failures are not resolved by
adding this observation.

The native CI harness records only fixed failure-stage labels, structured error
enums and source coordinates in uploadable evidence. Raw helper/server
diagnostics remain private; a synthetic invalid-profile run verifies the
redacted failure path.

G6-062 remains in progress. See
[capability evidence](../evidence/gate-6/node-capabilities.json) and the
[Connection control scope](node-connection-sdk.md).
