# Codex task host

Connect gives an installed Codex CLI a persistent communication task through
MCP, a supplied skill and a packaged host adapter. The developer configures the
permitted operations and recipients once. The assistant then uses those tools
without per-action approval prompts.

The tested host is `codex-cli 0.162.0-alpha.17.2` on local macOS arm64. The
[actual-model draft journey](evidence/gate-4/real-model-draft-7edb1b1.json) and
[current incoming-contact journey](evidence/gate-4/real-model-incoming-7edb1b1.json)
record useful model behavior separately from scripted runtime tests. Each report
identifies its tested source; the [implementation plan](connect-gate-4-implementation.html)
tracks final candidate qualification.

## Install and start

1. Install the local Connect candidate and initialize isolated state. Keep state
   outside the release directory. See [local release](LOCAL_RELEASE.md).
2. Authenticate the selected Codex CLI with your chosen account. Connect inherits
   the account location; it does not copy authentication into task exports.
3. Create a task in an existing conversation through the public SDK or JSON CLI.
   Configure its operations, concrete recipients and expiry. Follow
   [task access](TASK_ACCESS.md) to register the exact host binding and issue a
   bounded HostSession grant.
4. Use `connect task-access export-codex` to install the protected grant,
   isolated MCP configuration and Connect skill. Preserve the completed export.
5. Start one bounded turn with a private UTF-8 prompt file:

```sh
connect task-access run-codex --installation .local/codex-review \
  --prompt-file .local/review-prompt.txt
```

The launcher records the task, installation digest, original prompt and run
intent before spawning. It saves the exact Codex session UUID as soon as that
identifier arrives. A completed host turn does not complete the Connect task.
Two launchers cannot use the same installation concurrently.

The MCP configuration uses `bearer_token_env_var`, `required=true`, ten-second
startup/tool deadlines and `default_tools_approval_mode="approve"` for the
Connect server. The host supports these documented
[MCP settings](https://learn.chatgpt.com/docs/extend/mcp?surface=cli).
Connect still checks current task authority on every operation. The launcher
uses isolated configuration and passes the bearer outside model tool arguments.
An intact older export without the approval field remains usable; the launcher
applies that policy without silently editing its saved assets.

## Continue the same task

Once a durable reply arrives, explicitly resume the installation:

```sh
connect task-access run-codex --installation .local/codex-review
```

This selects its saved session UUID using the documented
[noninteractive resume interface](https://learn.chatgpt.com/docs/non-interactive-mode).
It never selects `--last`, creates a replacement conversation or asks the user
to retype context. The task journal recovers missed events; notifications are
only hints. This adapter supplies explicit continuation, not an automatically
waking daemon. A developer may schedule the same bounded invocation.

Two tasks can share authorized conversation history while retaining separate
questions, grants and effects. For an attributed follow-up, `connect_send.reply_to`
is the received **conversation journal event ID**. In `connect_task_wait`, use
`data.events[*].data.data.event_id` for a `reply` whose nested `task_id` matches.
The outer task-event ID, message ID, source ID and question ID are different.
For an independent incoming task, its `initiating_event_id` supplies that
reference. Source IDs belong in message citations, not reply-routing arguments.

The [agent-presence example](../examples/agent-presence/README.md) accepts an
independent contact, creates a source-bound task and invokes this host through
public interfaces. Each invocation accepts one configured contact. Application
checkpoint replay does not create another model turn or repeat a completed
communication effect.

## Credential refresh and failure behavior

The owner/application issues a new bounded grant using current backing
authority, then adopts it through the protected adapter:

```sh
connect task-access renew-codex --installation .local/codex-review \
  --grant-file .local/renewed-task-grant.json
connect task-access run-codex --installation .local/codex-review
```

Renewal requires the original task, invocation binding and operation list. It
locks the installation and atomically replaces a private credential pointer;
it preserves the exported assets, session UUID and interrupted run intent.
`renewed-not-invoked` confirms local adoption, not successful server admission
or delivery. The original grant ID remains installation identity; each MCP
request and effect uses current selected authority. The model cannot mint a
grant. Task cancellation, expiry, lost membership and current revocation still
deny execution and replay.

Retain logical request IDs and immutable inputs after uncertainty. Reconcile
through status, the effect ledger and journal before retrying the original
operation. A new request ID would be new work. Missing session state, changed
assets, a mismatched renewal or an active host lock is a repair error. No
backing credential fallback is allowed. A transient model error can leave a
turn unconfirmed even when tools completed; inspect the retained session and
Connect receipts, then continue that same session.

Inspect without launching a model:

```sh
connect task-diagnostics --config .local/native-task.toml --task-id TASK_ID \
  --codex-installation .local/codex-review
```

Diagnostics require separate current Native/owner read authority. They report
retained correlation and turn state, not prompts, bearer secrets or raw logs.
See [task diagnostics](TASK_DIAGNOSTICS.md).

## Bounds and process ownership

`--timeout-seconds` accepts 1–300 seconds, default 120, independently of each
MCP request budget. Private JSONL output is bounded to 4 MiB per attempt and
512 KiB per event; the installed grant secret is redacted. The deadline includes
setup, spawn and observation. Errors preserve the original communication intent.

Each Unix turn owns a process group. On macOS, the launcher signals that group
before reaping its leader, preventing group-ID reuse during cleanup. Timeout,
stream failure and cancellation use the same ownership. A bounded native group
snapshot permits Darwin's zombie-only signalling exception only when the
unreaped leader is present and every member is a zombie. Live members or failed
inspection keep cleanup unsuccessful. The
[cleanup audit](evidence/gate-4/codex-process-cleanup.json) retains its negative
cases and earlier failures.

Deliberately detached sessions and a launcher killed without destructors require
external supervision. These checks qualify local macOS arm64; they do not
promise arbitrary process-tree containment or portability.

## Reproduce the separate evidence layers

The account-free runner uses the actual installed CLI with scripted loopback
Responses and isolated account roots. It tests skill discovery/injection, saved
call intent before dispatch, interruption, exact-session replay, two-task
isolation, artifact integrity and current revocation. It performs no inference:

```sh
bash scripts/check-gate-4.sh --codex-binary /absolute/path/to/codex
```

The ordinary workspace suite deliberately ignores external-host checks. The
complete runner selects and executes them, and fails if the supplied host is
missing. The [runtime audit](evidence/gate-4/codex-owned-runtime.json),
[draft audit](evidence/gate-4/codex-draft-journey.json) and
[host concurrency profile](evidence/gate-4/host-concurrency.json) record their
measured boundaries. Real-model reports above establish useful follow-through
at their recorded versions; neither layer substitutes for the other. Power
loss, arbitrary host storage failure and universal model instruction-following
are outside these claims.
