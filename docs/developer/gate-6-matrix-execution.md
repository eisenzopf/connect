# Gate 6 transport matrix execution

The 32-cell matrix is the acceptance framework for G6-058. A failure in one
cell must not postpone attempts in other executable cells. Workloads run on the
owner's Mac Studio and local Ubuntu24.04 ARM64 guest. G6-054 remains deferred.

The [current 32-cell table](g6-058-matrix-status.html) links actual attempts and
prerequisites. The latest complete attempt, v192, attempted all 32 configurations
and recorded all 432 verdicts: 312 passed, 72 failed and 48 blocked, with zero qualified
configurations. Both original workers and all original workload guardians exited;
all 32 report hashes and frozen source guards were checked. No storage interruption
occurred. See [v192 evidence](../evidence/gate-6/matrix-full-v192.json).

The current [correction batch](../evidence/gate-6/g6-058-post-v192-correction-plan.json)
addresses original PCM loss, target activation ordering, typed metadata retry,
observer resource lifetime and fixture gaps. The Firefox loopback ICE and native
QUIC leaf-certificate fixes are applied and syntax checked. Original audio
retention, subscription metadata retry and explicit canvas capture remain private
candidates. Activation, observer lifetime and Node programme ownership are still
being implemented. A narrow WebRTC startup metadata draft preserves the exact
original attempt; private readiness and server resource growth still need causal
evidence. None of this new work has a runtime pass. Preserve every original age,
authority, media quality and idle limit; no inherited cell qualification or
threshold adjustment.

The previous coordinated v196 library batch passes37candidate checks plus a
separate passing baseline control and expected pre-fix failure. All3actual
signed server cases pass, including both AV signing modes and final shutdown.
See [terminal custody evidence](../evidence/gate-6/owned-terminal-cleanup-candidate.json)
and [signed server evidence](../evidence/gate-6/signed-server-batch-progress.json).
These focused results did not replace the full v192 observations.

No matrix run is active. Review and seal exact before/after verification of the
complete correction batch, then execute another full32/all432 candidate from one
clean identical host/VM revision. Keep all tracked files, prepared dependencies
and runner inputs unchanged until both original workers join.

`scripts/run-gate6-transport-matrix.py --phase run` starts the complete two-host
coordinator from the Mac Studio. Every run schedules all 32 configurations and
evaluates all 432 required checks. Each host continues after an individual
failure, and the coordinator merges their original reports. It rejects stale
candidates, changed contracts, duplicate host reports and evidence assigned to
another platform, transport or signing cohort. Raw output stays under ignored
`.local/`. Per-cell selection and partial execution stages are not accepted.

Native/browser, Mac-server/Linux-client, capture, current-authority and
ten-second idle producers are integrated. Their focused checks prove harness
boundaries, not runtime qualification. A configuration passes only after every
required observation satisfies the full contract. The next complete attempt
uses the coordinated library, source, capture and lifecycle corrections.
The owner-approved cache cleanup recovered 66.9 GiB internally; the existing
ARM VM root was expanded to 64 GiB. Check current free space before launch;
previous free-space readings are historical observations. Mac BPF capture
access was restored and remains a runtime preflight observation.

## Browser source operating profile

The next candidate declares `capture_input_mode:
"caller-owned-audio-node+owned-video-track"` in its source evidence and
`audioCaptureMode: "caller-owned-audio-node"` in the original programme record.
Each original synthetic GainNode is passed through the public SDK capture
input. WT uses its existing worklet in the source context; RTC owns a
same-context destination track. Capture retirement removes only that physical
edge. The source owner keeps the original clock and closes the source context
after all physical captures retire.

This is an explicit producer change from the v191 cross-context synthetic
MediaStream capture bridge. It does not revise any retained result. The
62-second programme, 60-second measured window, six sources, 15s/40s movement
windows, 30fps source clock, 100ms pulse edges, and all decoded media/age/coverage
limits remain unchanged. Browser source gain is 0.05 low and 0.2 high, the
separately recorded input-envelope calibration; native source gain is unchanged.
The graph-input feature and component checks do not qualify a transport cell;
the complete next 32-cell run must provide current decoded and lifecycle proof.

## Historical decoded-media investigations

The following entries retain the state and pending actions at each historical
attempt. Current completion and next execution are stated above.

Attempt v163 (`b03fcc00`) connects ten-second idle observations to the same
original local ARM WT EdDSA workload. Four new verifier cases pass on ARM Linux.
All four moves and local capture/renderer/client retirement finish within
356 ms. Five SDK media peers have zero active bindings/queues; physical server
owners and transport pools return to baseline. **Idle restoration fails:**
eight control task entries remain; server descriptors 49 versus 35 initially,
server RSS 124,751,872 versus 62,173,184 bytes, and browser tree RSS
2,157,506,560 versus 1,827,500,032 bytes. Threads/process counts also remain above
baseline. The browser tree includes Node and owned PulseAudio; native helper is
absent. These are sampled numeric observations, not allocation traces or a leak
attribution. No cache allowance is fitted after the result. See
[idle evidence](../evidence/gate-6/matrix-idle-resources.json).

The integrated ledger now records the owned-drain/idle requirement as **failed**
instead of unrun for this attempted cell, independently of media failure. Source
inspection found that periodic control cleanup reaped blocking polls but not
completed outer JoinSet entries. Candidate `2ca6793e` also joins those completed
controls without a fresh command; its new focused production regression is
running locally. Full workload verification of the fix remains pending. The stopped original
v163 database also retains two accepted Sessions: the fixture ended Connections
but omitted its two preallocated Session-end operations. Candidate `4b1a78a3`
ends both Sessions before client closure under the same five-second deadline.
The v165 offline build completed, but its binary selected zero tests: the
runner omitted the required `test-support` feature. This is recorded as
unexecuted, not passed. Candidate `2b4e27f2` enables the feature and requires the
exact regression in the binary test list before execution. The corrected run
and fixture checks remain pending. v164 had stopped before test execution on
a missing locked crate and VM DNS failure. All 75 missing
locked cache files were transferred from the Mac with Cargo.lock SHA256 checks.


Attempt v162 at `47ccc658` reran only the affected local ARM Chrome WT EdDSA
quality case with passive encoder-input diagnostics. All four moves, owned UDP
capture, source/SDK/executable guards and original cleanup pass; all six decoded
quality routes still fail. Audio totals 58.18–59.46 seconds; AV median absolute
skew 98.2–116.7 ms and p95 141.3–159.1 ms. Two routes fall to 21/22 changing
frames in their weakest second. The new encoder-input envelope observer rejects
same-resolution callback times on all six sources, so the numeric input/receive
latencies are provisional and do not establish a causal timing boundary. The
original report is retained unchanged. Subsequent diagnostic reports explicitly
label invalid input or receive envelopes `observer-invalid`; one new focused
regression passes. No additional live run or unrelated suite was launched.

The matrix remains 25 not-run, five failed, two incomplete and zero qualified.
Too much complete-workload integration followed narrow component work. Finish
that integration alongside the actual media fix; failed media cells must not
prevent other executable cells from being attempted.

The new `--stage quality` producer runs the original browser programme with
independent WebCodecs PCM/pixel observers over the declared 1–61 second window
and an owned exact-port UDP capture. At `342e943f`, both local ARM Chrome WT
signing variants complete all four moves and teardown within275ms, and both
capture files validate with zero kernel drops and joined original processes.
All six receive routes **fail** quality. Audio totals58.64–59.58s versus59.9s;
AV median80.7–117.0ms/p95138.4–159.2ms exceed80/120ms. All EdDSA routes meet
24 changing frames/second, while three ES256 routes have weakest seconds22–23.
No complete cell or Gate task is promoted.

[Original results and corrections](../evidence/gate-6/matrix-browser-quality.json)
retain the first failed measurements, corrected observer audio-port mapping,
source-clock sampling correction and scratch-preflight not-run attempt. Source
polling now observes the unchanged30fps AudioContext coordinate every5ms;
all actual source-coordinate skips are zero. It never creates catch-up frames
or rebases the programme. Every regular file in terminal fixturesv150–v154 was
verified against its lossless private archive before removing the expanded tree.
Recent raw failures and captures remain directly accessible.

The current observer counts actual decoded20ms audio frames and evaluates their
spectral identity; it reports the uncovered interval after each20ms buffer.
Video counts only unique changing identified pixels per second, a stricter test
than all decoded frames. These distinctions remain explicit and must not be
silently mixed with native observer statistics. Three new content/coverage tests,
one port/activation-ownership case and one quantized-source-clock case pass;
the affected stalled-clock terminal test also passes. No unrelated suite reran.

The AV markers show **audio arriving after Video**, not the reverse. Next causal
work is to instrument original capture input, encoded submission and decoded
receive times using the same marker ordinals. Inspect the AudioWorklet path,
server audio handling and receive queues before changing buffering. The Video
capture path uses a media element callback, but the observed skew direction does
not establish it as the current cause. Preserve deadlines and original media;
do not compensate with timestamp shifts, padding or relaxed quality thresholds.

## Execution order

Earlier movement-stage execution (November 4): actual Chrome WebTransport EdDSA and
ES256 both pass the six-source, four-move 62-second programme at candidate
`7bbe7700`. Each run completes two outward moves at 15 seconds and two returns
at 40 seconds, retaining the original master clocks while retiring each old
Connection capture. Six original ends and seven client closes finish within
276 ms of the fixed terminal; three explicit keepalive rounds join without
reconnect or credential refresh. The original controller is terminal and gone.
[Seven retained attempts](../evidence/gate-6/matrix-browser-programme.json)
include five failures and these two stage passes. Source, generated SDK and
executable guards pass, and original browser/server owners join for all attempts.

Actual execution exposed two defects. The browser Video allocation validator
hardcoded Connection generation one and rejected valid replacement parameters.
It now binds one consistent generation from the signed original-operation offer
and checks the final movement receipt; new focused cases cover generations two
and three, conflicting port generations and receipt mismatch. Next, the movement
fixture tried to transfer a publication port with no direct subscription. It now
transfers the actual subscribed local receiver, whose ID is the selected source
publication's ID. Both corrections are exercised by the successful actual runs.

The public `session.connections.prepareReplacement` API submits the two
original-root target phases once, installs the signed parameters for real
precommit capture, and waits for the original receipt. The continuing source
owner uses one immutable 62-second clock and nonperiodic 100ms AV markers;
missed source frames are recorded, never synthesized. Actual source draws are
**not independent decoded-media quality evidence**.

Remaining integration steps:

- Resolve the decoded audio/AV and remaining Video failures in the new quality
  producer, which now observes all six logical receive routes across replacement
  and return, with exact owned packet capture. Its passing movement/capture
  stages cannot override failed quality measurements.
- Add actual current-authority expiry/conflict/fallback checks and ten-second
  idle-resource restoration to the same complete-cell verifier.
- Implement the WebRTC target adapter lifecycle. The service still refuses
  WebRTC replacement in `canonical_replacement.rs`; allocation, negotiated
  media, private readiness, commit and drain must all exist before its programme.
- Execute remaining runtime/network cells. Preserve source-specific observations;
  the mixed-source progress ledger cannot qualify a frozen acceptance candidate.

The preceding source/keepalive increment ran only new local focused checks: ten signed-control SDK
keepalive cases and four source ownership/clock boundary cases. Eight keepalive
cases passed on the initial run; two test assertions incorrectly compared a
null-prototype wire object with a plain object and passed after correcting the
assertion (only those two reran). Source cases cover cloned-track retirement,
switch/return ownership, missed-frame accounting, stalled-clock terminal and
capture/close races. These are in-memory boundary fixtures, **not actual device
movements or decoded media evidence**. Earlier admission suites were not rerun;
no matrix cell or Gate task was promoted.

1. Probe both actual local hosts at the committed candidate. Record tool and
   frozen browser versions, scratch capacity and systemd availability. The
   current ordinary Ubuntu container has `docker-init`, so it cannot stand in
   for the specified systemd/isolated-client-namespace Remote profile. The parent
   Colima VM already runs Ubuntu24.04 ARM64/systemd and has namespace tools and
   noninteractive sudo. Its pinned toolchain is now connected, and the first
   actual systemd/native Remote workload has run with separate client namespace.
   All four moves completed; all six media routes failed quality. Namespace,
   service and clients retired without forced cleanup. No replacement VM was used.
2. Execute available native signing/platform cohorts independently, starting
   with Ubuntu local ES256. Preserve the failed EdDSA observation; do not rerun
   it merely to obtain a green matrix. Run other cohorts as their environments
   become ready. Build once per current candidate through Cargo's content-bound
   local cache, retaining the source/executable guards in each workload.
3. Connect the existing real Chrome WT, Chrome WSS/WebRTC and Firefox browser
   owners to the six-connection, four-participant, two-Session programme. Use
   actual browser media, both signing cohorts, normal CA trust, authenticated
   selection and fallback/refusal; retain the common source clock and four
   moves. The native peer and Node control component are not browser substitutes.
4. Complete the local remote-profile environments and bind their actual
   addresses, systemd service, client namespace and impairment to each run.
   Named external hosts, provider accounts and G6-054 are outside this execution.
5. Connect fresh current-authority expiry/conflict observations, original-owner
   teardown and ten-second idle resource restoration to each cell. The deadline
   starts at the original terminal intent, not after output hashing or cleanup.
   Independently validate every required check, capture, decode and profile
   budget before qualifying any cell. Do not inherit a requirement from another
   platform, transport, signing cohort or source revision.
6. Use matrix failures to choose focused corrections and validate those fixes.
   Then rerun all 32 configurations in one fresh complete attempt. Close G6-058
   only when all 32 satisfy the complete contract.

## Commands

Run from the selected checkout, using a fresh output directory each time:

```sh
python3 scripts/run-gate6-transport-matrix.py --phase probe \
  --output .local/gate-6/matrix-candidate/host-probe

python3 scripts/run-gate6-transport-matrix.py --phase run \
  --output .local/gate-6/full-matrix-next

python3 scripts/run-gate6-transport-matrix.py --phase merge \
  --host-report .local/gate-6/matrix-candidate/mac/matrix.json \
  --host-report .local/gate-6/matrix-candidate/linux/matrix.json \
  --output .local/gate-6/matrix-candidate/combined
```

Run requires matching clean commits on the Mac and `/workspace` in the existing
`connect-gate6` ARM VM. The coordinator starts both original host workers; do not
launch them separately. It uses the exact installed ARM browser image recorded
in the coordinator. Optional `--chrome` and `--firefox` paths select the Mac's
frozen browser binaries. Mac executable overrides are not forwarded to Linux
clients; those clients use their pinned Linux bundle. Each fixture verifies the
actual browser version.

Every run includes all 32 configurations. Partial `--stage` values are accepted
only with `--phase probe`, which executes no workload. Run prepares dependencies
and builds the current TypeScript SDK before the guarded workloads. Producers
retain actual capture and original process ownership. Build/test sources must
stay unchanged until the original controller terminates. `state.json` records
its PID; verify that actual process when checking a running job. A stale state
file alone does not prove liveness.

Probe and merge exit zero only for a successfully generated ledger. Run exits
nonzero until the complete matrix qualifies. Inspect `cells[].check_results`
for each required verdict and `cells[].execution.component` for the original
component report binding. An exited worker's unfinished rows are blocked
without manufacturing attempts or behavior evidence. No command here dispatches
GitHub Actions or provisions cloud resources.

## Browser programme implementation

The admission library (`scripts/lib/browser_matrix_admission.mjs`) now defines
seven actual browser SDK owners, membership/audience changes, original operation
reference recovery, two accepted/upgraded Sessions, and six audio/video offers
with current Session-scoped subscriptions. It preallocates the four moves at
15/40 seconds on the original 62-second programme, and journals each original
mutation before submission. It returns the live owners for the remaining media
programme; its admission-only retirement retains the original five-second
acceptance deadline and joins cleanup even after that deadline fails.

The private actor exporter (`scripts/lib/browser_matrix_actors.py`) checks the
selected CLI installation's identity, credential, signing cohort and protected
asset paths. The Node key adapter (`scripts/lib/browser_matrix_keys.mjs`) checks
that each converted private key derives the enrolled public key; it supports the
actual Ed25519 RFC5958 fixture format and ES256 without exposing key material in
public reports. Both retained signing cohorts exported/imported successfully.

Six initial focused checks pass for intent persistence, uncertainty, original
references, deadline/cleanup ownership and signing-key conversion. Key checks
also pass on Linux Node22.16.0. Three additional actor-export checks pass for
private-file boundaries, issuance credentials and signing cohorts. Those are library checks. Actual
Chrome admission is now connected to the matrix runner and has passed both WSS
signing cohorts at `a7b85a59`; it does not qualify a cell. The complete 60-second media, four-move,
expiry and idle observations remain required. See
[the bounded implementation evidence](../evidence/gate-6/browser-matrix-admission-candidate.json).

The first [systemd/namespace Remote EdDSA result](../evidence/gate-6/matrix-native-remote-eddsa.json) completes the control/movement programme but fails independent media quality. Its cold VM-host build is retained for the next cohort; the attempted cache bind did not expose the container cache and is not claimed as reuse. Full original private media and captures remain on the owned VM.

The independent [Remote ES256 workload](../evidence/gate-6/matrix-native-remote-es256.json) also completed all four moves/control checks and original cleanup, but all six quality routes failed. The current progress ledger records three attempted cells across their identified revisions, with zero qualified. No further native rerun is justified merely by these failures; continue browser launcher/media integration.

## Historical Chrome admission implementation

The production fixture/owned launcher is now `scripts/rehearse-gate6-browser-matrix.py`.
It initializes a new CLI instance, creates a normal CA/leaf pair, configures WT,
WSS/WebRTC and the HTTPS application surface, seals a stopped backup through the
existing materializer, and enrolls seven separate Devices through the public CLI.
Media endpoint verification names the actual native signed-device proof; the
subsequent browser authentication is a separate observation. An untrusted fresh
Chrome profile must refuse the CA before another fresh profile imports it.
The actual browser then uses the public BrowserSdkClient to run admission.

Admission is now part of every full run. To inspect only its historical stage
wiring on the local Ubuntu ARM host without executing a workload:

```sh
python3 scripts/run-gate6-transport-matrix.py --phase probe --stage admission \
  --browser-image sha256:2856c7b7ae4eea13ba0fb4a5605b4f182557f0b1842f45efebd2c03ea66e9e6c \
  --output .local/gate-6/browser-admission-wiring-probe
```

`chrome-wss-webrtc` selects the independent WSS/RTC admission path. This stage
retains full media, four moves, fallback/refusal, expiry and idle requirements as
unexecuted. A green admission component is incomplete, never a cell pass.
All synthetic credentials, TLS keys, original operation references and raw logs
stay private. The image uses the VM host network for this explicit Local scope;
its actual ARM ELF and frozen browser version are checked. No host personal
profile, camera, microphone, provider or account is used. The owned PulseAudio
null sink supplies a real-time output clock. Its short private mount alias
respects Linux Unix-socket path limits and joins the original daemon afterward.

The actual [browser observations](../evidence/gate-6/matrix-browser-admission.json)
retain eleven attempts and their sources. The corrected WSS SDK preserves a
signed `501 unsupported_feature` as an explicit refusal; unrelated `503` and
unrecognized `501` failures retain uncertainty. Six new signed control checks
prove the before/fix boundary on both transport adapters and pass on Linux.
Four additional matrix-stage boundary checks and one real UDP STUN/socket check
also pass on both local hosts. Capture startup now follows accepted-reference
persistence and precedes the offer-readiness wait. Both WSS cohorts complete all
six current AV offers/subscriptions with active synthetic capture, six original
ends and seven client closes in less than 300 ms. This is startup/retirement
evidence, not decoded quality or the full movement programme.

The earlier WT private-CA attempts failed before authentication: Chrome QUIC
rejects locally added root trust even though HTTPS/WSS accept it, consistent with
[Chromium's documented local trust policy](https://chromium.googlesource.com/chromium/src/%2Bshow/main/net/data/ssl/chrome_root_store/faq.md).
The earlier statement that a publicly trusted certificate was required for this
local fixture was too restrictive. The existing SDK supports the standard
WebTransport `serverCertificateHashes` mode. Before v154, the operating profile
records its exact fresh one-day ECDSA P-256 leaf trust, independent of the enrolled
server signing key. The pinned leaf is checked against the certificate actually
served; a wrong hash must fail before any application stream or mutation.

Both actual Chrome WT signing cohorts now pass admission with this explicit local
trust mode at `b7b64d2a`, including the two wrong-pin negatives, seven enrolled
Devices, six active AV captures/current subscriptions and original retirement.
See [the source-bound WT result](../evidence/gate-6/matrix-browser-pinned-wt.json).
This clears the local certificate prerequisite, not full-cell qualification or
a public-CA deployment. HTTPS/WSS keep normal profile CA validation. No global
certificate-error bypass or developer-mode flag is used. The standard defines
[certificate hashing and its short-lived certificate requirements](https://www.w3.org/TR/webtransport/#dom-webtransportoptions-servercertificatehashes).
Let's Encrypt cannot issue for `localhost`; it is unnecessary for this declared
local trust mode. See [Let's Encrypt's localhost documentation](https://letsencrypt.org/docs/certificates-for-localhost/).


November 4 continuation: v166-prechecks passes four new/affected checks on the
local ARM VM at 6c596c55 (original Session ends/deadline, original idle clock and
the independent idle evidence boundary). The corrected test-support Rust build
is still running; no pass is inferred from compilation. Candidate 67762f43 adds
actual browser fallback to the admission workload before the fixed media
programme: owned UDP silence forces WSS, owned TCP refusal makes both paths
unusable, and the original admission create receipt is read before/after the
refusal without submitting another mutation. Failed physical owners and both
fixture listeners must join. This is control-selection evidence only, not WSS
media qualification. Its two new focused checks and actual Chrome run are
pending. Existing passing cleanup checks will not be rerun.


Later November 4: the original v166 completes successfully in 860.09 seconds,
with exactly one selected Rust regression passing in 0.16 seconds, no ignored
cases, and unchanged source/executable guards. v167 at 90038f24 passes its two
new unavailable-socket/evidence-boundary checks and is building the production
CLI for one affected local ARM Chrome WT EdDSA quality workload. No prior
passing cleanup tests are repeated. The full workload remains pending; neither
focused check proves actual browser fallback. Its original controller is 30874.


Latest November 4 result: v167 at 90038f24 passes both actual browser selection
requirements (WT unavailable selects WSS with a timeout reason; neither usable
path explicitly refuses without resubmission). Two original-operation reads
recover the admission create receipt; four physical probe owners and fixture
sockets join. These checks qualify only this candidate/cell. The full run then
fails one first move at initial mute/Authority, while the other first move
completes. No complete decoded window or ten-second terminal idle observation
is available; v163 remains the last full quality/idle observation.

Candidate 368e35b5 captures source/retained references before the once-only
replacement start effect; original Store read/current binding/cancellation and
deadline checks remain mandatory. A new initial-mute registry-contention case
is compiling as v168 on the owned ARM VM. It compares old lookup refusal with
current captured-owner success and expired-invocation refusal, then requires
the same original replacement and decoded video. This tests a concrete
mechanism; the original initial-mute log alone does not prove the exact
authority subcheck responsible. No repeat of previously passing cases or
GitHub execution is requested.
