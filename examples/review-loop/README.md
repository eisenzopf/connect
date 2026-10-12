# Independent native review app and deterministic agent

These two separate processes use only `connect-client` for communications.
They have no privileged server API or Connect-store dependency. The agent is a
fixed responder, not an LLM/Vapi integration. The example demonstrates a local
SQLite business action; exactly-once external effects require an application's
own idempotency/outbox design.

The separate [durable task runner](TASK_RUNNER.md) adds explicit task binding,
scoped invocation credentials, reply consumption, automatic reconnect and a
transactional business checkpoint through the public SDK. Use the `task` and
`task-inspect` commands for that workflow.

Build both binaries and initialize once from the Connect checkout:

```sh
bash scripts/prepare-dependencies.sh
cargo build --locked -p connect-server -p connect-review-loop --bins
./target/debug/connect init --directory .local/demo --example review-loop
```

Run these in three terminals, with the checkout as the working directory:

```sh
# Terminal 1
./target/debug/connect serve --config .local/demo/server.toml
# Terminal 2
./target/debug/connect-example reviews --config .local/demo/reviews.toml
# Terminal 3
./target/debug/connect-example agent --config .local/demo/agent.toml
```

Both identities initiate a distinct conversation. Reviews requests a review of
draft-002; agent asks which draft is waiting. Each recipient records its local
business action and queues a reply. Normal output reports readiness and safe
counts, not tokens or message bodies. Ctrl-C closes each physical client.

Stop both example workers before running the automated version:

```sh
./target/debug/connect demo verify --config .local/demo/operator.toml
```

The server stays running. This command launches both example processes for a
bounded interval, inspects their own checkpoints, and checks native history for
a request and peer reply in each independently initiated conversation. It never
opens the server database or stops an unrelated process. Repeat it with the same
work files to preserve operation IDs and deduplicate earlier effects.

App-owned work files default to `reviews.work.sqlite3` / `agent.work.sqlite3`
beside the corresponding client config, with private file permissions. Override
with `--state PATH` only for an intentional new or retained app workspace.
They contain saved command IDs/inputs/results, pending reply commands, processed
event IDs, local business actions and inbox cursor. Never replace these with the
server database. Losing this app state loses its own deduplication history.

`--run-for-ms 1200` runs an example for a bounded demonstration. Inspect safe
counts after a worker stops:

```sh
./target/debug/connect-example inspect --state .local/demo/agent.work.sqlite3
```

On disconnect or unknown command acceptance the example exits with an explicit
error and retains its pending command. Restart it with the same config and work
file after the server returns. It replays the original logical request, recovers
its saved receipt and resumes from its cursor. Stopping an identity does not
remove its durable server presence or waiting messages.

Process-level verification:

```sh
python3 scripts/check-examples.py
python3 scripts/check-example-recovery.py
```

The first proves independent initiation/replies, an offline draft-003 request,
server/example restart and business deduplication after deliberately replaying a
cursor in disposable app state. The second builds an explicit test-only binary
and abruptly exits before/after the app transaction or after server acknowledgment
before recording the local receipt. It resumes pending replies without repeating
the app action. Normal example packages exclude those fault controls.

## Two-way connected review application

The new `connected-app` mode records actual review decisions and independently
initiated draft intake tickets, with explicit reply binding and developer contact
policy. It uses its own durable ledger and the public SDK; the older deterministic
roles and pre-created task runner retain their existing contracts. See the
[connected app guide](../../docs/CONNECTED_APP.md) for the packaged local preset,
wire bodies, recovery, safe inspection and qualification limits.
