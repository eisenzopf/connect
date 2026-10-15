# Gate 5 candidate — final owner handoff

Candidate **284cef16c64b324ce2add037b013249692a60087**, version
**0.1.0-alpha.1**, is owner-approved and release-ready for **local macOS arm64**.
**50/50 implementation tasks and 16/16 acceptance criteria are complete at
their stated scopes.** The owner approved this sealed candidate on October 15,
2026: “yes i approve”. G5-050 and A5-16 are complete. The candidate is unpublished.

## Completed work

- The owner confirmed G5-046 usability completion and G5-048 quality review
  completion in this chat. Those confirmations are recorded separately from
  automated evidence. Detailed participant timings and final reviewer findings
  were not supplied; no three-person measurement dataset or new review run is
  claimed by this handoff.
- Final documentation, roadmap and all F-01–F-21 assignments are reconciled.
  The [acceptance report](evidence/gate-5/acceptance.html) and
  [acceptance map](gate-5-acceptance.json) expose the qualification basis.
- The [technical evidence seal](evidence/gate-5/final-candidate-seal-20261015.json)
  binds candidate hashes, evidence and current documentation. Later changes
  require an updated supplement/seal and affected retests when behavior changes.

## Candidate identity

| Archive | SHA-256 |
|---|---|
| `connect-0.1.0-alpha.1-macos-arm64-284cef16c64b-gate5-local-qualification.tar.gz` | `3fc18ca1d734ddec7eb47888898ab22e95668166eb8a2dc5792af9c3282d58de` |
| `connect-0.1.0-alpha.1-macos-arm64-284cef16c64b-gate5-local-source.tar.gz` | `a7011377b39e0269aa2786ae718360dbe34c28c79131718ed00db2a3e76f043e` |
| `connect-0.1.0-alpha.1-macos-arm64-284cef16c64b-gate5-local.tar.gz` | `a26f107672df840b2f16933dbff44bcff0640b407d3379d2c234752b6380a701` |

Installed manifest SHA-256:
`da8e1b67ca557211fc2a394ba5fe84e4b9b504178e64917673494f5e3205418a`.

Archive hashes are retained from the independently accepted installed report.
The archives are absent from this checkout; finalization verified the recorded
proof hashes and Git runtime-source identity, without rehashing or rebuilding
those archives. Distribution must use these exact archives and verify their
bytes before upload. The current source checkout and this documentation
supplement are later than the immutable candidate source archive.

## Evidence and limits

The [complete installed proof](evidence/gate-5/complete-installed-284cef1.json)
records 18 baseline checks, 26 Gate 5 phases, all nine media conditions and both
strict inventories. All 415 selected registry dependencies have pinned notices.
This finalization changes documentation/evidence only and makes no new runtime,
model or provider execution claim.

Source qualification retains the historical 84-phase run and reviewed deltas;
a new full 85-phase source run is not claimed. The 28 actual-model calls ran on
ab843b8 and apply through the accepted
[unchanged-runtime composition](evidence/gate-5/actual-model-composition-284cef1.json).
The [six provider claims](evidence/gate-5/provider-claims-ab843b8.json) retain
their original live-execution sources and limits.

Supported video is one simultaneous two-party VP8/PCMU call across native/native,
native/browser or browser/browser. Browsers use WebRTC; native clients use QUIC.
All 18 policy decisions passed. Two impaired raw A/V quality observations remain
advisory under Decision 012; clean/mute quality and all non-A/V hard checks remain
strict. Generated-media evidence does not establish physical-device behavior.
No multiparty, screen-sharing, SIP/PSTN video, Vapi vision, automatic model wake,
other-platform or remote-deployment support is added.

See [release notes](GATE5_RELEASE_NOTES.md), [package instructions](GATE5_PACKAGE.md)
and [local installation/recovery](LOCAL_RELEASE.md) for compatibility and setup.

## Owner decision and distribution

The owner explicitly approved the exact candidate and its documented limits on
October 15, 2026. No remediation was requested. The
[approval record](evidence/gate-5/owner-approval-20261015.json) binds the exact
reviewed seal and handoff hashes. G5-050 and A5-16 are complete; this local
candidate is release-ready.

Publication requires a separate instruction. The proposed distribution is the
three immutable archives above with matching checksums, release notes and the
current evidence/documentation supplement at an owner-selected destination.
No destination, upload, public-site change or deployment has been performed.
