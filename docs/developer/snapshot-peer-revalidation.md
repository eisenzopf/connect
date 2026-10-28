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

The Ubuntu verifier built the Store test binary, checked all three new names,
and passed 22 snapshot and 38 peer-authority functions, including the capture,
projection-coverage and current-key regressions. The same source-bound run
passed all 77 existing native functions and the installed Node control/recovery
journey for EdDSA and ES256 in 64.72 seconds. Its ten phases completed within
the unchanged four-second SDK, 15-second phase and 90-second function budgets;
all six original helpers and both servers joined, with zero physical peers and
media readers. Both no-grant movement refusals also passed in both algorithms.

Artifact 12842951491 from commit `8e7a8d21` verifies 8,077 unchanged inputs;
3,985 tracked sources were independently compared with that Git revision.
Compiled Store/server/CLI hashes and the full results are in the
[evidence record](../evidence/gate-6/snapshot-peer-revalidation.json).
The long CI phase was a 1,385.93-second server-library rebuild; Store compilation
took 55.53 seconds. Local Mac Rust execution remains explicitly skipped because
of the retained linker ENOSPC.

This proves the duplicate-work reduction and these functional/security scopes.
It does not establish the cause of every earlier timing failure or qualify a
performance profile, newer SDK receipt/audio scenes, full Node SDK or Gate 6.
