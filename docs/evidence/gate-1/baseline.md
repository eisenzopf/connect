# M0 — reproducible source and regression baseline

Command: `bash scripts/check-baseline.sh` completed successfully on the environment
recorded in `environment.md`. It verifies the prepared source graph first and
uses the pinned upstream lockfiles; Connect's own lockfile remains separately
recorded. No live providers or remote deployment were exercised.

| Suite | Passed | Failed / ignored |
|---|---:|---:|
| QUIC full-stack / loopback | 1 + 4 | 0 / 0 |
| Core lifecycle / pagination / messaging | 8 + 1 + 10 | 0 / 0 |
| UCTP coordinator / datagram / pending dispatch | 45 + 3 + 3 | 0 / 0 |
| SIP library | 829 | 0 / 0 |
| WebRTC data adapter / credential diagnostics | 5 + 4 | 0 / 0 |
| Parley API / conference / create | 10 + 16 + 2 | 0 / 0 |

Total: **941 passed**. Upstream warnings were emitted, including Parley's unused
field; no failed test was suppressed. This is a focused reuse baseline, not an
assertion that every test in each upstream repository ran.

Pinned revisions and patch digests: `dependencies/source-lock.json`.
Native authenticated dispatch proof: `dispatch.md`. Experimental application
profile, fixture results, state ownership and limits: decisions 001/002 and
`contracts/connect-control-v1.md`. These resolve the initial application-hook
question; the native transport itself was already available in rvoip.

M0 is closed. Subsequent milestones must prove Connect's durable behavior,
transport parity, public SDK and packaged local journey; this baseline does not
mark those gates complete.
