# Task diagnostics

The native public SDK and `connect task-diagnostics` sample the existing task
interfaces. They add no transport, task store or administrative authority.

```sh
connect task-diagnostics --config .local/native-task.toml --task-id TASK_ID
```

Use a currently authorized task-owner configuration or an explicitly exported
Native task grant. The command never reads the server database or falls back
to another credential. HostSession and provider MCP grants retain their own
adapters; they cannot be converted into Native authority. Missing optional
permission appears as `availability: unavailable` with a stable code.
Authentication, transport, invalid-cursor and protocol errors fail the sample.

Rust applications use the same convenience:

```rust,no_run
# async fn inspect(client: &mut connect_client::ConnectClient, task: &str) -> connect_client::Result<()> {
use connect_client::task_diagnostics::DiagnosticCursors;
let report = client.task_diagnostics(task, &DiagnosticCursors::default(), 100).await?;
assert_eq!(report.task.task_id, task);
# Ok(()) }
```

Each of the three paged views has its own `after` and `next` checkpoint:

- `journal`: task states, effect identities/states, attributed replies and step
  leases. This is catch-up from the authoritative journal, even without a wake.
- `sources`: authorized source IDs, actor attribution, provenance and availability.
  The body is omitted, including message text and artifact/session metadata.
- `unassigned`: incoming reply references with no task assignment, when the
  caller's current conversation authority permits that owner view. A Native
  grant cannot acquire this administrative read from another credential.

Continue a particular view with its matching option:

```sh
connect task-diagnostics --config .local/native-task.toml --task-id TASK_ID \
  --after TASK_JOURNAL_CURSOR --sources-after RECALL_CURSOR --limit 25
```

`--unassigned-after` controls the separate owner reply view. A cursor from another
task, reader or view fails; it never selects new authority. Limit is 1–100 and
the server's existing page/text bounds still apply. The report is one sampled
page per view, not global counts. Recall's `omitted` preserves the server's
explicit unread extent; `null` in other pages means that extent is unknown.
Filtering assigned replies can produce an empty unassigned page with an
advanced cursor. Neither an empty page nor a last returned cursor proves task
completion or exhaustion of the conversation.

The SDK makes six bounded public reads and rechecks task authority last. They
are separate observations, not one atomic database snapshot. If task revision
changes during sampling, `changed_during_sample` is true. Each request enforces
current access; a revoked grant cannot return a cached diagnostic report.
No diagnostic read consumes a reply, acknowledges app work, changes a runner
checkpoint or initiates communication. Persist processed events and checkpoints
only in the application's own transaction after processing.

An effect observation retains its task/step/request identity and journal state,
but omits outcomes and arbitrary provider errors. It describes the recorded
event, which may precede a later reconciliation. Inspect the same effect through
authorized `EffectGet` / `connect_effect_status` before deciding whether to retry.
Uncertainty requires reconciliation of the original IDs. Waiting is a task
state; a completed tool request or host turn does not complete the task.

The `server_readiness` section is authenticated process health, not provider
delivery or remote assistant availability. For the main MCP endpoint,
`configured: false` and `reason: not-configured` mean optional configuration is
absent; that component does not degrade the aggregate. Old configured component
observations retain their wire shape and deserialize with `configured: true`.
An enabled listener reports bound, draining, stopped or worker-exited through
supervision. Cancelling shutdown retains abort ownership; it cannot detach a
listener and leave a ready observation.

External Codex sessions and reference runners are separate processes. Their
local inspection is described below; server readiness alone does not attest
those workers. Protected metadata and app-owned state help operator repair,
but never mint communication authority. No permission dashboard or consumer
interface is added here.

[Task access](TASK_ACCESS.md) · [Codex host](CODEX_TASK_HOST.md) ·
[Gate 4 implementation](connect-gate-4-implementation.html#G4-024)

The earlier 34a9d1f [component checkpoint](evidence/gate-4/task-diagnostics-component.json) records
203 behavior checks and three documentation compilations at frozen source inputs.
It explicitly retains the two initial fixture failures and the open worker and
uncertainty qualifications; G4-024 is not complete.

## Local host and runner observations

Inspect one explicitly selected Codex installation alongside the task:

```sh
connect task-diagnostics --config .local/native-task.toml --task-id TASK_ID \
  --codex-installation .local/codex-task
```

`codex_worker` reports `unconfigured` when no installation is selected. A valid
export without saved adapter state reports `not-started`. A held adapter lock
reports `turn-active`; it does not prove the child model is healthy. A completed
host turn reports `turn-complete` even when the task is still waiting. After a
stopped incomplete turn, `interrupted` retains the exact session/run IDs;
`missing-session` requires repair of the original intent. Binding, private-file,
asset and lock failures are explicit. `changed-during-sample` asks the caller to
retry inspection when retained state changes around the lock probe.

The inspector validates the existing protected installation and retained binding,
reads bounded private metadata, and omits prompts, bearer secrets, logs and
session contents. A successful probe releases its shared lock immediately.
The CLI rechecks the authenticated task after local sampling before printing;
its final task state/revision are updated and concurrent revision changes remain
explicit. Metadata observations confer no authority or permission to replay work.

The reference app provides a separate SDK client view:

```sh
connect-example task-diagnostics --config .local/invocation.toml \
  --task-id TASK_ID --state .local/runner.sqlite3 --limit 100
```

It authenticates the task before inspecting its own private database and checks
current authority again through the SDK after the snapshot. The stored owner,
task and conversation must match. Inspection opens SQLite read-only, does not
create/migrate state, and keeps commands, outcomes and text out of the report.
The checkpoint sequence must agree with committed app events. SQLite work runs
on a blocking worker rather than the async I/O thread.

`lock-held` is an ownership hint. `stopped-with-pending-work` means retained
commands or a staged page need reconciliation; it does not dispatch them.
`stopped` does not mean the task is completed. `not-started` means no selected
state file was found, not proof that no earlier app state existed elsewhere.
A saved checkpoint may lag a new bound reply in the task journal. Resume only
the original configured runner/state; diagnostic reads never acknowledge that
reply or replace its task. These local observations and remote pages are sampled
separately and can change after inspection.

[Worker inspection evidence](evidence/gate-4/worker-diagnostics.json) records 24
local behavior checks, strict quality checks and the retained fixture failure.
Selected-host qualification, full external-worker supervision/readiness audit
and the changing-metadata branch remain open; G4-024 is still in progress.
