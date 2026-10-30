# External decision worker

G6-073 is in progress. This directory is application-owned business logic;
Connect remains responsible for its own journal and current authorization.

The private WorkerState journal uses a fixed decision-worker.sqlite3 filename in
an explicit ignored .local directory. It never accepts a Connect database path.
One live process owns the journal; a different worker/conversation or unrelated
database profile refuses. A real process exit releases SQLite's physical locks
while retaining pending model/Connect intent for the next owner. No database
transaction spans an external await. A revision assertion prevents stale writes.

State is bounded JSON in a private0600 database. Restart reads the exact original
pending operation/intent; it does not turn unknown into success or create a new
mutation ID. Journal facts and current authorization must still come from the
public Connect SDK when the worker resumes.

The current implementation uses the locally qualified Node26.7 API, documented
in the [versioned Node SQLite reference](https://nodejs.org/download/release/v26.7.0/docs/api/sqlite.html).
The state tests use only isolated application files and a real child process.
They launch no model, provider, telephone or Connect communication.

Run:

    node --test examples/parley-on-connect/worker/*.test.mjs

The pure policy accepts only configured action names and bounded text. Model
resource/approval/role/execution fields refuse. Current source/session gates, exact
owner/epoch/proposal/scope decisions and stable logical action slots are checked
before a prepared intent can reach the executor. The approval reader belongs to
the authenticated application; incoming content is never that reader.

`DecisionWorker` serializes authorized event references into this application
journal. It re-reads each fact through the public SDK before using its content,
including on restart. It saves a bounded model request before `generate`, and
uses only `recover` for a pending request. A completed answer passes the pure
policy and fresh application decision checks before any mutation. Model input
is capped at32KiB, text at4096bytes, and the business plan at32 model proposals.
Unknown model requests and unknown Connect operations remain pending.

`ConnectWorkerGateway` accepts the public `NodeClient`; it has a closed switch
over message, invitation, end, consultation, return, handoff and device movement
methods. It prepares an SDK-validated original reference without network IO,
then submits with the persisted operation ID. Recovery restores that original
reference and reads status. Session context uses current public snapshots;
`sessionRevision` in the worker context is the SDK topology revision used by
these Session mutations. An incoming event cannot select a resource, revision,
operation ID, application decision reader or provider API.

The application supplies reviewed action bindings (`stage`, `slot`, `command`,
`scope`, pure `buildInput` and required business gates), a fresh `context(plan)`
reader, an authenticated `decision(intent)` reader, and `allowAdvance` for its
business graph. These are trusted code dependencies, never model-generated
objects. `intentDigest` identifies the exact prepared action displayed to its
approver; model output and incoming message flags do not create that decision.
App-owned speech completion must come from the actual owned source lifecycle.
SDK operation completion alone never advances the business stage or proves
delivery, played speech, decoded media or the complete business outcome.

Typical application composition (after provisioning and authentication):

```js
const state = await WorkerState.open({ directory, workerId, conversationId,
    initial: initialPlan({ planId, planVersion, stage }) });
const sdk = new ConnectWorkerGateway({ client, conversationId });
const worker = new DecisionWorker({ state, sdk, app, model, bindings });
try {
    await worker.run(sdk.events(worker.snapshot().cursor), { maximumEvents: 64 });
    // An app-owned speech/approval update can call worker.wake().
    // Explicit app business transitions use worker.advance(next, evidence).
} finally {
    state.close();
    await client.close({ timeoutMs: 5000 });
}
```

The current36 worker checks cover repeated facts, invalid actions, current
approval/session/speech gates, uncertain model and Connect calls, restart and
real process exit. The public Node SDK tests run its actual typed methods and
reference validators with an explicitly synthetic helper; they do not qualify
a native server, real media, Linux, remote hosts or actual-model execution.
The existing19 Parley fixture checks also pass (55 combined).

Remaining G6-073 work: a concrete scenario/model adapter, native integration
and a separately authorized actual-model rehearsal. Existing G6-065 model
execution authorization has already been consumed. See the source-bound
[increment evidence](../../../docs/evidence/gate-6/decision-worker-initial.json).
