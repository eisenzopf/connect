# Task diagnostics

[Regular agent installations](AGENT_ACCESS.md) use the same public task view.
A separate current credential for the same owner identity can inspect retained
work after model-key revocation; an administrator has no membership bypass.
Regular host sampling reports that installation's saved session/run state, not
per-task model progress. Optional narrow installations retain their own binding.

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
`grant-expired` means the protected installation is intact but its selected
grant needs owner-issued renewal before another turn. It reports unavailable;
inspection neither renews the grant nor resumes the host. Use `task-access
renew-codex` with a currently issued replacement that preserves the exact binding.

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
That earlier audit covers the original inspection views. The combined readiness
projection and changing-metadata case are recorded separately below. The
[dc73996 qualification](evidence/gate-4/code-recovery-dc73996.json) includes the
installed diagnostic path. Owner review reopened G4-024 to allow a separate,
current same-owner credential to read the retained task journal and source pages
after the original backing credential is revoked, expires or the task becomes
terminal. Current caller authentication still applies; this does not revive
continuation under the old credential or add a host-authentication probe.
Diagnostic fields and reason codes are unchanged. The
[numbered plan](connect-gate-4-implementation.html#G4-024) records the repaired read path qualified at 709cf82. Earlier reports remain proof of their
recorded version.

## Combined execution readiness

Both diagnostic commands now include `execution_readiness`, using the existing
readiness component shape. It combines the authenticated server snapshot with
only the explicitly selected local host or runner. The original
`server_readiness`, `codex_worker` and `runner` observations remain separate.
This is an operator-local view; it does not register an external worker with
the server, create a remote heartbeat, or turn local metadata into authority.

| Observation | Execution component | Meaning |
| --- | --- | --- |
| No worker selected | Unconfigured | No worker availability was established. A healthy server alone leaves combined execution readiness uncertain. |
| Selected but not started | Unavailable | Start the original configured worker. |
| Ownership lock held | Uncertain | Ownership was observed; model progress and provider delivery are unknown. |
| Runner stopped | Unavailable | Automatic continuation is not running. |
| Pending work or interrupted host | Unavailable | Reconcile the original worker/session and saved mutation IDs. |
| Host turn completed | Degraded | This host requires explicit resume; a completed turn does not complete the Connect task. |
| Metadata changed during inspection | Uncertain | Retry the sample; mixed session/run metadata is omitted. |
| Invalid installation, binding or state | Unavailable | Repair the selected installation; no credential fallback occurs. |

A server failure remains visible alongside worker reasons. A changed task revision
adds `task-changed-during-sample`; it does not silently present older journal pages
as one atomic snapshot. The readiness projection includes only stable reason
codes and component labels. It performs no communication or task mutation.

Rust integrations can compose their own trusted local observations with
`WorkerCondition::component` and `TaskDiagnostics::execution_readiness`.
`Readiness::from_components` provides the common conservative aggregation rule;
optional unconfigured components never mask a configured failure.

[Execution-readiness evidence](evidence/gate-4/worker-execution-readiness.json)
records 53 passing targeted local checks, including worker failure/recovery,
current-authority denial, unchanged checkpoints and changing host metadata.
Strict Clippy, Rustdoc and repository formatting pass. Earlier fixture startup
failures are retained; their original transient cause was not independently
established. Selected-host, installed-candidate and integrated acceptance remain
open, so this component does not complete G4-024 or Gate 4.

The [local task operating profile](TASK_OPERATING_PROFILE.md) measures actual
SDK admission at task, lease and reply capacity, with bounded paging, waiting
and restart checks. Its measurements qualify those specific limits; they do
not replace the worker readiness or installed-host checks above.

For historical binary migration, consistent task snapshots and interrupted-copy
recovery, see [local Gate 4 recovery](TASK_RECOVERY.md).

Current owner-review qualification: Runtime 709cf82 passes 42 offline phases, 17 installed checks and 32 actual MCP calls with no errors. Qualification controls d64f1b9 combine the reviewed fixtures at 7bdd534 with all 3 hosted jobs. Local macOS arm64 owner review only; Gate 5, community publication and remote deployment remain separate.

[offline-709cf82](evidence/gate-4/offline-709cf82.json), [package-709cf82](evidence/gate-4/package-709cf82.json), [real-model-agent-709cf82](evidence/gate-4/real-model-agent-709cf82.json), [review-remediation-709cf82](evidence/gate-4/review-remediation-709cf82.json), [qualification-controls-d64f1b9](evidence/gate-4/qualification-controls-d64f1b9.json)
