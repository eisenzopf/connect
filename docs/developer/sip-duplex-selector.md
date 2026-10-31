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
