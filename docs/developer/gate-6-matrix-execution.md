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
   for the specified systemd/isolated-client-namespace Remote profile.
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
