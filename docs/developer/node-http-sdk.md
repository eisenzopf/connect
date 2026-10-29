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
individual recipient facts carry Conversation/message IDs. At that checkpoint the consumer
assumed an empty page `{receipts: [], next: null}` instead of nonexistent
page-level resource fields. The shape-only correction passed syntax but remained
wrong about emptiness; exact70 and profile3 below record the queued-receipt fix. Current authorization checks,
causal witness, singleton effect, request/phase deadlines and cleanup assertions
remain in the fixture. The five exact2d failures stay recorded as failures.

The [exactf79 native matrix](../evidence/gate-6/node-native-f79.json) retains all
seven Node22 and eight Node26 failures. Node26 reaches the original Message fact,
helper restart and resumed fact, then fails another fixture assertion requiring
`causal_event_ids` in compact canonical `operation.updated`. The
[resumed witness correction](../evidence/gate-6/node-resumed-causal-witness.json)
keeps original ID/record/command/completion comparisons and makes the host verify
both original and resumed full causal links through authenticated Store reads,
plus one exact journal row for each. It does not introduce another application
route or invent a compact response field. The new native fixture profile is8;
its production runtime verification is pending.

Fixed-stage relative checkpoint timings now help distinguish a slow preceding
stage from an event wait. The upload schema permits only the signing algorithm,
known checkpoint, relative seconds and the existing15s deadline, with128 entries
maximum. Credentials, payloads, raw stacks and private paths remain excluded.
Local109 Node component/type functions,33 SQLite selection vectors and six privacy
functions pass, alongside JavaScript syntax and Rust formatting. Node22's original
event deadline, genuine candidate adoption and strict audio minimum failures
remain open; request4s, phase15s and whole-function90s limits are unchanged.

Candidate replacement also needs a fenced transfer of its original pending
native worker. The [cleanup prerequisite](../evidence/gate-6/native-candidate-cleanup-fence.json)
fixes a hazard before that transfer: old cleanup used `generation >= original`
and did not check its affected row count. It now requires the exact generation,
an absent or matching observed runtime, and exactly one projection update.
A mismatch rolls back the original operation/effect/journal changes as well.
Already-retired cleanup stays idempotent. Six isolated SQL/rollback vectors and
formatting pass; the required Store regression still needs Linux compilation
and execution. This does not complete candidate adoption. The remaining protocol
must retain old attempt attribution, retire/join its original worker, acquire
fresh replacement custody/resources and update the existing Connection ID under
an exact ownership fence. An unanswered offer supplies metadata, not readiness.

The [exact70 native matrix](../evidence/gate-6/node-native-70.json) retains six
Node22 and five Node26 failures. Both reach the resumed compact causal-field
assertion, which profile8 corrects through independent Store/journal witnesses.
Node22 also records read-only `transport-uncertain` at the revoked consultation
return read; a bare EOF is not classified as authentication. Candidate failures
remain. Both HTTPS algorithms still fail receipts: the earlier page-shape fix
kept an incorrect empty-page assumption. The Store queues a native journal fact
on acceptance. [Profile3](../evidence/gate-6/node-https-queued-receipt.json) instead
requires exactly one scoped queued receipt and independently verifies zero
`delivered`/`read` facts. Its installed consumer also runs from its external
project directory. Actual production TLS execution of this correction is pending.

The [cleanup follow-up](../evidence/gate-6/native-cleanup-terminal-recovery.json)
passes29 actual local Store native-wire functions with8,228 guarded inputs
unchanged. It keeps the live-successor fence while permitting the original
unobserved retirement after real Session end/leave, and prior-epoch interrupted
cleanup under an exclusive successor owner. The exact old mapping and attributed
absence remain required. Initial runs caught an invalid test generation rewind
and an omitted restart-recovery path; both are corrected with retained failed
logs.23 extracted SQL vectors also pass. Candidate worker custody/adoption still
needs implementation; these are Store results, not actual replacement media proof.

The [current local production HTTPS checks](../evidence/gate-6/node-https-production-local.json)
pass both EdDSA and ES256 with the installed external Node26 consumer. Each
checks the exact queued journal receipt, zero delivery/read claims, independent
original native and Store/journal cause, one original mutation, current revocation
and joined server/process ownership. All8,232 guarded inputs and the executable
stay unchanged; request4s/phase15s/function90s limits remain. This qualifies the
recorded macOS arm64 HTTPS subset; Linux and full SDK qualification remain open.
The [originala50 Linux artifacts](../evidence/gate-6/node-native-a50.json) stop at
the older cleanup test generation rewind before any installed SDK scene executes,
so they supply no result for current profiles8 or3.

The [current local native profile 8](../evidence/gate-6/node-native-production-local.json)
passes both signing algorithms on the original production executable with the
installed Node26 consumer. This includes the resumed event correction: full Store
causal links and exact singleton journal facts match both original and resumed
logical operations. Native/HTTP IDs and status, typed connection/session recovery,
finite/live journal behavior, membership changes, current refusals and all eight
helpers/six validators/two servers drain successfully. The run takes33.85seconds
with8,238 guarded inputs and the executable unchanged. Original4s/15s/90s limits
remain; no decoded-media or candidate-adoption qualification is inferred. Linux
qualification and the separate candidate/consultation scenes remain open.
