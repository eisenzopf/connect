# Connected review app

The `connected-app` mode is a developer-owned application using the public Rust
SDK over UCTP/QUIC. It initiates a review of actual draft content and accepts
independently initiated submissions from configured contacts. It stores an
approved, rejected or changes-requested review decision and a received intake
ticket in its **own** SQLite database. It never opens Connect's database.

This is the local Gate 4 connected-app component, not a claim that Gate 4 or its
installed agent/artifact journey is complete. The source and failure/recovery
qualification are recorded in [the component audit](evidence/gate-4/connected-app.json).

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
canonical SHA-256 digest:

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
or a protected history-only MCP grant to read the corresponding server sources;
[task installation](TASK_ACCESS.md) describes the protected issuance CLI. The
actual MCP component test reads the business response with its journal provenance
using a narrow grant and the official MCP client.

The sample reconnects transport failures at a bounded interval with the original
configuration and IDs. Known rejections stop the workflow with retained state;
there is no credential fallback or automatic widening. Cancelling the outgoing
review preserves incoming work, including after restart. Each task lasts ten
minutes, so a queued effect cannot be renewed by inventing an ID.

Input bounds: policy 16 KiB, incoming body 8 KiB, draft body 4 KiB, title 128 bytes,
note 1 KiB, contact list 16, inbox page 100 events / 512 KiB, ledgers 10,000 entries,
reply lookup ten pages and consume-CAS reconciliation three attempts. Capacity
or lookup exhaustion preserves unfinished work for inspection. There is no
retention/compaction service for the app database in this component.

The one-effect claim is for this exclusively locked app database and Connect's
idempotent local communication operations. Cloned app databases, arbitrary
external business effects, publication, provider delivery and detached worker
supervision require their own contracts and are not qualified here. Test-only
crash controls are compiled only with `test-support`.
