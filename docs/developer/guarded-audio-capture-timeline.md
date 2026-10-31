# Guarded audio capture timeline

G6-054 requires decoded duplex audio and truthful audio/video clocks through
network loss and recovery. The mixer consumes at most one 20 ms output quantum
per tick. Its packet-age limit, bounded PCM queue, and skipped-executor-tick
policy remain part of that contract.

An empty input tick and missing source PCM are different observations. If a
source's next original packet is contiguous, the mixer has delayed that PCM;
it has not lost a source interval. Advancing output RTP by two quanta while
advancing the original capture clock by only one creates inconsistent clock
coordinates. When actual PCM is dropped, the output clock must preserve the
corresponding source gap.

The regression fixture uses the actual guarded mixer and original queued-media
delivery. It publishes a 20 ms PCMU source packet, executes an empty tick, then
publishes the next contiguous original packet. It checks capture NTP and output
RTP independently, including source RTP wrap and 8/48 kHz mixing. Its result is
separate from a browser or telephone media qualification.

The proposed correction retains numeric source coordinates per output binding:
source identity and generation, codec/mixer rates, original RTP position, PCM
offset, and rational conversion remainder. Contiguous source media advances the
output clock continuously. Real source gaps and discarded PCM advance it by the
actual missing media duration. Sender-report arrival or NTP correction must not
rewrite that audio sample timeline.

This metadata holds no packet, route lease, clock predicate, or authorization
reader between ticks. Every output still uses current source, receiver, scene,
subscription and route checks. Binding replacement resets the numeric mapping.
Unsupported or multiple source timelines use the existing mixer clock; no
capture anchor is invented for them. An empty input tick produces no PCM.

The existing real missed-quanta, packet-age, trimming, selection, cancellation,
revocation, and clock-lifetime tests must continue to pass. Additional checks
cover true missing source intervals, partial packets, pending sender reports,
RTP wrap and codec-rate conversion. Neither a passing Core regression nor
receiver concealment statistics establish the cause of a browser quality
failure. The original decoded-audio, AV timing and network recovery checks must
also run without changing their thresholds.

Current public runtime evidence:
[`linux-canonical-picture-runtime.json`](../evidence/gate-6/linux-canonical-picture-runtime.json).
The original-code counterexample is source-verified: run `41195276063` at
`1f3d766d` fails its first 8 kHz output RTP assertion after the capture NTP
assertion passes. The other 30 selected Core functions pass. The 48 kHz loop
is unrun after that failure. The correction at `31a7f613` passes all 32 selected actual Core functions in
run `41196260443`, including both rates in the original regression and the
added true-gap/pending-report/partial/wrap/rate-conversion cases. Browser and
transport quality qualification remains separate. See [`guarded-audio-capture-timeline.json`](../evidence/gate-6/guarded-audio-capture-timeline.json).

The six native network cases and both original signed AV functions plus ten
Core repair regressions also pass after the correction. The strict browser
diagnostic still fails AV timing: both baseline directions and one of four
direct-bridge observations exceed 120 ms p95, with zero bad audio samples in
all six observations. These failures remain open in G6-054.
