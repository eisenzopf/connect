# Durable task runner

This deterministic reference app uses `connect-client` over native UCTP/QUIC.
It asks for a draft response, consumes a specifically attributed reply, records
one local business action, and sends a follow-up in the same conversation. It
does not decide whether a draft is approved from arbitrary message text. The
Gate 3 `reviews`, `agent` and `inspect` commands remain available separately.

The application database belongs to this example. It never opens Connect's
server database. Its persisted binding is the explicit task, owner and
conversation; changed bindings, unrelated databases and concurrent writers to
the same app file are rejected.

## Protected setup

The developer first creates a conversation and a task through the public API.
Give that task `send`, `history`, `inbox` and `request_status` permissions and
explicit recipients. Its protected owner credential needs the task inspection,
claim, inbox and reply-consumption operations. The app does not create or infer
tasks from incoming text.

Issue a separate Native invocation grant through `connect task-access issue`,
then export it using `connect task-access export-native`. The grant permissions
are the same four task permissions above. Keep the owner/controller config and
exported grant config under ignored private `.local/` state. Never install the
controller credential in an assistant's tool environment.

After preparing those files and saving the explicit task ID:

```sh
bash scripts/prepare-dependencies.sh
cargo build --locked -p connect-review-loop --bin connect-example
./target/debug/connect-example task \
  --controller-config .local/task-demo/controller.toml \
  --invocation-config .local/task-demo/invocation.toml \
  --task-id "$CONNECT_DEMO_TASK_ID" \
  --state .local/task-demo/runner.sqlite3
```

Both configurations must describe the same identity, endpoint and TLS trust.
The invocation configuration must contain a Native task grant, and the
controller must contain a distinct backing credential. Both authenticate and
inspect the explicit task before opening app state. This runner cannot widen a
grant or use the controller as a communication fallback.

The reviewer answers through the public `TaskCommand::Reply` interface using
the received question's message ID. A plain message with matching words stays
unassigned and cannot advance this task. A provider adapter must supply its own
verified assignment; this example does not infer one.

## Recovery and transaction boundary

Every command's exact input and logical ID are saved before dispatch. Transport
loss reconnects with backoff from 100 ms to 2 seconds and the same fixed
configuration. Unknown acceptance preserves the original ID and CAS revision.
Expiry, revocation, cancellation and known policy rejections stop the runner with
its state intact. Resolve that condition explicitly before restarting.
Known application rejection codes are retained separately from unknown physical
acceptance.

A definitive `task-state-conflict` on reply consumption allows bounded protected
reconciliation: reread the current task, retain the rejected original command and
its code, and atomically save a new consume intent and active reference before
dispatch. Each pass permits at most three attempts. Communications and operations
with unknown acceptance keep their original intents; reconnect alone cannot rebase
them. Other conflicts retain state and require explicit resolution.

The question has one task-derived logical identity across workers. A bounded
server step claim coordinates startup, but the lease is not communication
authority. Losing a lease cannot invent a second question or authorize another
provider dispatch.

Fetched journal pages are saved before consumption. Each specific reply is
consumed through a persisted controller command before any local action. The
server's consume CAS excludes a competing worker. If a crash occurs after that
remote commit, this app resumes its saved page and replays its original consume
receipt under current authority. A rejection such as `reply-consumed` cannot be
treated as a successful consume.

The local SQLite transaction then commits the business action, handled event
IDs, follow-up command and journal checkpoint together. A crash before commit
rolls them all back; a crash after commit leaves the follow-up ready to recover.
Repeated pages cannot repeat the action or move the cursor backwards. Invalid
ordering, changed event identities and wrong task/conversation bindings fail.

A communication result marked pending or uncertain is saved as such. Its effect
handle remains available in the app outbox; it is not converted into another
dispatch. The example's business action is local SQLite work, not a guarantee of
exactly-once effects in an external system. Preserve one authoritative app store;
cloning it into independent active workspaces also clones its logical history.

Ctrl-C stops waits and exchanges. An interrupted exchange may have committed;
its pending intent remains for replay. `--run-for-ms` supplies a bounded run
instead. Safe inspection is available while stopped or running:

```sh
./target/debug/connect-example task-inspect \
  --state .local/task-demo/runner.sqlite3
```

Grants remain short-lived. The runner never refreshes them, loads a broader
credential, or rereads edited credentials during a reconnect. A protected
developer must explicitly issue a new grant under current task authority and
restart with the matching config and retained app state. Media ending does not
complete or cancel the task.

## Local qualification

```sh
cargo test --locked -p connect-review-loop --all-features
```

The explicit `test-support` build includes process exits at dispatch, ACK,
consume and app commit boundaries. Normal builds exclude those controls. The
tests launch independent example processes, use public native peers, restart the
server and runner at each crash boundary, and exercise competing workers and current grant
revocation. They use disposable local state and no provider accounts. Evidence
and remaining Gate 4 requirements belong in the numbered implementation plan;
these tests alone do not close the gate.

The provider uncertainty fixture uses the pinned private Vapi SDK with a
loopback service that records one acceptance and withholds its HTTP receipt.
The app exits after Connect acknowledges the question but before saving that
outcome. The provider worker records uncertainty; both server and app restart
with the original task, grant and state. The app saves the actual uncertain
effect, and a second app restart neither repeats the send nor treats it as a
review response. The fixture inspects only the application's own outbox and
uses public clients for Connect's jobs, history and effect receipt. This proves
recovery in that controlled case, not live Vapi support or external exactly-once
delivery.
