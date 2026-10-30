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

The server binds and advertises eight owned `127.0.0.1` UDP ports. Browser
`getStats` must prove the nominated address and port remain in that range.
The earlier port-preserving proxy was correctly rejected: ICE found a direct
path that bypassed its impairment. Those failures remain recorded.

The current lane runs only inside its dedicated network-disabled Linux Docker
container. Only the full/fault lanes receive `NET_ADMIN`, used to create owned
loopback queue disciplines and sixteen UDP source/destination port filters.
Control TCP, local audio sockets and unrelated traffic remain outside the filters.
Both UDP directions receive independent 3% kernel random loss and 50ms delay
with 50ms default uniform jitter (0–100ms configured delay). No fixed-seed or
exact realized 3% RTP loss is claimed. The kernel's
[netem distribution implementation](https://github.com/torvalds/linux/blob/v6.1/net/sched/sch_netem.c)
uses a uniform random value when no distribution table is supplied.

Twenty seconds after the actual measured window becomes ready, both directions
get one three-second 100% loss interval. Its observed interval starts after both
commands acknowledge and ends before either restoration starts, so command
latency grants no extra recovery time. The decoder reads that observed interval
and retains the original two-second recovery bound. Both directions must show
actual burst drops, ordinary drops and forwarded packets. Numeric queue counters,
source manifests and cleanup evidence are retained. Each queue is bounded to512
packets. The original media ports must be reclaimed and the original noqueue
loopback configuration restored before a pass. The helper refuses non-Docker
execution or a pre-existing queue configuration; it never changes the host.

Both participants must independently identify the opposite decoded audio/video
source before and after interruption. Existing A/V and motion observers retain
their thresholds. This is a local fault component, separate from distinct-network,
clean video quality and acceptance qualification.

The current Mac refuses binding `127.0.0.2` with `EADDRNOTAVAIL`; its network
configuration is unchanged. The separate manually dispatched
`Gate 6 isolated Linux browser media` workflow runs the actual cases on a native
Linux x86_64 kernel, in a network-disabled container with the previously verified
CFT156 base image. It builds production runtime libraries at release opt3,
separately from the recorded local producer's root-package opt0 exceptions.
Only the integration-test driver uses an explicit final-target opt0 override;
all eleven required runtime libraries must still report opt3. The original
full-opt3 driver run remains separate. It records exact source, executable,
image, package and automation hashes. No image or package is published.
Compiler progress exposes only completed-artifact counts and fixed runtime
library names/optimization levels. Successful builds require all eleven checked
client/server/store/service, Core, media, RTP and RTC libraries at opt3. Private
paths, package identifiers and raw compiler output remain excluded. The original
one-hour build limit and media thresholds are unchanged.
The fixture links the public server library directly and uses
`--no-default-features --features test-support` to omit the unrelated CLI
executable. Normal builds retain the default `cli` feature and produce `connect`.
The producer refuses unexpected CLI artifacts in this library-only lane.

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
## Synthetic browser permission in the owned Linux container

The network-disabled Chromium fixture explicitly enables its loopback ICE
interface and grants camera/microphone permission only to each fresh synthetic
page's loopback origin. Canvas/AudioContext input does not itself request device
permission. A controlled owned Linux probe showed pending host gathering without
permission and completed gathering with two host candidates after the grant.
The fixture captures no physical device and does not disable address obfuscation
or certificate checks. Actual current Linux x86 media results remain required.

Failed decoded-audio observations retain at most sixteen numeric sample measurements: elapsed time within the measured window, finite RMS and frequency (otherwise null), and a fixed phase label. The count of omitted failures stays explicit. These observations contain no raw audio, peer identifiers, addresses or private fixture fields. They explain an existing failure without changing identity, coverage, burst exception or recovery rules.

The independent browser observer also saves before/after RTP counter snapshots from `getStats`: at most four owned peer objects and eight allowlisted numeric RTP/media rows per peer. Packet loss, NACK/PLI/FIR, retransmission, audio concealment and jitter-buffer counters explain transport behavior; they are not a substitute for decoded audio/video or a routing diagnosis. Unknown fields, transport addresses, track identifiers and nonfinite or negative counters are excluded.

Chrome reports `native-decoded-frames/1` for its identified moving-pixel count.
The observer owns a cloned received track and a two-frame queue, samples actual
decoded pixels, records frames refused for dimensions, closes borrowed frames and
joins before teardown. Source identity covers all decoded frames. The original
render callback pixel counts and composition counters remain visible separately;
rendered AV-marker timing keeps its existing observer. Firefox retains its
declared render-callback measurement. The operating profile records the controlled
callback undercount and the unchanged quality limits. A missing or failed native
observer cannot qualify a Chrome cell.
