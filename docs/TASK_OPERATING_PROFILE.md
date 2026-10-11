# Local task operating profile

Gate 4's [frozen limits](decisions/009-task-runtime.md#recall-and-communication-limits)
are admission ceilings, not a throughput promise. The executable profile below
measures a separate server through the public Python SDK, its persistent Rust
helper and native UCTP/QUIC. It creates real records; it does not seed SQLite.
The [recorded results](evidence/gate-4/task-operating-profile.json) cover this
component of [G4-030](connect-gate-4-implementation.html#G4-030), which remains
in progress.

## Run the profile

On local macOS arm64, with Python 3.11 or newer, from the repository root:

```sh
bash scripts/prepare-dependencies.sh
cargo build --locked -p connect-server --bin connect
python3 scripts/check-task-capacity.py
```

Use `--binary /absolute/path/to/connect` to select a separately built binary.
The script does not build implicitly. Each run creates a fresh private
`.local/task-capacity/<timestamp>/` installation, disposable identities,
credentials, database, server logs and `result.json`. No provider account,
public ingress, existing installation or production database is used. The
900-second budget includes initialization and restart. Interruptions fail the
profile and unwind owned server/helper processes. Preserve failed reports;
do not restart an uncertain operation with a new logical ID.

## What is exercised

| Phase | Behavior required to pass |
|---|---|
| Active tasks | Admit 1,024; deny the next creation; replay the original creation receipt unchanged. |
| Step claims | Hold 16 live leases on distinct tasks; deny a seventeenth; release one and reuse that slot with the same previously rejected input. These are simultaneous leases, not 16 runner processes. |
| Reply inbox | Admit 1,000 explicitly bound replies; deny the next without appending an event; replay a reply unchanged. Consume one, retry the rejected input and verify journal history retains all 1,001 admitted replies. |
| Paging and waiting | Page the journal within 100 events and 32 KiB per projection; reject limit 101. An empty native wait requests 10 seconds and must finish between 9.5 and 12 seconds, including local response overhead. |
| Retained tasks | Create 10,000 total records through SDK operations; deny the next even after cancellation. The original creation receipt and task cancellation remain usable at capacity. |
| Restart | Stop the owned server, reopen the same state, verify three boundary records and their conversation, and confirm retained-capacity denial persists. |

The reply phase starts with a 16 KiB question. Byte bounds may produce fewer
than 100 events per page; the report records the actual maximum. Consuming a
reply releases inbox capacity without deleting conversation or task history.

## Interpret the measurements

Each phase records elapsed time, instrumented task exchanges and their p50,
p95 and maximum latency. These are serial end-to-end SDK calls, including the
Python/Rust helper, loopback transport, authorization, store and response work.
They are not QUIC network latency, a provider benchmark or a concurrency SLO.
The reply phase includes its intentional 10-second empty wait. Initialization,
conversation creation and negotiation are outside the exchange histogram.

RSS is sampled every half second for the owned server PID only. It excludes
the Python driver and native helpers, and can miss short-lived peaks. A phase
with no sample reports `null`, not zero. Database sizes are apparent bytes;
phase totals include the SQLite WAL when present, while the final report shows
files after shutdown. Source and binary digests must remain unchanged during
the run. Credentials, generated IDs, raw logs and unfinished command inputs
stay in the private run directory; public evidence contains sanitized results.

## Retired voice setups: source review and remaining measurement

The shared 32-listener ceiling bounds open task MCP endpoints; it is not a
ceiling on retired setup records. After a call ends or its setup expires,
`VapiMedia` retains the closed setup for that session. This prevents a later
bind from falling back to an ordinary call without the original task tools,
and prevents another setup from replacing the original recovery intent.

Source review at `493d5c3` found a separate bound: protected setup requires an
existing authorized durable session, the in-memory map accepts one setup per
session, and session creation checks the store's configured `sessions` limit
(10,000 by default). Closed setups retain the directory path, service reference
and expiry; their listener is removed during close. They are drained when the
server shuts down. This is a count-bound argument, not measured retained memory
or a claim that 10,000 simultaneous endpoints can run.

The relevant implementation is [setup authorization](../crates/connect-store/src/tasks/media_setup.rs),
[session admission](../crates/connect-store/src/sessions.rs),
[setup retirement](../crates/connect-server/src/vapi_media.rs) and
[listener ownership](../crates/connect-server/src/vapi_bootstrap/media.rs).
The existing expiry check verifies that the endpoint closes, private intent and
durable state survive, and replacement is rejected. G4-030 still needs a
repeated retirement profile with memory observations and capacity/restart
behavior. Do not remove closed markers simply to reduce the map: any replacement
must preserve the no-fallback and original-intent guarantees.

## Sixteen concurrent assistant hosts

The separate [host-concurrency result](evidence/gate-4/host-concurrency.json)
qualifies sixteen actual Codex CLI processes against one Connect server. It
uses the shipped protected host composer, native Python SDK and a recorded
release binary. The Responses endpoints are scripted loopback fixtures; no
model account or inference is involved.

```sh
python3 scripts/check-host-capacity.py \
  --binary /absolute/path/to/connect \
  --codex-binary /absolute/path/to/codex
```

This explicit profile has a 480-second budget and keeps private state and logs
under `.local/host-capacity/`. The complete Gate 4 runner also selects it using
its explicit Codex configuration; source-only runs omit it. It requires the
source archive's qualification fixtures as well as the application and SDK.

All sixteen hosts must reach the first Responses request together. A one-second
gate permits two process observations before allowing actual MCP calls. Each
host reads its task and initiating source, then sends one attributed reply.
The verifier checks the actual MCP event results and public conversation history.
After server restart, sixteen fresh launchers resume their exact saved sessions
and return the original accepted receipts. Another application checkpoint replay
must create no host turn. All observed host process groups must exit.

The qualified run took 6.083 seconds for cold setup/work and 3.215 seconds for
warm resume/work; the full driver, including teardown, took 14.057 seconds.
Sampled server RSS peaked at 36.2 MiB. The gated sixteen-process Codex main-process
RSS totals were about 1.60 GiB cold and 1.63 GiB warm. Those host samples exclude
child tools; they are neither peak total memory nor a resource guarantee.
Other offline checks ran on this machine during measurement. Command timings
include the deliberate gate and local initialization, not real-model latency.

Sixteen is the measured assistant-host count, **not a new host admission limit**.
It is separate from the sixteen simultaneous reference-runner step leases above.
The profile does not prove model reasoning or sixteen reference-runner processes.
Six negative-control/positive-control verifier tests reject completed prose with
missing or failed tools, wrong bindings, changed warm receipts and surviving
process groups.

## Work still required

Remaining operating work includes actual reference-runner process concurrency,
complete MCP payload/deadline loads, grant refresh and expiry under sustained
load, age-based retention and retained voice-setup metadata. The
[installed local candidate](evidence/gate-4/package-candidate.json) separately
passes its package checks; real-model journeys, whole-item dependencies and
full Gate 4 acceptance remain open. The separate
[shared MCP capacity audit](evidence/gate-4/mcp-shared-capacity.json) retains
its own request/listener admission and lifecycle evidence.

For recovery, use [task diagnostics](TASK_DIAGNOSTICS.md) and the
[host guide](CODEX_TASK_HOST.md). A retained journal is authoritative;
notifications only indicate that it may have changed. Unassigned replies
require an authorized explicit assignment. A completed or stopped host turn
requires the documented resume action; a healthy server alone does not prove
that an agent is running. Per-action approval prompts are not required for
operations already included in the developer's configured task authority.
