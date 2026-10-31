# Owned SIP duplex selection

G6-054 requires decoded media in both directions. The owned SIP bridge has
separate incoming and outgoing queues, with cancellation and lifecycle changes
checked first. Its original biased select always checked incoming media before
outgoing media. A ready incoming backlog could therefore postpone every ready
outgoing frame until the backlog was drained.

The correction alternates data priority after each selected frame. When both
queues stay ready, each direction receives service after at most one selection
from the other direction. Either direction also runs independently. The existing
outer cancellation/lifecycle priority and all per-frame current ownership,
authorization, codec and actual-delivery checks remain in force. No worker,
queue, reader, delay, or packet-age allowance is added. Original worker completion
and port cleanup still own the same resources.

The conformance integration target includes the exact prepared production
selector module. Its three unchanged assertions cover both ready-queue fairness
bounds, single-direction FIFO and cancellation of an idle wait. Source and binary
hashes, exact test inventory and original test results are retained. The runner
executes each function separately within one absolute 30-second test budget; a
failed batch cannot invent per-function failures from unavailable stdout.

Run the source-bound synthetic component with the existing Linux fixture
workflow's `sip-duplex` lane. The ordinary full lane also requires this component.
The original `telephone-network` lane separately checks actual SIP/RTP, journal
ordering, decoded opposite tones, replay, signed end, original joins and owned
bridge removal. Its thresholds and deadlines remain unchanged.

The selector counterexample is verified at `15a5675d` in run `41197830642`:
the actual aggregate is one pass and two fairness failures. The initial
per-function boolean report is rejected because of the batch-output handling
error. A passing selector test cannot qualify a telephone call or establish the
cause of a separate telephone failure. See
[`sip-duplex-selector.json`](../evidence/gate-6/sip-duplex-selector.json).

The full telephone run at `e6f9c6e9` (`41198516894`) still fails the
unchanged native input minimum in both EdDSA direct and loss cases, as did the
comparable `15a5675d` run (`41198159725`). ES256 was not exercised by that
stop-on-first-failure harness. All original processes and the NAT worker joined,
and the empty bridge was removed. This rejects a telephone completion claim.

The runner now records fixed-enum counts from existing SIP admission, Core
queue/delivery and native egress trace messages, plus the actual native receive
capture byte count. Raw logs and identifiers stay private. Both algorithms run
independently with their original assertions and deadlines; a failed case cannot
hide the remaining algorithm. Tracing may affect timing and provides diagnostic
evidence only. No production media behavior or acceptance limit changes.

Linux source fixture caches now save under the same lane-specific key used for
restoration, after completed compilation with unchanged source inputs. A behavior
failure can retain the completed build; a failed compilation cannot mark it
cacheable. Source content/mode checks still govern restored timestamps. The
change has no measured build-speed claim until a subsequent run proves reuse.

Source `12231a45`, run `41203387431`, executes all four original telephone
cells. All fail the unchanged native input minimum, with3,360–6,400 actual
received bytes. SIP receives173–200 ingress attempts per cell, admits68–91
and refuses96–111. Core refusals are predominantly `NotReady`. This rejects
missing transport or absent native egress as a complete explanation; it does
not identify the specific admission cause. Existing initial-SIP Store authority
trace messages are now reduced to fixed counts (`authority-busy`, `forbidden`,
`not-ready`, admitted or other) before choosing a production policy change.

The first Store-counter run (`3e69b274`, `41206380817`) verifies a cached
hosting build in0.466s. Its two direct cases fail earlier at the original
subscription; both loss cases still fail native input. The new Store counters
are rejected: `tracing` records optional string values as bare fields and omits
`None`, while the initial reducer expected debug `Some(...)`/`None`. The
corrected reducer accepts both actual and debug formats and preserves unknown
values only as `other`. It does not infer Store contention from the rejected
counter report or change any production media behavior.

The corrected Store-counter source `c05ac310` (`41206644225`) verifies
564–625 actual `authority-busy` initial-SIP reads per case. The native path
already retries only that error in a ten-millisecond acquisition budget; initial
SIP used one try. The candidate reuses that existing helper before a lease is
created. Every attempt revalidates the original runtime/origin/current Store;
forbidden, not-ready and other permanent errors are returned immediately. No
positive grant, SQL lock or physical IO is retained or retried. Actual Core
admission still applies its current gates and original packet/IO bounds.

The telephone CI lane now also executes the two existing helper tests for fresh
authorization, immediate permanent refusal and an expired original acquisition
budget. Those and all four unchanged phone cells must pass on the candidate;
format and Python boundary checks alone do not qualify this correction.

The bounded retry is rejected and reverted after source-verified run
`41207024225` at `b31d00c1`. Both existing helper tests pass, but all four
original telephone cells fail. The three media cases retain only800,640and1120
native bytes and encounter9,22and24native egress deadline refusals; direct
ES256 fails at the original subscription. Store-busy counts fall, but that
metric does not establish useful delivery. Hosting compilation took1367.7s and
the newly required service test compilation/execution1206.1s; the behavioral
failures remain explicit. No thresholds or call assertions are weakened.

The initial public SIP read still uses the command SQL mutex, whereas current
native packet metadata already uses bounded read-only WAL observers and seals
SQL/clock versions under a fresh complete authority interval. The next change
targets that read path instead of waiting in the synchronous media callback.
