# External decision worker

G6-073 is complete for its recorded local implementation scope. This directory is application-owned business logic;
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

The initial36 worker checks cover repeated facts, invalid actions, current
approval/session/speech gates, uncertain model and Connect calls, restart and
real process exit. The public Node SDK tests run its actual typed methods and
reference validators with an explicitly synthetic helper; they do not qualify
a native server, real media, Linux, remote hosts or actual-model execution.
The existing19 Parley fixture checks also pass (55 combined).

The earlier component increments remain recorded in the source-bound
[increment evidence](../../../docs/evidence/gate-6/decision-worker-initial.json).
The completed authorized model and original-response recovery are recorded in
[actual-model evidence](../../../docs/evidence/gate-6/decision-worker-model-authorized.json).

`RecordedProposalModel` now supplies the durable model-response boundary. It
fsyncs an immutable original request before calling a trusted model callback,
then saves its response independently of the worker's journal. `recover()` reads
that original response or a trusted read-only observer of the original request. An unknown request, partial file or changed
request/source binding never invokes a replacement. `scriptedModel` exercises
this same path with explicitly simulated, application-configured proposals.
It owns no Connect client or communication credential. An external callback's
actual model execution still requires its separately reviewed configuration
and explicit authorization; this class grants none.

Six additional model-boundary checks pass, bringing the combined local suite
to61 (42 worker plus19 existing fixtures). They include concurrent callers,
lost response, source substitution and a real process exit after response fsync.
See [model recovery evidence](../../../docs/evidence/gate-6/decision-worker-model-recovery.json).

The engine also retains its last committed journal revision across external
awaits. An unexpected write by another application component refuses with
`stale-business-state` before submission, preserving that component's state.
This regression brings the current combined suite to62 (43 worker/19 existing).

The native lane now runs the public Node SDK against a freshly initialized,
source-bound Connect executable and two isolated signed identities. Real journal
attribution binds an exact owner decision; a controlled response loss after SDK
submission recovers the original status after reopening the worker journal.
Current membership removal refuses replay. Server and helpers join and the
original listener is reclaimed. Controlled replay snapshots of the retained
answer reject an invalid action, speech still playing and stale Session
metadata without another model turn or Connect effect. These are explicit
metadata fault injections, not native Session/media qualification.

The native runs exposed two assumptions absent from the synthetic helper:
opaque cursors are resealed on replay, and optional SDK JS properties may be
undefined. Correlation now uses stable event/conversation identity; JSON model
context omits absent properties and opaque resume tokens. Consumers close their
owned reader before reauthorizing a fact. Cached answers pass policy again on
restart. All72 combined component checks pass (53 worker/19 existing).

Run the isolated native lane explicitly with a verified current source binary:

```sh
python3 examples/parley-on-connect/worker/native-rehearsal.py \
  --executable /absolute/source-target/debug/connect \
  --expected-sha256 REVIEWED_SOURCE_BINARY_SHA256 \
  --output /absolute/checkout/.local/fresh-native-worker-run
```

The actual model callback is prepared and disabled by default. Its pinned Codex
host receives only the protected proposal request; it receives no Connect or
provider credential/tool configuration. An exclusive authorization-consumption
record prevents another invocation with the same reviewed configuration, even
for a different request. Recovery only reads retained output; unknown remains
pending. See the [bounded rehearsal review](../../../docs/developer/decision-worker-rehearsal.md)
before authorizing this separate action. Default npm and native runs start no
model/account/provider call.

The owner separately authorized one 60-second pinned model proposal rehearsal.
That single turn completed, but its initial audit rejected two nonfatal startup
diagnostics. The failed run remains retained. A read-only observer recovered
that exact completed trace without starting, probing or resuming a model. A new
worker process resumed the original private app and Connect journals, submitted
one exactly approved proposal and recovered its lost SDK reply. Current membership
withdrawal refused replay. All76 component checks pass (57 worker/19 existing).

The auditor permits only the two exact pinned diagnostics before the turn, once
each; unknown, duplicate and post-start errors and all tool items still refuse.
The original host PID/exit/forced-cleanup metadata was missing before its failed
audit and remains explicitly unknown. Future launches persist it before audit.

`model-host.py --recover --config PRIVATE_CONFIG --request ORIGINAL_REQUEST`
checks the retained request, configuration, launch, authorization consumption
and completed JSONL. It performs no host invocation. `recover-native-rehearsal.py`
can resume one retained isolated native fixture with the original executable:

```sh
python3 examples/parley-on-connect/worker/recover-native-rehearsal.py \
  --original /absolute/checkout/.local/original-native-worker-run \
  --executable /absolute/source-target/debug/connect \
  --expected-sha256 ORIGINAL_SOURCE_BINARY_SHA256
```

It refuses an existing recovery directory; it never resets pending state or
consumes a second model authorization. A failed or unknown original remains
pending for inspection. This local implementation result does not qualify
physical speech, whole-app continuity, providers or Gate6 acceptance.
