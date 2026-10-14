# 012 · Proposed Gate 5 impaired-network acceptance

Status: proposed, awaiting the owner's decision. This document does not amend
Decision 011, change test verdicts, or qualify Gate 5.

The owner expects video to have problems under poor network conditions. D5-01
requires at least 24 decoded fps in the clean profile and bounded recovery under
loss and delay variation. Decision 011 additionally requires A/V alignment with
an 80 ms median and 120 ms p95 limit, without an impairment exception. The
current checkers also require 0.7 uniquely paired markers per measured second.

## Proposed acceptance rule

Keep the existing clean 60-second qualification on all three named paths:
640×360 VP8, at least 24 independently decoded moving fps, identified duplex
audio, valid coded calibration, at least 42 paired A/V markers, and the existing
80 ms median / 120 ms p95 synchronization limits. Camera/microphone mute and
other failure, authorization, lifecycle and recovery checks retain their rules.

For the existing 60-second impaired scenario, keep the exact 3% loss,
0–100 ms delay/reordering and two 250 ms interruptions. Require every existing
non-A/V acceptance predicate, including:

- Identified duplex audio, correct source/stream identity and actual decoded
  moving video at the existing impaired floor of 1 fps.
- Continuous fresh-motion checks and recovery within two seconds of each
  interruption, rather than a single frame followed by a freeze.
- Authenticated media on the selected impaired hop, current authorization,
  bounded queues and packet/frame age, required forwarding observations, and
  the existing resource/network ceilings.
- Paired teardown within five seconds, durable Conversation continuity and
  independent reliable-control behavior.

During that impaired window, report reduced frame rate, measured A/V skew and
unavailable A/V calibration as degraded quality. Neither a measured excursion
above the clean A/V limits nor insufficient impaired A/V samples would alone
block acceptance. This explicitly permits degraded lip-sync under impairment;
it does not assert synchronized A/V when the evidence is unavailable. Malformed
observer inputs, wrong source identity, broken audio, failed recovery, resource
violations and cleanup failures remain failures.

This proposal does not promise restoration of full clean quality within two
seconds after all network faults cease. That claim would require a separately
specified same-call recovery window and additional evidence. The two-second
requirement here remains the existing fresh-motion interruption recovery rule.

## Evidence and implementation boundary

The retained 98 native/browser impairment report has 30 calibrated A/V pairs,
34.17 ms median and 114.65 ms p95 skew. Its insufficient sample coverage fails
the current checker. The retained 7f browser/browser sparse direction has no
valid A/V estimate; its failed cadence calibration is not proof of acceptable
sync. Both reports retain their original failed verdicts.

Chrome 153's clean failure remains a failure under this proposal. It has 84
calibrated pairs per observer, with medians of 89.8 and 99.15 ms. An impairment
exception cannot qualify that clean result.

If approved, implementation must record the revised qualification policy in
new reports, keep A/V component measurements and limitations visible, and
update native/browser and installed acceptance consistently. Fresh tests and
an identified matching package are required. No prior report is rewritten or
retrospectively promoted. All three paths, complete installed qualification,
actual-model work, three independent human walkthroughs and final owner review
remain required. Publication remains a separate action.

References: [approved profile](011-gate-5-profile.md),
[product plan](../connect-gate-5-plan.html#video),
[retained media evidence](../evidence/gate-5/release-media-diagnostics.json),
and [Chrome compatibility evidence](../evidence/gate-5/browser-version-compatibility.json).
