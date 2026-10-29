# Installed Node native movement fixtures

The existing actual native handoff/replacement fixtures now accept an explicit
installed Node controller. The Node consumer uses only public Session.handoff,
Connection.move, original-reference restoration, operation observations and
client cleanup. Independent native peers keep physical media custody. Rust
compares Node's captured topology, role and binding metadata with the host
journal result; a fresh helper must recover the same record and metadata.

The current profile has 26 independently bounded functions. The original 24 cover session and Conversation role scopes with outgoing
observe/leave; EdDSA and ES256 endpoint device movement; preparation cancellation
and current-authority recovery; and a known commit whose recording projection
fails before recovery. Existing decoded audio, no-leakage, source retirement,
one-effect/attempt and endpoint/key revocation checks remain in their fixtures.
No Node metadata handle gains native negotiation custody.

The complete profile requires 26 fixture cases, 52 original helper joins and
26 joined servers, with zero physical peers and media readers before shutdown.
Each SDK request has four seconds, each process phase has an absolute 15-second
cap, and each test function retains a 90-second outer cap. The Node movement
flag is passed to each intended subprocess, leaving the original Rust SDK tests
on their original controller. The two additional candidate cases now pass locally
under both signing algorithms: actual pending-offer retirement, fresh managed
preparation, stable public candidate identity, decoded media, a subsequent
Endpoint move and original helper/server drains. See the
[current local evidence](../evidence/gate-6/native-candidate-production-local.json).
The complete current 26-case local and Node 22/26 Linux matrices still require
their own current results; the two candidate passes do not replace them.

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
SDK qualification and Gate 6 acceptance remain open. At that historical checkpoint, Mac Rust linking was skipped for ENOSPC.
The current local build and candidate results above resolve that local skip. See [the source-bound results](../evidence/gate-6/node-native-5d.json)
and [the movement evidence](../evidence/gate-6/node-movement-audio.json).

The subsequent exact `b2e0cc21` artifacts pass all24 movements on Node26 and23
on Node22. The Node22 ES256 speaker move fails the unchanged strict assertion
that non-silent audio cannot reach a forbidden receiver. Its cause is unresolved;
prior successes do not qualify that failed scene. The next source records only
fixed probe categories, source/receiver indices and counters before the same
assertion. No media routing, threshold or probe duration was changed. See
[the retained b2 results](../evidence/gate-6/node-native-b2.json).
