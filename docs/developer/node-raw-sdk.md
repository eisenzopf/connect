# Node raw canonical exchanges

`NodeClient.raw.validate(value)` returns an immutable, branded current envelope.
It calls `connect sdk-validate-envelope` synchronously with the captured JSON on
stdin. This local command loads no configuration, credentials, Store or client
and opens no endpoint. Its frozen Rust envelope parser is the same parser used
by the public Rust SDK. A schema-valid envelope can still be unsupported for
finite submission. Validation neither authenticates a signature nor grants
permission or media custody.

The matching trusted executable is required. Local validation is bounded to two
seconds, one MiB input and 16 KiB output; the original subprocess is joined before
return. Invalid schema is `validation/not-submitted`; unavailable validation is
`configuration/not-submitted`. Malformed validator output fences the original SDK
helper. There is no parser or transport fallback.

```ts
const checked = client.raw.validate({
  v: 2, id: 'saved-attempt', ts: '2026-10-28T00:00:00Z',
  type: 'operation.get', cid: savedConversationId,
  payload: {operation_id: savedOperationId},
});
const replies = await client.raw.request(checked, {timeoutMs: 4000});
for (const reply of replies) {
  // Original signed JSON, including optional fields; keep private.
  const signedEnvelope = reply.value;
}
```

`request` uses the client's original Rust SDK helper, queue, fixed configuration,
current authorization and cleanup. Rust revalidates and classifies the captured
intent, retaining logical mutation ID and scope while generating a fresh wire
ID, timestamp and signature. There is no automatic retry. A lost submitted
mutation keeps its original operation key and uncertainty. Returned frames have
passed Rust's full schema, signature and correlation checks; Node checks bounded
framing, capture, scope and signature shape without claiming separate JavaScript
cryptographic verification. Successful batches retain every correlated frame
and end in ACK. Refusals use `NodeSdkError`.

One envelope is at most one MiB, 32 JSON levels and 100,000 JSON nodes. The helper
wrapper permits nesting overhead and up to two MiB total response with at most
128 envelopes. Typed requests retain their existing 64 KiB limit. Returned
batches, tokens and JSON trees are frozen; structural token lookalikes are refused.
Auth/step-up, native negotiation, binary transfers and live event leases require
their dedicated lifecycle APIs. Rust supplies this classification; Node does not
maintain another command-policy table. Finite journal replay requires explicit
`live:false`.

The local synthetic suite passes 71 functions, seven covering this interface,
plus negative declarations and installed distribution equality. Actual frozen
vector checks and the installed native `node-controls/4` candidate are prepared
for Ubuntu CI. They cover local validation with invalid configuration, original
signed record reads, same-ID replay without another message, fresh wire IDs and
current revocation. These runtime results remain pending. See
[the evidence](../evidence/gate-6/node-raw-sdk.json); G6-062 remains in progress.

The first actual Rust cohorts (`02238a30`/`72e07ba9`) compiled the SDK/CLI, but
the new validator suite exposed stale embedded operation metadata constraints:
two local functions passed and the full-vector function failed. The older232
vector/schema copy accepted19 negative Connect vectors. A recorded240th upstream
patch synchronizes the exact254-vector contract in a fresh prepared tree. The
existing test remains required; current Rust/native execution is pending. See
[the schema comparison and retained failures](../evidence/gate-6/node-embedded-schema.json).
