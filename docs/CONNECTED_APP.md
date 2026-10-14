# Connected review app

The `connected-app` mode is a developer-owned application using the public Rust
SDK over UCTP/QUIC. It initiates a review of actual draft content and accepts
independently initiated submissions from configured contacts. It stores an
approved, rejected or changes-requested review decision and a received intake
ticket in its **own** SQLite database. It never opens Connect's database.

This is the local Gate 4 connected-app component, not a claim that Gate 4 or its
installed agent/artifact journey is complete. The source and failure/recovery
qualification are recorded in [the business audit](evidence/gate-4/connected-app.json)
and [the artifact exchange audit](evidence/gate-4/connected-app-artifacts.json).

## Start a fresh local example

Build with the repository's dependency and formatting conventions:

```sh
bash scripts/prepare-dependencies.sh
cargo build --locked -p connect-server -p connect-review-loop --bins
./target/debug/connect init --directory .local/connected-app --example connected-app --bind 127.0.0.1:4464
./target/debug/connect serve --config .local/connected-app/server.toml
```

In another terminal:

```sh
./target/debug/connect-example connected-app \
  --config .local/connected-app/reviews.toml \
  --policy .local/connected-app/connected-app.json \
  --state .local/connected-app/business.sqlite3
```

The preset creates three protected client configurations: `reviews.toml` owns
app tasks, `agent.toml` is the accepted reviewer/contact, and `outside.toml` can
reach the app but is denied by its business policy. The app has explicit task
control scopes and concrete recipients. It does not receive the installation's
operator credential, and its app credential must not be placed in a model's
environment. Existing installations and their credentials are preserved; this
preset applies only to a fresh directory.

Edit the developer policy before first use to select a reviewer, allowed
contacts and a real draft. Agents use their configured capabilities autonomously;
there are no per-action approval prompts. The original credential, endpoint,
trust material and policy are bound to the app state. Restart with matching
inputs; a deliberate policy or credential change requires an explicitly chosen
new app state rather than silently adopting the old ledger.

## Contact the app through the public SDK or JSON CLI

The app prints its outgoing task and conversation IDs. Read the review question
from the accepted participant's inbox; it includes actual draft text and its
canonical SHA-256 digest, plus an `artifact` object identifying the uploaded JSON
file. Before answering, fetch the file through `artifact_get` / the public SDK,
decode each base64 chunk, and verify its size and SHA-256 against that reference:

```sh
./target/debug/connect request --config .local/connected-app/agent.toml <<'JSON'
{"op":"inbox","cursor":null,"limit":100,"wait_ms":0}
JSON
```

Answer with `TaskCommand::Reply` / JSON `task` → `reply`, using the returned
question **message ID** as `reply_to`. The typed body is:

```json
{"action":"review_decision","draft_id":"draft-002","decision":"approved","note":"Ready for publication."}
```

For the preset, copy the outgoing question's `message.id` and
`message.conversation_id` from the inbox result into the placeholders below.
Read the artifact first; its ID, size and hash come from that question's JSON
body. These are public IDs, never the credential in a client configuration.
Use the selected installed executable in place of `./target/debug/connect`.

```sh
./target/debug/connect request --config .local/connected-app/agent.toml <<'JSON'
{"op":"artifact_get","conversation_id":"OUTGOING_CONVERSATION_ID","artifact_id":"ARTIFACT_ID_FROM_QUESTION","offset":0,"limit":32768}
JSON
```

Decode `data.data` from base64 and compare its byte count and SHA-256 with
`data.artifact.size` and `data.artifact.sha256` and the original question's
artifact reference. The example's JSON draft is bounded to 32 KiB; a larger
public artifact requires following chunk offsets until `eof` is true.
After reviewing those bytes, send the complete operation below. Replace the two
public IDs and use the actual draft ID if you edited the preset policy. The
`body` is a JSON **string** inside the operation, so its quotation marks are escaped.

```sh
./target/debug/connect request --config .local/connected-app/agent.toml \
  --task-address connect:reviews@local <<'JSON'
{"op":"task","request":{"task_profile":"connect-task/1","command":{"action":"reply","request_id":"review-approval-1","conversation_id":"OUTGOING_CONVERSATION_ID","reply_to":"QUESTION_MESSAGE_ID","body":"{\"action\":\"review_decision\",\"draft_id\":\"draft-002\",\"decision\":\"approved\",\"note\":\"Reviewed the delivered draft.\"}"}}}
JSON
```

`--task-address` negotiates the app's task profile using the same reviewer
credential; it does not issue authority. The reply resolves the task from the
attributed question; it does **not** take
an arbitrary task ID. Keep the complete request and `review-approval-1` unchanged
if an acknowledgement is lost. An accepted receipt is not yet the app's business
result: inspect the worker ledger and read the outgoing conversation history
with the operation shown below after the worker processes it.

`changes_requested` and `rejected` are the other supported decisions. A plain
`Send` containing identical text stays unassigned; its sender or draft ID cannot
select a task. Connect's authoritative reply record must match the original
question, task, conversation and journal event. The app consumes that attributed
reply before atomically committing its decision, checkpoint and response intent.

For independent contact, use `Create` with the app identity from `reviews.toml`,
then `Send` in that conversation with this typed body:

```json
{"action":"submit_draft","request_id":"submission-001","draft":{"id":"incoming-001","title":"Community update","body":"The actual draft to receive."}}
```

To perform that independent contact through the CLI, copy only `identity_id`
from `.local/connected-app/reviews.toml` into `APP_IDENTITY_ID`. Create a new
conversation as the allowed `agent` participant:

```sh
./target/debug/connect request --config .local/connected-app/agent.toml <<'JSON'
{"op":"create","request_id":"independent-intake-1","participants":["APP_IDENTITY_ID"]}
JSON
```

Copy the returned `data.id` into `INCOMING_CONVERSATION_ID`, then send the full
submission operation:

```sh
./target/debug/connect request --config .local/connected-app/agent.toml <<'JSON'
{"op":"send","request_id":"incoming-wire-1","conversation_id":"INCOMING_CONVERSATION_ID","body":"{\"action\":\"submit_draft\",\"request_id\":\"submission-001\",\"draft\":{\"id\":\"incoming-001\",\"title\":\"Community update\",\"body\":\"The actual draft to receive.\"}}"}
JSON
```

Read the independently initiated conversation to find the app's
`draft_received` response. Use the outgoing conversation ID instead to find the
review outcome. Wait for the worker to process the input, then repeat this
read-only request if its response is not present yet:

```sh
./target/debug/connect request --config .local/connected-app/agent.toml <<'JSON'
{"op":"history","conversation_id":"INCOMING_CONVERSATION_ID","after":null,"limit":100}
JSON
./target/debug/connect-example app-inspect --state .local/connected-app/business.sqlite3
```

For these fresh examples the history fits one page; otherwise follow `data.next`
as `after`. Retain the create, send and business request IDs across uncertainty.
After stopping only your worker and server with Ctrl-C, restart both with their
original commands and state paths, and repeat history/inspection. The original
approved review and received ticket must remain; replaying the saved exact
review reply must not create another business result.

The app uses the sender's conversation unchanged and creates its own task anchored
to the incoming journal event. Its response has `action: draft_received`, a ticket
ID, draft digest and `status: received`. This records receipt of the draft; it does
not claim the draft was published. A cancelled outgoing review does not terminate
the conversation or remove the app's incoming communication surface.

The submission's `request_id` is a business ID within sender + conversation,
separate from each transport mutation ID. Preserve both IDs across uncertainty.
A semantic duplicate produces one ticket and response. Changed content under
the same business ID is recorded as a conflict and cannot overwrite the original.
A different conversation is an explicitly different submission context.

## Submit a draft file

An independent participant can upload a JSON `Draft` with `ArtifactBegin`,
`ArtifactAppend` and `ArtifactFinish` in its conversation with the app, then send:

```json
{"action":"submit_artifact","request_id":"submission-002","artifact":{"id":"RETURNED_ARTIFACT_ID","size":123,"sha256":"RETURNED_SHA256"}}
```

Use the actual returned ID, byte count and digest. The app requires the file to
belong to the authenticated sender in that same conversation, with media type
`application/json`. It reads and hashes actual bytes before creating a ticket.
The private business record retains the artifact reference and parsed draft.
The response's draft digest uses canonical JSON; the source byte digest is also
retained and can differ when the sender used pretty-printed JSON.

Mismatched file references, malformed drafts and unavailable or deleted artifacts produce
visible `invalid_artifacts` / `unavailable_artifacts` inspection counts and no
business effect. Current authorization and retention are checked again on recovery;
the app does not reuse cached source bytes after deletion. If task creation was
accepted just before a crash, it first reconciles that original task intent; the
task record stays retained with its original expiry and no business effect. A deleted outgoing
file cannot produce a review decision, but the app remains reachable for incoming
submissions. An already-sent review question is never rewritten or re-uploaded.

New outgoing tasks include the artifact capabilities needed to publish their draft.
Older task records retain their original scope and inline question; restarting
never silently broadens their authority. Full old-binary upgrade qualification
remains part of G4-032.

## Recovery and inspection

Restart the app and local server using their retained state. App intents are saved
before dispatch; an unfinished inbox page is retained before handling it. Ticket
or decision, processed-event marker and response outbox commit in one SQLite
transaction. The server checks current authority on dispatch and replay. A later
tool observation may progress from pending to completed: the app validates it
against the same authoritative task/step/request/effect and retains the initial
receipt instead of overwriting it or issuing another mutation.

```sh
./target/debug/connect-example app-inspect --state .local/connected-app/business.sqlite3
```

Inspection returns review/ticket status, source event/task/conversation IDs,
digests and pending, waiting, uncertain, rejected, denied and conflict counts. It
omits credentials, draft bodies and review notes. It samples only the app-owned
database, not worker liveness or current Connect authority. Results are limited
to 100 with an explicit omitted count. Use authorized task diagnostics, history
or a protected history/file-read MCP grant to read the corresponding server sources;
[task installation](TASK_ACCESS.md) describes the protected issuance CLI. The
actual MCP component test reads the draft bytes and business response with its journal provenance
using a narrow grant and the official MCP client.

The sample reconnects transport failures at a bounded interval with the original
configuration and IDs. Known command rejections stop the workflow with retained state;
there is no credential fallback or automatic widening. Cancelling the outgoing
review preserves incoming work, including after restart. Each task lasts ten
minutes, so a queued effect cannot be renewed by inventing an ID.

Input bounds: policy 16 KiB, incoming body 8 KiB, draft body 4 KiB, serialized
artifact 32 KiB with 4 KiB chunks, title 128 bytes,
note 1 KiB, contact list 16, inbox page 100 events / 512 KiB, ledgers 10,000 entries,
reply lookup ten pages and consume-CAS reconciliation three attempts. Capacity
or lookup exhaustion preserves unfinished work for inspection. There is no
retention/compaction service for the app database in this component.

The one-effect claim is for this exclusively locked app database and Connect's
idempotent local communication operations. Cloned app databases, arbitrary
external business effects, publication, provider delivery and detached worker
supervision require their own contracts and are not qualified here. Test-only
crash controls are compiled only with `test-support`.

## Repeat the Gate 5 public-interface journey

After building normal binaries or selecting an installed candidate, run the
public CLI observer against those exact executables:

```sh
python3 scripts/check-connected-app.py \
  --source-revision RECORDED_SOURCE_REVISION \
  --server-binary /absolute/path/to/runtime/bin/connect \
  --example-binary /absolute/path/to/runtime/bin/connect-example \
  --output .local/gate5/app-journey-new-attempt.json
```

Use a fresh output path for each attempt. The runner creates isolated private
state, reads the delivered draft bytes, starts both outgoing review and incoming
submission flows, checks unauthorized and ambiguous input, rejects a concurrent
worker, and restarts the server and worker with the original business IDs and
checkpoint. Duplicate input and a late second decision must preserve the original
results and produce only one public response per business effect. It records
selected binary/tool hashes before and after execution and cleans up its owned
processes. It never opens Connect's database.

The separate `test-support` integration suites exercise lost upload, task,
consume and response acknowledgements, deleted source files and revoked app
credentials. Those controls are absent from normal distribution binaries.
[Gate 5 app evidence](evidence/gate-5/app-journeys.json) keeps these source and
selected-binary observations separate from final installed qualification.
Actual inference uses the [agent journey](AGENT_JOURNEY_QUALIFICATION.md);
independent fresh-user walkthroughs remain a separate acceptance requirement.
