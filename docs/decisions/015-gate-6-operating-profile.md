# 015 — Gate 6 operating and qualification profile

2026-10-15; G6-011. These are implementation targets, not measured results.

November 3 owner disposition: return **G6-054 to the holding pen** and continue
the remaining Gate 6 dependency graph. G6-054 remains deferred and incomplete;
its earlier resumption instructions below are historical. Do not run or
investigate it while completing the other tasks. Its eventual qualification
still applies to final reconciliation. Continue G6-058 on local native ARM
resources, with the reduced four-move programme and existing quality limits.

November 2 owner scope amendment: “we don’t need x86. focus on arm” and
“we can do that later.” The active Linux target is Ubuntu24.04 ARM64, on the
local Mac Studio VZ guest. Defer x86_64 qualification and remove it as a current
Gate6 completion dependency. The active32-cell matrix keeps both MacARM64 and
UbuntuARM64, Local/Remote profiles, all4transports and both signing cohorts.
All workload, current-authorization, recovery, quality, decode, capture and
cleanup requirements remain unchanged. Earlier x86 results retain their old
source/architecture scope and cannot qualify ARM. Historical dated profiles below
are superseded only as to the active Linux target; they are not relabeled passes.

November 2 owner movement amendment: “that's more moves than is realistic.
reduce the moves for the test.” The active device-movement profile is now
`gate6-realistic-device-roundtrip-v1`: four moves total, one switch and one
return in each of the two concurrent Sessions. Both Session chains schedule
their switch at 15 seconds and return at 40 seconds on the original 62-second
programme clock. A chain waits until its scheduled offset before current reads
or target admission; a slow move cannot reset that clock or extend the programme.
The four participants, six Connections, 60-second media observation, eight-second
current reads, media quality and interruption limits, authorization, original
operation recovery, independent decode, packet capture, five-second teardown and
ten-second idle checks remain required. This changes device-movement count and
cadence only; the separate twenty-handoff requirement remains. Bind every new
result to the movement profile and digest in
`contracts/gate6-transport-matrix.json`. Earlier twenty-move runs remain
historical stress evidence; earlier partial runs cannot qualify this new profile.

November 2 G6-054 resumption: after the reduced-movement run, the owner asks
for the deferred remote-media test to be retested and reaffirms local resources
for all test execution. Select the native Ubuntu24.04 ARM64 guest and its exact
Rust1.97.1 target; no GitHub Actions media job or cloud machine is used. Browser
fixture images must be Linux ARM64 and contain the already verified public
Chromium1248/CFT156.0.8078.4 ARM distribution. Retain all original relay, expiry,
restart, impairment, independently decoded media and resource-drain assertions.
Record new source/runtime/image identities before execution; historical x86
results cannot become ARM passes. Returning to the task does not waive any
remaining browser/native/telephone or topology clause.

November 2 runner-wide amendment: “update all runner test requirements to arm
linux.” All active Linux runner preflights, package targets and ELF checks,
browser/native/telephone fixture requirements, build images and workflow
selections now use ARM64 Linux. The local Mac Studio and its native Ubuntu guest
supply test execution; GitHub is for source synchronization. Automatic test
triggers are removed and no Actions workflow is dispatched. The owner also says
“but don't rerun any tests let's just retest this one and then move on to
completing the rest of the gate 6 plan.” Only G6-054 is to be retested now;
runner edits receive static inspection without general regression reruns. No
historical result is relabeled, no current ARM pass is inferred from an edit,
and all remaining Gate6 work stays in the approved dependency plan.

November 4 matrix idle observation, before first execution: retain the same
running server and the same isolated browser container after the original media
terminal. Record two one-second-separated pre-authentication samples, then
one-second workload and idle observations. The original terminal starts the
unchanged ten-second reclamation window; it is not reset after client cleanup.
Compare current public physical-owner/task/resource-pool counts and each owned
process tree's live processes, threads, descriptors and RSS against the fixed
pre-workload baseline. No extra descriptor/thread/RSS retention is allowed in
this first profile; any retained allocation is a failure to investigate, not an
implicit cache allowance. CPU remains a measured cumulative/delta observation,
not a capacity claim. Require stable restoration by ten seconds, original local
capture/renderer/media-queue cleanup within five seconds, and original process
joins afterward. Keep the browser page, server and PulseAudio fixture alive
while observing idle; no forced GC, source-clock adjustment or process restart.
Browser process measurements include its Node driver and owned PulseAudio
fixture. No native SDK helper exists in this browser/browser topology. Missing
or unavailable observations cannot establish idle. Media failures remain failed
while this independent requirement is measured.

## Runtime and topology

Local development/first run: macOS arm64. Baseline observed macOS and toolchain
versions are recorded in [baseline](../evidence/gate-6/baseline.json). Rust/MSRV
is the repository's 1.97.1 toolchain for this increment. Python minimum is 3.12;
Node minimum is 22. Supported package versions and actual runners are recorded
by candidate qualification.

Hosted server: Ubuntu 24.04 LTS ARM64, a declared systemd service plus a
documented container build for reproducibility. Test Chrome preferred WT and
forced WSS/WebRTC fallback, Firefox fallback, and native/browser/native peers.
The initial pinned browser targets are Chrome 155.0.8059.80 (installed local
baseline) and Firefox 141.0 / Playwright revision 1490 (fallback target from the
existing harness manifest, not yet executed). Linux targets Ubuntu 24.04 LTS
ARM64; record image digest, kernel and package versions before its rehearsal.
A local machine cannot claim an uninstalled remote runner has been tested.
Profile changes update this record before the affected run.

Earlier November2 local testing update (before the owner deferred x86): run development and media
diagnostics on the Mac Studio's native ARM64 Linux guest. The dedicated Colima
`connect-gate6` VZ profile has 6 vCPUs, 12 GiB RAM and a 60 GiB guest disk;
foreign-architecture emulation and host mounts are disabled. Use the explicit
`colima-connect-gate6` Docker context, preserving the existing default VM and
its unrelated database. Build and run from the guest disk, as an ordinary user.
The local Ubuntu24.04 ARM64 component uses the same source, workloads, deadlines
and quality limits, with explicit `G6-058-component/ubuntu24-arm64` identities.
It cannot qualify an x86_64 acceptance cell. The original 32-cell inventory and
hosting targets remain unchanged. Stop GitHub-hosted media qualification;
run41574727724 was cancelled before any media result. Any cloud resources need
owner discussion. See the [local procedure](../../scripts/fixtures/local-linux/README.md).
For the ARM app component, select the owned Playwright1.64.1 Chromium1248
156.0.8078.4 browser before its first execution, with AArch64 production runtime
archives and the ordinary source-bound offline Node installer. Record actual
browser availability/version and keep all37 app checks unchanged. Installation
or a component profile alone establishes no media pass.

October 25 local WT measurement update: the installed Chrome runner is now
155.0.8059.85. Pin that patch version for the explicit 60-second capture/render
measurements and record its actual CDP version in each report. The original
155.0.8059.80 installation baseline remains historical. Firefox/fallback,
Linux, native interoperation and named remote/provider profiles retain their
separate qualification requirements; no media threshold changes are made.

October 27 local fallback measurement update: select the installed, code-signature
verified Firefox158.0.1 with public WebDriver BiDi/Puppeteer25.16.0 for the next
normal-trust macOS arm64 fallback matrix. The initial Firefox141/Playwright1490
manifest target remains an unexecuted historical planning target. Local Firefox
uses independent visible native windows, a fresh owned profile/CA trust and an
explicit owned loopback STUN fixture; its loopback ICE test preference remains
recorded in each browser profile. Native address obfuscation and normal TLS
validation remain enabled. Chrome remains155.0.8059.85. These target selections
do not change source content, capture/draw cadence, observers, thresholds or the
60second window. Prior failed audio observations remain failures. Linux/remote,
physical devices and provider lanes still require their own named execution.

October 27 native fallback receiver update: before the next normal installed
browser matrix, the SDK sets the same 80 ms preferred native jitter-buffer target
on its audio/video receivers before an offer. The installed Chrome/Firefox
capability probes retain this target; a private Firefox comparison measured
about 71–76 ms audio buffering with the target and 51–53 ms with automatic
buffering. These distinct runs do not establish causality or remove retained
audio failures. Sources, observers, thresholds, 60-second windows and independent
AV-sync checks remain unchanged. The target is a preference, not a hard latency
guarantee. Unsupported or ignored native control refuses before signaling.

October 30 candidate browser target update, before the next qualification:
select Chrome156.0.8078.48 for the local macOS arm64 fallback runner and Chrome
for Testing156.0.8078.4 for the isolated Linux x86_64 runner. The Linux browser
comes from `mcr.microsoft.com/playwright@sha256:d65108d1a8095d8b0c4a5e32bf1807a40a5ce90ff0c78473e1116b1c243235e8`;
its observed executable SHA256 is
`377a9fc07b495c3b8d0ab8ec1fdfd153641759f9756fea34d8dfce4de82f5025`.
The native Ubuntu24.04 host preflight records the actual kernel and native ABI;
the media image separately records its pinned paced-audio additions. Each run
still requires the exact installed version and its own source/executable guard.
The earlier 155 targets and source-bound exploratory 156 observations retain
their recorded scopes; this update grants no retrospective qualification.
Firefox158.0.1, the source/codec/packet bounds, 80 ms preferred receiver target,
60-second window, moving-frame/audio/AV limits and recovery rules are unchanged.
Browser-only receiver comparisons are diagnostics and cannot change the
production preference or qualify a Connect transport cell.

October 30 Chrome decoded-pixel observation correction, before the next run:
select `native-decoded-frames/1` for the Chrome fallback fixture. Source-bound
Linux native RTC controls at `a7d87fe8` measured about 25.6 changing 640x360
decoded frames/s while frame callbacks observed only 22; the public composition
counter was also higher than the callback count. The new observer reads actual
borrowed VideoFrames from a cloned received track, identifies every decoded
source, counts only matching 640x360 pixels and records size refusals. It retains
at most two queued frames, closes every borrowed frame and joins its owned reader
and clone. Four ownership/error tests and separate Mac/Linux direct-pixel controls
support this measurement change; they do not qualify a Connect path.
Render callback counts, composition counters and the existing rendered AV-marker
observer remain separately reported. Firefox retains its declared render-callback
observer. Chrome refuses unsupported direct-frame observation rather than
silently changing methods. The 24 independently decoded moving frames/s limit,
60-second window, source, codecs, receiver preference, audio identity, AV sync,
freshness/recovery and teardown rules are unchanged. Earlier callback-based
failures retain their original result. Actual Connect execution under the new
identified observer is still required.

Native endpoint TLS verifies configured server identity and trust. Hosted HTTP
requires HTTPS, WSS and HTTP/3 with declared origins and trusted proxy policy.
Provision short-lived scoped participant/device credentials through a trusted
backend; provider keys stay server-side. Configure public SIP/RTP addresses,
ICE/TURN and callback ingress independently. A reachable HTTPS route is not proof
of duplex telephone media.

October 27 isolated Linux HTTPS fixture target: use the existing local image
`sha256:a81873456efa5d13f9b67cfbecc4e11bf6946e924684f6ffc9a1161ca20fe2b5`
(Ubuntu 24.04.5 LTS amd64; Rust 1.97.1, LLVM 22.1.6, Clang 18.1.3). The local
Docker daemon runs in an arm64 Lima VM, so this amd64 userland is emulated.
The G6-053 functional fixture uses fresh synthetic state, a private CA, four
separate HTTPS roles, an internal container network and only host-loopback
published ports. Freeze this target before executing the host/client checks.
It cannot establish native x86 performance, named remote browser/media coverage,
the G6-056 installation lifecycle or any whole-gate acceptance criterion.

The G6-053 isolated remote-client fixture uses the same pinned Linux client
image and the current Mac arm64 server CLI (SHA256 recorded with its build).
The Linux client reaches four Mac loopback HTTPS listeners through the local
Lima `host.lima.internal` alias. That exact name and port are provisioned in
the private fixture CA/leaf and each Remote-profile public origin before start.
This is a distinct-kernel local hosting boundary, with no external host/provider
or model account. It does not substitute for Ubuntu server installation or the
named remote/media qualifications. Freeze this topology before its wire checks.

October28 owned TURN implementation fixture: coturn4.18.0 on local macOS arm64,
loopback-only listening/relay addresses, one high listener port, sixteen relay
ports and eight canonical UDP media ports. The UDP control lane disables TCP;
the TCP control lane disables UDP. Both use UDP relay allocations, fresh
synthetic REST credentials, existing synthetic Opus/VP8 sources and independent
FFmpeg decode. Each test has a90second deadline, followed by a10second owned
allocation drain. Freeze these bounds before the positive runs. This does not
qualify distinct-network/browser/telephone, secure TURN, WAN/performance or any
whole acceptance criterion. See [procedure](../developer/turn-relay-rehearsal.md).

October28 shared STUN/TURN All-policy fixture observation update: retain the90second
test deadline and10second zero-allocation drain limit, then allow a separate
10second join of the original coturn4.18 process after its explicit zero report.
The initial All-policy run passed AV and deallocation but failed the former
combined10second process-exit observation; retain it as a failure. Upstream's
five-second listener dispatch also runs after drain completion. Freeze this
version2 observation before rerunning; no media/profile threshold changes.

November 2 local Mac browser target update, before its next affected run:
installed Google Chrome now reports156.0.8078.80. Its application signature
passes strict/deep verification with Google LLC teamEQHXZ8M8AV. Freeze that
exact patch for the next owned macOS profile and reference-app rehearsal.
Earlier156.0.8078.48 results retain their original failures and source scope.
Linux x86_64 and local ARM64 Chromium remain156.0.8078.4; Firefox remains
158.0.1. The32cells, source programmes, observers, deadlines, AV limits,
recovery and cleanup checks are unchanged. This records an installed version,
not a browser/media pass or evidence that ARM fixes any Mac failure.

## Default bounds

November4 local WebTransport trust clarification, before the next affected run:
use the standard `serverCertificateHashes` API with the exact SHA-256 of the
fresh owned fixture's ECDSA P-256 leaf, valid for one day. HTTPS assets and
WSS retain normal fresh-profile CA validation. The independent server signing
pin and current enrolled Device credentials remain required. A deliberately
wrong leaf hash must fail before any application stream or mutation; the exact
configured hash must authenticate the actual browser owners. No certificate-error
bypass, developer flag, inherited fixture pin or external hostname is used.
This is the existing SDK's explicit local WebTransport trust mechanism, already
documented in the G6-049 rehearsal. It does not claim a publicly trusted CA
deployment or convert earlier private-CA QUIC refusals into passes. The32cells,
full media/movement programme and all quality/authority/cleanup requirements are
unchanged. The WebTransport specification defines this hash verification mode
and its two-week maximum certificate lifetime:
https://www.w3.org/TR/webtransport/#dom-webtransportoptions-servercertificatehashes.

November1 native six-Connection marker source update, before its first affected
run: source manifest `connect/native-av-sources/2` adds the existing
`connect-av-cadence/1` programme to each of the six identified Opus/VP8 sources.
The three-frame/100ms pulse keeps each source's distinct tone, using the same
0.05 low/0.7 high peak amplitudes as the browser fixture. A separate80×80 patch
at96,0 changes from black to white; the original source-color corner and moving
360p30 background remain. Encoding rates, packet bounds, source duration62seconds
and measured60second window remain unchanged. The independent decoder must
calibrate matching source ordinals from observed decoded gain/pixel edges and
retain the42pair/80ms median/120ms p95 limits. It cannot assign ordinals from RTP
origins, wall clocks or nearest periodic edges. Numeric source-clock diagnostics
remain separate. Initially high levels and onsets after an unobserved interval
cannot establish an edge; continuous low must rearm the observer for200ms.
Packet/frame receive times bind this component's decoded-content observations;
live decoder completion and physical display/speaker timing remain separately
unmeasured. Earlier constant-tone source1 failures retain their original result;
this required marker implementation grants no retrospective qualification.

October31 G6-058 inventory freeze: enumerate Local and isolated Remote-profile
fixtures on macOSarm64 and native Ubuntu24.04x86_64 for nativeQUIC, ChromeWT,
ChromeWSS/WebRTC and FirefoxWSS/WebRTC, with EdDSA/ES256 signing cohorts. Keep
Chrome156.0.8078.48 on macOS and CFT156.0.8078.4 on Linux. Select Firefox158.0.1
as the Linux fallback target too, before its first affected run; an uninstalled
Linux Firefox is unrun, never an inherited macOS result. The Mac/Linux guest and
Ubuntu systemd/network-namespace fixtures are existing bounded Remote-profile
precedents, not named public hosts. Record each actual topology and execution
independently. This adds the explicit inventory, not a result or any change to
media/quality thresholds. Full cell producers still require all work-order
verification while G6-054 is deferred. See the
[matrix](../../contracts/gate6-transport-matrix.json) and
[procedure](../../scripts/fixtures/transport-matrix/README.md).

| Resource | Default / hard check |
|---|---|
| UCTP reliable envelope | 1 MiB |
| Inline message body | 64 KiB decoded |
| History/event page | 100 default, 500 items and 512 KiB maximum |
| File / installation file bytes | 16 MiB / 256 MiB initial profile |
| Artifact transfer chunk | 32 KiB decoded |
| Live event lease / queued refs | 30 seconds / 256 |
| Unauthenticated peer deadline | 10 seconds |
| Lost-last-connection grace | 30 seconds unless explicit end |
| Endpoint confirmation callback | 45 seconds; configured DTMF confirmation |
| Named qualification workload | four participants, six connections, two concurrent sessions |
| Media / quality | [media profile](../../contracts/connect-media-v2.md) |

Existing secure coordinator per-peer budgets (32 Sessions, 64 Connections and
16 cumulative opened Streams per Connection) remain upper bounds until a more
restrictive installation policy is selected. Admission is bounded before external
effects; the minimum tested workload is not advertised capacity.

## Execution lanes

Default checks are local and fixture-only with isolated state and owned processes.
Actual-model, remote-host and live-provider lanes are explicit. G6-093 records
the concrete host, credential references, consenting endpoints, call/spend bounds,
versions and cleanup before those actions. Use the owner's existing authorization
where applicable; do not repeatedly ask for the same approved implementation.

An unavailable Linux/browser/provider environment is recorded as unrun, not a
passing equivalent. Measured CPU/RSS/queues are attributed to server, helper and
browser separately. Media/profile targets freeze before execution; a failed
measurement is not erased by changing its threshold after the fact.


## Native public packet membership producer amendment

The native replacement capture profile is `connect/physical-av-segment/3`, and
native browser decoded observation is `connect/browser-matrix-decoded-media/2`.
The producer retains each original packet/decoded observation and classifies it
using its exact completed native activation cut, including genuinely public output
that arrived before durable completion. Private decoder context remains excluded.
Initial owners require their original public admission; each replacement requires
its own signed-reader-bound public cut. All original source/retirement and Data
receipts remain required. WebRTC observation stays on its existing profile/window.

Native observation remains `[2,62)` seconds and browser observation `[1,61)`;
all 60-second coverage, 59.9-second audio, FPS, age and marker limits are unchanged.
Neither overlap nor delayed replies can create extra coverage: repeated original
source video timestamps and audio source quanta across physical histories fail.
Audio identity comes only from the exact original SR/RTP source clock and frozen
programme origin for the same logical peer. Missing or ambiguous lineage fails;
rounding labels the original source quantum only and never changes row times or
PCM. No silent deduplication, clipping, filler or clock repair is permitted.

This is a prospective producer/profile change. Historical v192 rows are not
requalified by it. Implementation, focused runtime proof and all32 qualification
are separate states; the private draft has no execution result.

## Prospective resource capacity amendment for the next Gate 6 run

The owner approved retaining strict Session ownership and cleanup while allowing
bounded process warm-up across three real workloads on the same original OS
owners. The later discussion rejected treating every increase in process RSS or
idle worker counts as a leak. This amendment implements that direction as the
prospective `connect/matrix-resource-capacity/3` profile. It does not revise any
earlier verdict. The November 4 zero-retention profile above and the later /1
and /2 plateau profiles remain historical evidence with their original results.

Each of the three original workloads must still complete its actual programme.
Every original ten-second cleanup window must restore zero Session physical
owners and owned tasks, restore original resource-pool usage, and prove natural
Session and helper joins. Keep exact original PID and birth identities throughout
the cycles. A missing required idle sample, absent valid workload capacity evidence, forced
cleanup, changed owner or incomplete workload cannot qualify. Incomplete
workload process observations do not establish capacity. Preserve their skipped
counts and reasons, and reject any known RSS budget breach even in those partial
observations. Each cycle still requires complete quantitative workload samples. Functional media, movement and
authority failures remain separate failures.

Freeze each role's process and descriptor capacity, and browser or helper thread
capacity, from numeric samples taken during the FIRST original workload before
its terminal. Later workloads and global process-summary peaks cannot raise this
reference. All final idle samples must remain inside that demonstrated capacity.
This permits an idle worker or descriptor count already demonstrated during the
original workload. It is not proof that every OS resource belongs to a cache.

The server thread ceiling is its original baseline total thread count plus the
512 blocking threads configured by the pinned Tokio 1.53.2 default runtime. Bind
that exception to the exact server main source and Cargo.lock inputs, the pinned
Tokio registry checksum and reviewed builder source. The server uses its sole
`#[tokio::main]` default multi-thread runtime. A changed or missing binding fails
closed until reviewed. This is a conservative finite process capacity ceiling.
It does not claim each idle thread has returned every possible job. Actual
Session task, pool and join checks supply the independent cleanup requirement.
Report each later idle count and its change without treating bounded pool
residency alone as failed Session cleanup. No runtime pool settings are changed.

The declared RSS operating budgets are **1 GiB for the server**, **4 GiB for the
browser process tree**, and **1 GiB for the native helper tree**. The aggregate
observed process-tree RSS ceiling is **6 GiB**. These are engineering capacity
budgets chosen for this local 12 GiB guest profile, not measured allocator limits
or OS-enforced quotas. Browser and helper roles are alternative client profiles.
All observed workload samples and final idle samples must fit their role budget.
Summed RSS can count shared pages more than once and is intentionally
conservative. These budgets leave room for the guest OS, capture and fixture
services. They do not waive resource-ownership checks or tune process allocation.

For each role, retain the RSS interval from all three final idle samples in each
cycle. There is no distinguishable later rise at observed resolution if the third
interval overlaps or lies below the second. Otherwise require the entire possible second positive increment to
be strictly smaller than the entire first increment. Equal, increasing, or
indistinguishable positive increments fail this warm-up test. There is no fitted
byte allowance, growth floor or contraction ratio. The intervals describe actual
observed variation, not statistical confidence. Three cycles cannot prove the
absence of a slowly decaying leak. Passing establishes bounded capacity and this
short warm-up criterion alongside actual ownership cleanup. A longer soak is a
separate claim. Keep raw observations and later growth diagnostics.

## November 7 owner resource disposition and separate soak

The owner selected **“Use budgets and a separate soak (Recommended)”**. The
prospective `connect/matrix-resource-capacity/4` profile therefore keeps the
declared server1GiB, browser tree4GiB, native helper tree1GiB and aggregate6GiB
operating budgets, the frozen first-workload count capacities and the exact
source-bound server thread capacity. It also keeps all three complete original
workloads, exact persistent owners, zero Session owners/tasks, original pool
restoration, actual natural joins and every ten-second cleanup window. These
remain hard requirements. No capacity value is raised by this disposition.

The three-cycle RSS interval and increment observations remain in every report.
A linear, accelerating or unresolved RSS rise is now a **diagnostic**, rather
than a failure of this capacity profile. It is carried into a separate sustained
growth assessment. The earlier /3 hard trend proposal and all older actual
verdicts retain their original scope. No historical result is relabeled. No
forced collection, allocator reset, cache trim or process restart supplies a pass.

The separate soak belongs to [G6-098](../connect-gate-6-implementation.html#G6-098),
“Measure transition quality, recovery and resource bounds”. It remains **pending**.
Its existing dependencies G6-057, G6-079, G6-092, G6-093, G6-095 and G6-096 remain
unchanged. A capacity-profile pass cannot mark G6-098 complete.

The proposed soak work is to retain the same original server and client process
owners for30 complete62-second programmes with the same four moves, six sources
and two Sessions. Keep the original per-move, cleanup and idle deadlines. Record
the full workload and final idle resource samples on every cycle, along with
exact process identities, Session owner/task restoration and natural joins.
Use the same declared capacity budgets throughout. This is a separate bounded
soak plan, not permission to expand the current three-cycle controller or start
new cells now. Its implementation and execution follow the existing G6-098
dependency graph.

Prioritize the three original noncontracting observations from v203

- macOS local Firefox ES256 browser residency
- ARM Linux local Chrome WSS/WebRTC ES256 browser residency
- ARM Linux local Firefox ES256 server residency

For those cases preserve role-specific RSS trajectories and the new allocator,
object-owner and cache-charge evidence. Assess repeated growth over the longer
sequence and attribute it before claiming a leak or bounded retained cache.
The browser RSS observations include browser-managed/native allocations that
server object counters cannot explain. Source, runtime and original verdict
evidence must remain bound. Any capacity breach or Session cleanup failure
remains a failure regardless of the RSS trend. The soak must publish its actual
completed-cycle scope and unresolved attribution, and it remains unexecuted
until its own required work is performed.
