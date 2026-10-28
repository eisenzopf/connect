# Node canonical helper component

G6-062 is in progress. `CanonicalNativeClient` implements finite current
canonical control using one owned Rust `connect stdio` process. The typed
[NodeClient component](node-typed-sdk.md) additionally implements Conversation,
message and durable operation/decision recovery through the public Rust SDK.
The full SDK, trusted provisioning, remote HTTP enrollment and full journey remain open.
Programmatic media is G6-064.

Install the matching trusted `connect` executable and protect its native client
TOML, enrolled signing key and trust files. The npm component includes JavaScript
and declarations; it does not yet bundle that executable. The helper loads its
configuration once and owns native TLS, UCTP, signature verification and current
server authority. Node never copies provider keys or reloads the enrollment.

```js
import { CanonicalNativeClient } from '@rudeless/connect';

const client = new CanonicalNativeClient({
  config: '/absolute/path/to/native-client.toml',
  executable: '/absolute/path/to/connect',
});
try {
  const frames = await client.request(
    'conversation.get', {}, { cid: existingConversationId },
    { timeoutMs: 1500 },
  );
  // Correlated verified observations through the original ACK.
} finally {
  await client.close({ timeoutMs: 1500 });
}
```

The finite request surface uses the current payload and scope vocabulary:
`cid`, `sid`, `connid`, caller-owned `operation_id` and `expected_revision`.
The bridge converts scope to the Rust command shape. Save a mutation's ID and
exact intention before calling. An ACK acknowledges the request; logical
completion is read through the original operation's journal state.

Each request captures its intent before queuing and has one absolute deadline
covering queue, write and response. At most sixteen exchanges are owned, with
one submitted to the sequential helper at a time. An unsent queued timeout or
cancellation removes only that intent. A submitted timeout/cancellation, partial
response, unexpected output or transport failure fences the original instance
and stops its helper. Errors retain the saved mutation key and distinguish
not-submitted, refused, uncertain and read-only outcomes; raw helper messages and
diagnostics are suppressed. No automatic restart, retry or transport fallback
occurs. Reopen explicitly and query the saved operation before deciding recovery.

The response wrapper is the current `connect-control/1` CLI protocol containing
verified `connect/2` envelopes. Independent helper exchange IDs, Rust wire request
IDs and logical mutation IDs stay distinct. The adapter checks bounded UTF-8,
canonical JSON, duplicate keys, exact wrapper shape, correlation, scope, terminal
ordering and signature shape. It preserves the Rust-verified original signature;
it does not claim an independent JavaScript cryptographic verification.

Idle `close()` sends EOF and joins the original helper. Closing an active
exchange preserves its uncertainty. A missed close deadline stops the owned
process and reports `cleanup-incomplete`; a successful local close does not
submit Session or Connection end. Live subscriptions and native media require
their dedicated SDK interfaces; finite event requests require `live:false`.

Run the source-bound component check from the repository:

```sh
python3 sdk/tests/check_canonical_helper.py \
  --output .local/gate-6/node-helper-FRESH
```

The 38 tests and negative TypeScript declaration checks use synthetic raw/typed
helpers and a fake HTTP listener. They exercise real
owned child processes, deadlines, queue cancellation, malformed/unsigned/foreign
output, redacted refusals and joined cleanup. Fixture signatures are shaped data;
these tests do not qualify cryptography or current server authorization.

The candidate can be packed locally from `sdk/typescript` with `npm pack` into
`dist/`, installed with `npm install --offline --ignore-scripts` in a fresh
external project under `.local/`, and exercised by copying
`sdk/tests/canonical_helper_consumer.mjs` into that project. Invoke the copied
script with the absolute path of `sdk/tests/fixtures/canonical_helper.mjs`.
That consumer verifies package exports, native/HTTP scope IDs, query-after-send,
original helper EOF exit0 and joined PID. It also uses synthetic backends.

Evidence and remaining work are in
[`node-canonical-helper.json`](../evidence/gate-6/node-canonical-helper.json).
No npm publication or full G6-062 qualification has occurred.

The installed real-server component also passes for EdDSA and ES256 on the
recorded local macOS arm64 host. It sends a real message through the Rust helper,
reads the same durable operation/result through both facades, replays the same
mutation without another message, reads finite events, edits configuration
without changing the running helper, and then restarts the worker to query the
original operation. After committed revocation, both facades report
`authentication/refused` with the saved mutation key. All four exec-traced helper
PIDs and both servers join. This covers message/journal control, not the complete
typed session/topology/provisioning journey.

To reproduce, copy `sdk/tests/canonical_native_server_consumer.mjs` into the
fresh external project with the locally installed package. Build the
`gate6_canonical_runtime` integration executable with `test-support` after
preparing dependencies. Set `TMPDIR` to a fresh private directory under `.local/`
and `CONNECT_GATE6_NODE_CONSUMER` to that copied script's absolute path. Run the
exact ignored test
`canonical_stdio::node::installed_node_helper_preserves_original_journal_recovery_and_current_authority`
with `--exact --ignored --nocapture --test-threads=1`. The fixture creates its own
scoped credentials and trusted native configuration; no operator credential is
passed to Node. It never starts a provider or model.
