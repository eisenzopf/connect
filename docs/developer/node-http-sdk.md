# Explicit Node HTTP control

Use `CanonicalHttpClient` for finite current `connect/2` commands and `HttpClient`
for named application operations. Both retain their default loopback HTTP mode.
Select `remote-https` explicitly to use a configured HTTPS origin:

```ts
import { CanonicalHttpClient, HttpClient } from '@rudeless/connect';

const control = new CanonicalHttpClient({
  endpoint: 'https://connect.example.test',
  credential,
  transport: 'remote-https',
});
const application = new HttpClient('https://connect.example.test', credential, 4000, 'remote-https');
try {
  const frames = await control.request('operation.get',
    {operation_id: originalId}, {cid: conversationId}, {timeoutMs: 4000});
} finally {
  await control.close({timeoutMs: 4000});
}
```

The endpoint and credential are captured once. Remote mode requires HTTPS with
platform certificate and hostname validation. URLs with user information, query,
fragment or a non-root path, malformed/oversized URLs, and invalid bearer values
are refused locally. Configuration errors omit the URL and credential. There are
no redirects, credential refreshes, automatic mutation retries or transport
fallbacks. Keep the original logical mutation ID across uncertain outcomes and
query that original operation before choosing any explicit replay.

The canonical route remains `/v2/commands`; application operations retain
`/v1/operations`. Existing framing, scope/correlation checks, response limits,
structured refusals, absolute request deadlines and owned canonical close remain
unchanged. Finite journal reads use `conversation.subscribe` with `live:false`.
Live native subscription leases, native Stream custody and incoming invitations
use the managed native SDK rather than finite HTTP.

The current local component suite passes109 functions, including four HTTPS
functions repeated against the exact packed and independently installed
136-file distribution. TLS itself is real; those route handlers are synthetic.
They prove trust/hostname refusal before HTTP admission, original intent capture,
actual refusal status, no redirect/retry, whole-body deadline and cancellation.

The production Linux candidate `node-https/1` runs both EdDSA and ES256. An installed
Node consumer talks to the original isolated HTTPS application listener with its
fixture CA installed before process startup. It compares one-effect replay,
finite journal facts and named receipt reads with an independent signed native
SDK record, then requires current revocation to deny read/replay/new mutation and
receipt data. SQL requires one original effect and zero forbidden effects; the
original Node process, native owner and server must join and drain. Four-second
requests, fifteen-second phases, ninety-second functions and five-second drains
remain required. Candidate execution is pending. No named remote endpoint or
provider/model account was contacted, and this does not close G6-062.

The original production HTTPS fixture used the same default subject for its CA
and signed server leaf. Exact6d diagnostics locate both algorithm failures at
the first snapshot transport exchange. A local Node/OpenSSL reproduction rejects
that CA/leaf topology with `DEPTH_ZERO_SELF_SIGNED_CERT`; a distinct-subject leaf
succeeds. The fixture now names its root and server separately. Normal CA and
hostname verification stay enabled. All four local HTTPS functions now use a
CA-signed distinct server leaf, and the full109 component/type checks pass.
The corrected original production fixture still awaits Linux execution; this
local reproduction does not claim its success. See
[the source-bound TLS fixture evidence](../evidence/gate-6/node-https-ca-subjects.json).

The [exactc527 Linux run](../evidence/gate-6/node-native-c527.json) verifies that
both Node22/26 matrices now pass the original initial HTTPS snapshot, send,
operation-record read and replay under both signing algorithms with normal CA
verification. Both then fail an assertion at `https-journal`; the complete HTTPS
journey remains unqualified. The installed script is named `https-consume.mjs`,
while its catch code matched only the source filename, so that result has no
exact assertion location. The diagnostic now recognizes both names and emits only
bounded line/column numbers; four diagnostic/privacy tests pass. The corrected
canonical/Store causal witness and later native stages still await exact-source CI.

The consumer's attempted `HttpClient.operationAction()` full-record read was
excluded by the current application profile. It is removed. Public identity and
state use canonical `operation.get`; the independent Rust host verifies the
same owner's full Store causal link and an exact journal-row digest. The HTTPS
witness is now `node-https/2`, with its original4s/15s/90s budgets and all
current-authority/one-effect/drain requirements. Exact38 failures remain retained;
no whole native or HTTPS pass is inferred from component checks.


The exact2d Linux run on Node22 and26 is retained in
[node-native-2d.json](../evidence/gate-6/node-native-2d.json). Both HTTPS algorithms
reach send, original status/replay and the journal before a fixture assertion
fails at25:110. The application `ReceiptPage` has `receipts` and `next`; only
individual recipient facts carry Conversation/message IDs. The consumer now
asserts the actual empty page `{receipts: [], next: null}` instead of nonexistent
page-level resource fields. This source correction passes JavaScript syntax;
its original-server Linux result remains pending. Current authorization checks,
causal witness, singleton effect, request/phase deadlines and cleanup assertions
remain in the fixture. The five exact2d failures stay recorded as failures.
