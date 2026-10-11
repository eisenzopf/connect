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
local worker inspection and stale-state qualification remain in G4-024; server
readiness alone does not attest those workers. Existing local host observations
and app-owned state can help operator repair, but never mint communication
authority. No permission dashboard or consumer interface is added here.

[Task access](TASK_ACCESS.md) · [Codex host](CODEX_TASK_HOST.md) ·
[Gate 4 implementation](connect-gate-4-implementation.html#G4-024)

[Local component evidence](evidence/gate-4/task-diagnostics-component.json) records
203 behavior checks and three documentation compilations at frozen source inputs.
It explicitly retains the two initial fixture failures and the open worker and
uncertainty qualifications; G4-024 is not complete.
