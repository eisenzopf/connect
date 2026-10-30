# Owned browser media network rehearsal

G6-054 remains in progress. This lane adds a measured local UDP fault fixture;
it does not establish distinct-network, named remote, installation or full Gate 6
acceptance. Clean-profile failures retain their original thresholds and evidence.

`scripts/rehearse-gate6-browser-network.py` requires a source-bound
`gate6_canonical_webtransport` test executable, its original repository input
manifest, an installed supported Chrome, Node and Playwright, and a fresh output
directory under `.local`. Select `--algorithm EdDSA` or `ES256`. The normal browser
environment uses `CONNECT_GATE6_CHROME`, `CONNECT_GATE6_NODE` and
`CONNECT_PLAYWRIGHT_MODULE`. Prepare dependencies before building.

The server binds eight owned `127.0.0.1` UDP ports and advertises the same port
numbers on `127.0.0.2`. Each proxy has one fixed server destination and bounded
original-client backends. Actual browser `getStats` must prove that the nominated
remote address and port traverse this pool before and after measurement; direct
bypass cannot qualify. No SDP is rewritten by the runner.

The existing impairment proxy applies fixed-seed 3% Bernoulli decisions and
0–100 ms delay to opaque UDP datagrams in each direction, with one three-second
blackhole interval. This is not a claimed 3% RTP packet rate. Actual counters,
realized drop fractions and stall drops are recorded. Both participants must
independently identify the opposite audio/video source, observe content before
the interruption and meet the existing two-second recovery checks afterward.
The existing impairment A/V policy retains raw skew/calibration results; it does
not turn this into a clean 24-fps pass. Every proxy closes, its workers join, and
the original frontend/backend addresses must be reclaimed.

The current Mac refuses binding `127.0.0.2` with `EADDRNOTAVAIL`; its network
configuration is unchanged. The separate manually dispatched
`Gate 6 isolated Linux browser media` workflow runs the actual cases on a native
Linux x86_64 kernel, in a network-disabled container with the previously verified
CFT156 base image. It builds a full release opt3 producer, separately from the
recorded local producer's opt0 exceptions. It records exact source, executable,
image, package and automation hashes. No image or package is published.
Compiler progress exposes only completed-artifact counts and fixed runtime
library names/optimization levels. Successful builds require all eleven checked
client/server/store/service, Core, media, RTP and RTC libraries at opt3. Private
paths, package identifiers and raw compiler output remain excluded. The original
one-hour build limit and media thresholds are unchanged.

An owned PulseAudio null sink supplies a paced software clock, with one private
Unix socket and cookie. The native-protocol cookie path follows the
[PulseAudio module documentation](https://wiki.freedesktop.org/www/Software/PulseAudio/Documentation/User/Modules/).
A separate original-browser clock check precedes media cases. No host audio
device, personal profile, system trust, provider or model account is exposed.
Each case owns new synthetic identities, state and a normal private CA profile.

The workflow keeps independent results for native ICE, ordinary clean AV,
same-peer browser ICE for both algorithms, unreachable TURN and both impairment
scenes. Raw logs, certificates, keys, cookies and browser profiles remain private;
only source manifests and bounded numeric summaries are uploaded. Preparation,
compilation, skipped cases and runtime qualification are recorded separately in
[the preparation evidence](../evidence/gate-6/browser-network-preparation.json).
