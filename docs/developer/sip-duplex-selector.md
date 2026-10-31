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
