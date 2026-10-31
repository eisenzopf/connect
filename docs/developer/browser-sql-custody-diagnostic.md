# Browser SQL custody diagnostic

This G6-054 diagnostic retains the existing browser baseline and direct bridge
fixtures and their original media limits. The production source is based on
494c85a2 (the same RTC correction used by full run41209664880). It does not
include the later initial-SIP WAL candidate. Only the diagnostic collector,
finite report reducer and their boundary tests change.

The explicit baseline diagnostic additionally selects existing
connect_store::sql_custody events. Those events already record completed SQL
custody units taking at least500microseconds; they do not measure every query,
file admission or complete policy callback. Source call-site buckets retain only
declared labels, integer lines and finite duration summaries. Raw paths, query
text and arbitrary fields remain in ignored private state. Original65,536event
and32MiB bounds stay fixed; omissions remain explicit. Instrumented timing is
advisory and can affect the run. It cannot qualify performance or establish a
cause from coincidence.

The diagnostic baseline requires all five actual Rust collector boundary tests,
including the new SQL timing field/query-exclusion check. Twelve Python boundary
functions pass locally. Rust and browser execution remain unrun until the exact
source-bound CI artifact verifies. No media limits, source clock, policy, queue,
permission, callback retry or provider/model behavior changes.
