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

## Retired voice setups: durable binding and full-capacity measurement

The original implementation retained a closed setup only in `VapiMedia` memory.
A local regression demonstrated that, after expiry and server restart, the same
session could dispatch an ordinary Vapi call without its original task tools.
That invalidates the earlier no-fallback claim across process restart.

The repair stores one immutable setup binding per durable session in schema 16.
It records the credential reference and original bounded grant definition, never
callback secrets or filesystem paths. Provider admission compares that exact
binding and current authority. Sessions with a protected setup cannot select the ordinary call fallback.
Voice-only task sessions without configured tools keep their existing behavior. A retired or unavailable listener requires a new voice session;
the original task, conversation, private recovery intent and history remain.

The process-local map can therefore release retired entries completely. Its live
listeners share the 32-listener server ceiling. Durable binding rows are bounded
by the retained session limit (10,000 by default). The full-capacity profile below measures that bound using real setup expiry and listener reuse. The additive migration
reserves older task-created sessions without inventing missing private intent.

The relevant implementation is [setup binding](../crates/connect-store/src/tasks/media_setup_binding.rs),
[session admission](../crates/connect-store/src/sessions.rs),
[setup retirement](../crates/connect-server/src/vapi_media.rs) and
[listener ownership](../crates/connect-server/src/vapi_bootstrap/media.rs).
The [restart repair audit](evidence/gate-4/vapi-setup-restart.json) qualifies
migration, rollback, exact-intent admission, concurrent registration, restart
denial and same-task continuation: 211 Store and 32 Vapi/parity checks pass, with
strict workspace lint and Rustdoc. Repeated retirement is now qualified by the profile below; full current-source and whole-item qualification remain open.

The full retirement profile is now available as a separately bounded check:

```sh
bash scripts/prepare-dependencies.sh
python3 scripts/check-voice-retirement.py
```

It exercises all 10,000 retained sessions in 313 batches, each with at most 32
live callback listeners and real five-second expiry. Every listener is checked
while live and after expiry. Only connection refusal proves retirement; address
exhaustion or other probe errors fail the run. A connection reset during teardown
remains pending until a later refusal within the same deadline. The eight-second batch observation
deadline includes waiting for expiry. The full exercise has a 45-minute budget,
inside a 55-minute build/test phase. The Gate 4 runner invokes this profile
explicitly after the ordinary workspace run, where it is marked ignored.

Boundary samples measure the whole Rust test process, including clients and the
loopback provider fixture; they are not server-only memory or exact peak memory.
The final checks require session-capacity rejection, identical durable binding
digests across server restart and denial of replacing the original setup.
Reports, original private intents and a Store snapshot remain under ignored
`.local/voice-retirement/`. No provider request is permitted.

The [retirement audit](evidence/gate-4/voice-retirement-load.json) qualifies all
10,000 setups across 313 batches in 1639.963 seconds. All 628 boundary samples
were present. Whole-process sampled RSS peaked at 60.7 MiB; this includes
the server, clients and local provider fixture and can miss transient peaks.
The final snapshot retains 10,000 distinct original bindings with no unknown
markers. Restart preserves their exact digest; replacing the last original
setup is denied, and the retained-session ceiling rejects another session.
The provider fixture received zero requests. All 777 source inputs remained
unchanged, and the supervisor found no leftover process-group members.

Three failed observer attempts remain recorded. Excessive pre-expiry probes
exhausted local TCP addresses, and an initial correction treated a shutdown
reset as an immediate failure. The final observer waits for declared expiry,
treats reset as pending and requires refusal before recording retirement,
within the unchanged deadline. Two Rust observer controls, 41 Python verifier
checks, dependency preparation, formatting and strict workspace Clippy pass.
This component result does not complete the full runner, installed candidate,
provider/model acceptance or G4-030's prerequisite items.

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

## Sixteen concurrent reference runners

The [runner-concurrency result](evidence/gate-4/runner-concurrency.json) now
qualifies sixteen real `connect-example task` processes against one local
server, separately from the Codex host profile. The Gate 4 runner builds both
normal executables before this phase, including in source-only qualification.

```sh
python3 scripts/check-runner-capacity.py \
  --binary /absolute/path/to/connect \
  --example-binary /absolute/path/to/connect-example
```

The 240-second profile creates seventeen tasks and protected native grants via
the public SDK and CLI. Sixteen workers hold distinct, unexpired 30-second step
leases. A seventeenth must exit with an explicit capacity error and no message
or claim. After one worker stops and its lease is explicitly released, the
rejected worker retries its original saved intent and joins the other fifteen.
Sixteen explicit replies produce one application action and follow-up each.

After server and application restart, the original cohort resumes saved state.
The previously stopped worker handles its delayed reply; duplicate replies for
the others return their original receipts. All seventeen conversations contain
exactly one original question, one reply and one follow-up. The driver reads
application state only through public diagnostics and a read-only comparison of
the saved claim; it never opens or modifies the Connect server database.
The independent phase supervisor verifies that no inherited processes survive.

At clean revision `1d7061e`, the profile passed in 5.852 seconds with normal debug
executables. Sampled server RSS peaked at about 67.8 MiB. Waiting worker cohorts
totaled about 323.4 and 324.0 MiB at the two observations. These are local samples,
not total-memory peaks or guarantees. The recorded 641.785 ms startup interval
is for the whole initial cohort, not an individual operation latency or SLO.
The sixteen-step limit does not impose a new limit on worker process counts.

The first two driver attempts failed on an incorrect address argument and an
incorrect expectation that a known rejection should count as uncertain pending
work. Both failures are retained. Five verifier controls and all 23 runner
checks pass; the exact new runner phase also passed in a clean checkout.

## MCP body and deadline load

The [MCP load result](evidence/gate-4/mcp-operating-load.json) measures a separate
server through raw HTTP stress probes. Existing official `rmcp` client tests
remain the protocol and SDK evidence; this harness targets request framing,
admission, timeouts and bounded tool projections.

```sh
python3 scripts/check-mcp-load.py --binary /absolute/path/to/connect
```

The 180-second profile creates private state under `.local/mcp-load/` and runs
in both full and source-only Gate 4 qualification. It exercises:

- Three 32-request bursts of valid JSON-RPC bodies exactly 512 KiB long.
- Two overflow bursts, using Content-Length and chunked transfer independently;
  all 64 oversized bodies receive HTTP 413.
- Two saturated waves of 32 bodies, each missing its last byte. An extra request
  receives 503; all stalled bodies receive 504 near the ten-second deadline.
  Admission recovers after each wave.
- Thirty-two maximum-duration waits. Each returns honest pending metadata and
  the original cursor, which remains usable for a subsequent request.
- Eleven history pages fetched in 32-request bursts, recovering all 32 seeded
  source IDs with provenance and a structured projection no larger than 32 KiB.

The framing limit applies to the JSON-RPC body, including its envelope and
padding; HTTP headers are outside that body limit. Ordinary bursts synchronize
client starts but do not assert that every execution overlaps. The stalled-body
waves independently prove all 32 admission permits are occupied. The history
limit applies to the structured projection; MCP can also include its text copy.

The clean `442c010` run passed in 33.084 seconds using normal debug executables.
The sampled server RSS peak was about 206.7 MiB. Stalled-body responses arrived
around 10.00 seconds including client overhead; tool waits around 9.27 seconds.
The largest measured structured history projection was 25,337 bytes. These
measurements describe this local run, not a throughput, memory or carrier-latency
guarantee. A short phase with no RSS sample reports `null`.

An earlier clean-checkout attempt completed the load but failed its final
observation: the setup client had exceeded the native transport's 30-second
idle timeout. The corrected harness closes setup before load and opens a fresh
public observer afterward; transport deadlines remain unchanged. The failure
is retained. Five verifier controls and all 28 qualification-runner checks pass.

## Grant expiry, renewal and autonomous cancellation under load

The [grant lifecycle result](evidence/gate-4/grant-lifecycle-load.json) qualifies
32 independent tasks through real wall-clock grant expiry, explicit renewal,
agent-initiated cancellation and server restart.

```sh
python3 scripts/check-grant-lifecycle.py --binary /absolute/path/to/connect
```

The 180-second profile uses private `.local/grant-lifecycle/` state. It creates
tasks through the native Python SDK and grants through the protected CLI.
Each agent first sends one message, retaining its request ID. Repeated
32-request status bursts cross the grants' twelve-second lifetime. Once expired,
all grants must deny both status and replay of previously accepted sends.

A saved grant export cannot renew itself: resuming it reports
`saved-not-refreshed` without changing its bytes. The developer then explicitly
issues replacement grants with the original task, binding and permissions.
Replaying every original send must return the exact saved receipt without a
second message. The old credentials remain unusable.

The configured scope includes `task_cancel`. Half the agents cancel their own
tasks while the others replay their original sends. No per-action human approval
is requested. After server restart, cancelled and expired credentials remain
denied, the other sixteen tasks remain usable, and owner inspection confirms all
32 original task bindings and message IDs. Issuance and inspection use a separate
protected owner interface, never a fallback credential for an agent tool.

At clean `f4f47f0`, the normal debug profile passed in 15.039 seconds. Its expiry
loop observed 3,936 successful status responses and 32 unauthorized
responses; explicit post-expiry probes independently verify denial. Sampled
server RSS peaked at about 88.1 MiB. The numbers describe this workload and
include local tool overhead; they are not throughput or memory guarantees.
Five verifier controls and all 33 qualification-runner tests pass. Both full and
source-only qualification run this profile after normal executable builds.

The grants are short by design. Existing contract tests establish the
300-second grant maximum; this is not a 300-second endurance run. Task lifetimes
outlast these grants, so elapsed-task terminalization, artifact expiry under
load and retired voice metadata retain their separate operating checks.

## Artifact and task age retention

The [artifact retention result](evidence/gate-4/artifact-retention-load.json)
qualifies 32 expiring artifacts and tasks through public interfaces, real
wall-clock expiry and two server restarts.

```sh
python3 scripts/check-retention-load.py --binary /absolute/path/to/connect
```

The 180-second profile creates private `.local/retention-load/` state. Each task
uploads a 64 KiB artifact using 32 KiB chunks, then repeatedly replays the same
accepted read as the artifact's twelve-second lifetime elapses. Expiry must
return a business denial even for that previously accepted request. Recall
retains the expired reference and provenance with no body, alongside the
original available conversation message.

Restart removes the 32 expired final blob files. A separate non-expiring control
file must retain its exact bytes. Expired source references remain visible and
cannot retrieve bytes after cleanup. The second part runs status requests to
the tasks' 45-second lifetime boundary, then checks denial and uses the public
owner interface to commit and inspect their expired state. Another restart must
preserve all task records and original message IDs. New owner-issued grants for
these expired tasks are denied; extending authority requires a separately
created task, not silently changing the original lifetime or conversation.

The range-read review also found that fetching a small artifact chunk read the
whole blob into memory first. Commit `d93bdd0` moves reads into a small module
that validates the opened file, seeks to the requested offset and reads only the
bounded range. Existing authority and expiry checks remain. Two new I/O tests
reject reads outside a 32 KiB window in a virtual 16 MiB file and check EOF and
short-read errors. They establish I/O bounds, not a large-file process-memory
benchmark. All 205 store tests and five server artifact regressions pass, as do
strict workspace Clippy, Rustdoc, formatting and 37 verifier checks.

At clean `dfda3b1`, the unchanged profile follow-up passed in 46.508 seconds.
Sampled server RSS peaked at about 107.4 MiB. Its artifact expiry loop returned
3,488 successful reads and 32 denials; explicit post-expiry probes separately
verify all 32 denials. Task status requests continued until expiry, followed by
32 explicit post-boundary denials. These are local workload measurements, not
memory or throughput guarantees, a 24-hour endurance run, or a full 256 MiB
storage-quota test. Cleanup concerns final blob files on startup, not secure
erasure of backups, downloads or every retained database byte.

The first source-stable qualification passed seven phases but failed the
retention phase during CLI initialization at the existing 15-second deadline.
A read-only startup diagnostic and the unchanged profile then passed; neither
binary nor timeout changed. The initial failure remains recorded and its cause
is unresolved. The older full-run and installed-package results predate this
production change, so current full regression and package qualification remain
required.

## Work still required

The retained voice-setup profile is qualified at its recorded source. Current-source
full regression, installed-candidate qualification and the separate debug CLI
startup finding also remain open. The [review reconciliation](evidence/gate-4/review-findings.json)
verifies the earlier QUIC parity fixture repair and its retained before/after
logs; historical unresolved notes predate that correction. The
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
