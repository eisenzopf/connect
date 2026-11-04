# Gate 6 transport matrix execution

The 32-cell matrix is the acceptance framework for G6-058. A failure in one
cell must not postpone attempts in other executable cells. Workloads run on the
owner's Mac Studio and local Ubuntu24.04 ARM64 guest. G6-054 remains deferred.

The [current 32-cell table](g6-058-matrix-status.html) links the actual attempts
and prerequisites. The first Linux ES256 attempt stopped on a read-only
`stream.subscribe` deadline during the movement programme; original owners
joined. This is distinct from the earlier EdDSA quality failure. The Mac has
both frozen browser versions available, but its BPF capture devices currently
require permission before the complete captured workload can run.

`scripts/run-gate6-transport-matrix.py` connects the existing native media
workload to exact matrix cell IDs. It retains all 32 rows, saves each original
attempt before and after execution, continues after a failed cell, and merges
reports from the two local server platforms. It rejects stale candidates,
changed contracts, duplicate host reports and native evidence assigned to a
browser or another signing cohort. Raw output stays under ignored `.local/`.

A native workload success is **incomplete**, not a complete-cell pass. Its
expiry/conflict and ten-second idle observations still need to be connected to
the complete-cell verifier. Every required check remains visible. Browser
workloads and Mac-server/Linux-client full workloads still need integration;
installed browsers and earlier narrow browser results do not fill those rows.

## Execution order

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
6. Use matrix failures to choose focused corrections, then rerun affected cells.
   Close G6-058 only when all 32 cells satisfy the complete contract.

## Commands

Run from the selected checkout, using a fresh output directory each time:

```sh
python3 scripts/run-gate6-transport-matrix.py --phase probe \
  --output .local/gate-6/matrix-candidate/host-probe

python3 scripts/run-gate6-transport-matrix.py --phase run \
  --cell G6-058/ubuntu24-arm64/local/native-quic/ES256 \
  --output .local/gate-6/matrix-candidate/linux-es256

python3 scripts/run-gate6-transport-matrix.py --phase merge \
  --host-report .local/gate-6/matrix-candidate/mac/matrix.json \
  --host-report .local/gate-6/matrix-candidate/linux/matrix.json \
  --output .local/gate-6/matrix-candidate/combined
```

Multiple `--cell` arguments run sequentially; a failure does not stop later
cells. Selection is explicit, so no earlier passing suite is silently rerun.
Run prepares dependencies and builds the current TypeScript SDK before the
guarded workload. Existing native producers retain actual capture and original
process ownership. Build/test sources must stay unchanged until the original
controller terminates. `state.json` records its PID; verify that actual process
when checking a running job. A stale state file alone does not prove liveness.

Probe and merge exit zero only for a successfully generated ledger. Run exits
nonzero while complete-cell qualification is unavailable, even if its native
component passes. Inspect `cells[].execution.component.passed` for the component
outcome and `cells[].execution.blockers` for prerequisites. No command here
dispatches GitHub Actions or provisions cloud resources.

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

## Execute actual Chrome admission

The production fixture/owned launcher is now `scripts/rehearse-gate6-browser-matrix.py`.
It initializes a new CLI instance, creates a normal CA/leaf pair, configures WT,
WSS/WebRTC and the HTTPS application surface, seals a stopped backup through the
existing materializer, and enrolls seven separate Devices through the public CLI.
Media endpoint verification names the actual native signed-device proof; the
subsequent browser authentication is a separate observation. An untrusted fresh
Chrome profile must refuse the CA before another fresh profile imports it.
The actual browser then uses the public BrowserSdkClient to run admission.

On the local Ubuntu ARM host, with the existing exact ARM browser image:

```sh
python3 scripts/run-gate6-transport-matrix.py --phase run --stage admission \
  --browser-image sha256:453a491700f0681a6595de797f97a993a68bfa5d0834d5909c3bcdbadf28a42b \
  --cell G6-058/ubuntu24-arm64/local/chrome-wss-webrtc/EdDSA \
  --cell G6-058/ubuntu24-arm64/local/chrome-wss-webrtc/ES256 \
  --output .local/gate-6/browser-matrix-candidate
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
