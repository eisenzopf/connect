# Installed Node native movement fixtures

The existing actual native handoff/replacement fixtures now accept an explicit
installed Node controller. The Node consumer uses only public Session.handoff,
Connection.move, original-reference restoration, operation observations and
client cleanup. Independent native peers keep physical media custody. Rust
compares Node's captured topology, role and binding metadata with the host
journal result; a fresh helper must recover the same record and metadata.

Six bounded functions cover session and Conversation role scopes with outgoing
observe/leave; EdDSA and ES256 endpoint device movement; preparation cancellation
and current-authority recovery; and a known commit whose recording projection
fails before recovery. Existing decoded audio, no-leakage, source retirement,
one-effect/attempt and endpoint/key revocation checks remain in their fixtures.
No Node metadata handle gains native negotiation custody.

The complete profile requires 24 fixture cases, 48 original helper joins and
24 joined servers, with zero physical peers and media readers before shutdown.
Each SDK request has four seconds, each process phase has an absolute 15-second
cap, and each test function retains a 90-second outer cap. The Node movement
flag is passed to each intended subprocess, leaving the original Rust SDK tests
on their original controller. Candidate-target DTOs are tested synthetically;
these native replacement scenes currently exercise endpoint targets. Actual
candidate device movement remains a separate unverified scope.

The verifier records SDK scene failures and proceeds to the other independent
SDK scenes. Any recorded failure still fails the aggregate run. Compilation,
source guards and ordinary required checks retain immediate failure behavior.
A synthetic subprocess check verifies both continued independent execution and
final aggregate/default failure. Failed scenes cannot report expected resource
join counts as verified joins.

The installed consumer passed eight synthetic operation/restore protocols plus
a deliberate missing-role failure with safe source coordinates. All 17 helpers
and 11 consumer processes joined. The full 64 SDK functions and negative
declarations pass with 204 guarded inputs unchanged. These are protocol/process
checks only. Mac Rust compilation remains skipped for retained ENOSPC;
Ubuntu compilation, media, authorization and failure-recovery results are pending.
See [the evidence record](../evidence/gate-6/node-movement-audio.json).

The first movement candidate `b36e119b` failed compilation with E0507: its shared
shutdown helper tried to consume a server behind a borrowed Fixture. No new
native movement function ran in that cohort. The correction keeps drain checks
before the fixture's original consuming shutdown and counts the join only after
success. Compilation and all actual movement checks remain pending.
