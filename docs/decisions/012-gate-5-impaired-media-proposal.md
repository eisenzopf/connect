# 012 · Approved Gate 5 impaired-network acceptance

Status: approved by the owner on October 15, 2026. The owner instructed:
“Relax quality expectations under poor network conditions”. The approved scope
is the impairment-only rule below, identified by qualification policy
`connect-media-acceptance/2`. It amends Decision 011's impaired A/V acceptance
requirement; it does not change the `connect-video-vp8/1` wire/media profile,
strict clean or mute rules, or any existing non-A/V hard check. Approval is not
qualification, task completion, owner acceptance of a candidate or publication.

The proposal and its earlier failed evidence remain historical. No old report
is rewritten or retrospectively promoted by this decision.

The owner expects video to have problems under poor network conditions. D5-01
requires at least 24 decoded fps in the clean profile and bounded recovery under
loss and delay variation. Before this amendment, Decision 011 also required
A/V alignment with an 80 ms median and 120 ms p95 limit without an impairment exception. The
original checkers required 0.7 uniquely paired markers per measured second.
Those strict requirements remain for clean and mute scenarios.

## Approved acceptance rule

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
above the clean A/V limits nor validly derived insufficient, ambiguous or missing
impaired cadence measurements alone blocks acceptance. This explicitly permits
degraded lip-sync under impairment;
it does not assert synchronized A/V when the evidence is unavailable. Malformed
observer inputs, wrong source identity, broken audio, failed recovery, resource
violations and cleanup failures remain failures.

This decision does not promise restoration of full clean quality within two
seconds after all network faults cease. That claim would require a separately
specified same-call recovery window and additional evidence. The two-second
requirement here remains the existing fresh-motion interruption recovery rule.

## Evidence and implementation boundary

The retained 98 native/browser impairment report has 30 calibrated A/V pairs,
34.17 ms median and 114.65 ms p95 skew. Its insufficient sample coverage failed
the original checker. The retained 7f browser/browser sparse direction has no
valid A/V estimate; its failed cadence calibration is not proof of acceptable
sync. Both reports retain their original failed verdicts.

Chrome 153's clean failure remains a failure under this decision. It has 84
calibrated pairs per observer, with medians of 89.8 and 99.15 ms. An impairment
exception cannot qualify that clean result.

Implementation must record `connect-media-acceptance/2` in new reports and
apply it consistently to native, browser and installed acceptance. Each
observation retains the unchanged raw `av_sync` result and adds a separate
`av_acceptance` object with `policy`, `scenario`, `status`, `quality_status` and
`reason`. The acceptance object cannot replace, erase or relabel a raw failed or
unavailable quality measurement. Invalid raw observer inputs or invalid derived
results remain failures; the advisory exception applies only to valid impaired
quality measurements and their availability.

Fresh policy guards, all required clean/impaired/mute route checks and an
identified matching candidate/package qualification are required. Earlier
reports without this policy field cannot count as new-policy evidence, even if
inspection suggests their only failure would now be advisory. No prior report is
rewritten or retrospectively promoted. All three paths, complete installed qualification,
actual-model work, three independent human walkthroughs and final owner review
remain required. Publication remains a separate action.

References: [approved profile](011-gate-5-profile.md),
[product plan](../connect-gate-5-plan.html#video),
[retained media evidence](../evidence/gate-5/release-media-diagnostics.json),
and [Chrome compatibility evidence](../evidence/gate-5/browser-version-compatibility.json).
