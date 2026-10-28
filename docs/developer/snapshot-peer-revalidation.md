# Snapshot peer-source revalidation

The installed Node control fixture identified a four-second deadline in
`Session.refresh()` after helper restart. Native end status/wait and Session end
wait had already passed. Session refresh reads the current Conversation
snapshot; the previous recorded full run passed this path, so the precise cause
of every historical deadline remains unproven.

A peer snapshot read repeated full topology/projection scope discovery in its
source-binding checks as well as in the snapshot executor. The change removes
the two duplicate source-check walks for one credential. The executor retains
its existing current scope discovery and coverage check both before and after
reading, in the same immediate SQLite transaction. The returned snapshot,
pagination, current visibility checks and final actor checks are unchanged.

`require_peer_snapshot_source` still verifies installation ownership, proof
liveness, exact credential/device/key binding and the exact Conversation/query
at both source-check points. For one credential the executor's actor comes from
the same current credential as the peer binding. Multiple credentials still use
the complete per-credential snapshot revalidation path; their authority is not
replaced by the last credential alone. No authority or projection result is
cached across requests or reused from an earlier transaction.

Regression tests assert three executor captures for one credential and seven
for two credentials. Additional checks reject a source-only proof lacking
projection coverage and a signing key revoked after admission. Existing snapshot
pagination, scope binding, membership/visibility and peer-authority tests remain
required. Capture instrumentation exists only in the Store unit-test build.

The Ubuntu verifier now builds the Store test binary, verifies the exact three
new test names, and runs the snapshot and peer-authority groups before its
existing native fixtures and installed Node journey. All original SDK request,
phase and whole-function deadlines remain unchanged. Local Rust execution is
explicitly skipped because the retained Mac linker ENOSPC prevents a reliable
native build. Compilation, security regressions and actual deadline behavior
are pending Ubuntu verification; this document does not claim a pass or a
resolved timing cause.
