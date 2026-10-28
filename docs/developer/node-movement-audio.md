# Installed Node native movement fixtures

The existing actual native handoff/replacement fixtures now accept an explicit
installed Node controller. The Node consumer uses only public Session.handoff,
Connection.move, original-reference restoration, operation observations and
client cleanup. Independent native peers keep physical media custody. Rust
compares Node's captured topology, role and binding metadata with the host
journal result; a fresh helper must recover the same record and metadata.

Twenty-four independently bounded functions cover session and Conversation role scopes with outgoing
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

The original Ubuntu Node 22 and Node 26 matrix jobs at `5d984b57` each passed
all 24 independently bounded movement scenes: 48 original helpers and 24 servers
joined, with physical peers and media readers drained. Private/restored
consultation audio and remote EOF also passed. Each artifact independently
matches 4010 tracked Git contents and modes with 8102 guarded inputs unchanged.

Each aggregate still failed a later receipt-revocation assertion, so these are
component passes only. Current source changes, candidate-target movement, full
SDK qualification and Gate 6 acceptance remain open. Mac Rust linking remains
skipped for retained ENOSPC. See [the source-bound results](../evidence/gate-6/node-native-5d.json)
and [the movement evidence](../evidence/gate-6/node-movement-audio.json).

The subsequent exact `b2e0cc21` artifacts pass all24 movements on Node26 and23
on Node22. The Node22 ES256 speaker move fails the unchanged strict assertion
that non-silent audio cannot reach a forbidden receiver. Its cause is unresolved;
prior successes do not qualify that failed scene. The next source records only
fixed probe categories, source/receiver indices and counters before the same
assertion. No media routing, threshold or probe duration was changed. See
[the retained b2 results](../evidence/gate-6/node-native-b2.json).
