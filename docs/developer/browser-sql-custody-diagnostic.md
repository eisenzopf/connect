# Browser SQL custody diagnostic

This G6-054 diagnostic retains the existing browser baseline and direct bridge
fixtures and their original media limits. The production source is based on
494c85a2 (the same RTC correction used by full run 41209664880). It does not
include the later initial-SIP WAL candidate. Only the diagnostic collector,
finite report reducer and their boundary tests change.

The explicit baseline diagnostic additionally selects existing
connect_store::sql_custody events. Those events already record completed SQL
custody units taking at least 500 microseconds; they do not measure every query,
file admission or complete policy callback. Source call-site buckets retain only
declared labels, integer lines and finite duration summaries. Raw paths, query
text and arbitrary fields remain in ignored private state. Original 65,536-event
and 32 MiB bounds stay fixed; omissions remain explicit. Instrumented timing is
advisory and can affect the run. It cannot qualify performance or establish a
cause from coincidence.

The diagnostic baseline requires all five actual Rust collector boundary tests,
including the new SQL timing field/query-exclusion check. Twelve Python boundary
functions pass locally. Rust and browser execution remain unrun until the exact
source-bound CI artifact verifies. No media limits, source clock, policy, queue,
permission, callback retry or provider/model behavior changes.

Run `41215282254` at `e8a4206b` is independently source-verified against
2,887 Git and 3,089 prepared inputs. Its producer builds in27.03s; all five
actual collector functions pass. The baseline still fails one AV p95 at128.1ms
against the unchanged120ms limit, while both audio observations pass. Direct
ES256 passes; direct EdDSA fails one AV calibration without retained pair scores.

The collector records46,985events with zero omitted. Mixer queue age p95 is
101.929ms. Recorded native acquisition SQL units have6.502ms p95 and66.466ms
maximum custody; under-interval units have9.519ms p95 and34.085ms maximum.
These durations include scheduling while SQL is owned. The trace spans the
whole fixture lifetime, including bootstrap/warmup/cleanup; it does not isolate
query CPU, every query, or the events responsible for a particular marker. The
measurements narrow further investigation without establishing a cause or any
full quality/performance pass. Full scoped results remain in the adjacent
[evidence](../evidence/gate-6/browser-sql-custody-diagnostic.json).
