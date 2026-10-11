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

## Work still required

This profile does not qualify 16 actual simultaneous host workers, complete
MCP payload/deadline operating loads, grant refresh and expiry under sustained
load, age-based retention, retained voice-setup metadata, installed packages,
real-model journeys or all Gate 4 acceptance criteria. The separate
[shared MCP capacity audit](evidence/gate-4/mcp-shared-capacity.json) retains
its own request/listener admission and lifecycle evidence.

For recovery, use [task diagnostics](TASK_DIAGNOSTICS.md) and the
[host guide](CODEX_TASK_HOST.md). A retained journal is authoritative;
notifications only indicate that it may have changed. Unassigned replies
require an authorized explicit assignment. A completed or stopped host turn
requires the documented resume action; a healthy server alone does not prove
that an agent is running. Per-action approval prompts are not required for
operations already included in the developer's configured task authority.
