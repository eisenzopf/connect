# 015 — Gate 6 operating and qualification profile

2026-10-15; G6-011. These are implementation targets, not measured results.

## Runtime and topology

Local development/first run: macOS arm64. Baseline observed macOS and toolchain
versions are recorded in [baseline](../evidence/gate-6/baseline.json). Rust/MSRV
is the repository's 1.97.1 toolchain for this increment. Python minimum is 3.12;
Node minimum is 22. Supported package versions and actual runners are recorded
by candidate qualification.

Hosted server: Ubuntu 24.04 LTS x86_64, a declared systemd service plus a
documented container build for reproducibility. Test Chrome preferred WT and
forced WSS/WebRTC fallback, Firefox fallback, and native/browser/native peers.
The initial pinned browser targets are Chrome 155.0.8059.80 (installed local
baseline) and Firefox 141.0 / Playwright revision 1490 (fallback target from the
existing harness manifest, not yet executed). Linux targets Ubuntu 24.04 LTS
x86_64; record image digest, kernel and package versions before its rehearsal.
A local machine cannot claim an uninstalled remote runner has been tested.
Profile changes update this record before the affected run.

Native endpoint TLS verifies configured server identity and trust. Hosted HTTP
requires HTTPS, WSS and HTTP/3 with declared origins and trusted proxy policy.
Provision short-lived scoped participant/device credentials through a trusted
backend; provider keys stay server-side. Configure public SIP/RTP addresses,
ICE/TURN and callback ingress independently. A reachable HTTPS route is not proof
of duplex telephone media.

## Default bounds

| Resource | Default / hard check |
|---|---|
| UCTP reliable envelope | 1 MiB |
| Inline message body | 64 KiB decoded |
| History/event page | 100 default, 500 items and 512 KiB maximum |
| File / installation file bytes | 16 MiB / 256 MiB initial profile |
| Artifact transfer chunk | 32 KiB decoded |
| Live event lease / queued refs | 30 seconds / 256 |
| Unauthenticated peer deadline | 10 seconds |
| Lost-last-connection grace | 30 seconds unless explicit end |
| Endpoint confirmation callback | 45 seconds; configured DTMF confirmation |
| Named qualification workload | four participants, six connections, two concurrent sessions |
| Media / quality | [media profile](../../contracts/connect-media-v2.md) |

Existing secure coordinator per-peer budgets (32 Sessions, 64 Connections and
16 cumulative opened Streams per Connection) remain upper bounds until a more
restrictive installation policy is selected. Admission is bounded before external
effects; the minimum tested workload is not advertised capacity.

## Execution lanes

Default checks are local and fixture-only with isolated state and owned processes.
Actual-model, remote-host and live-provider lanes are explicit. G6-093 records
the concrete host, credential references, consenting endpoints, call/spend bounds,
versions and cleanup before those actions. Use the owner's existing authorization
where applicable; do not repeatedly ask for the same approved implementation.

An unavailable Linux/browser/provider environment is recorded as unrun, not a
passing equivalent. Measured CPU/RSS/queues are attributed to server, helper and
browser separately. Media/profile targets freeze before execution; a failed
measurement is not erased by changing its threshold after the fact.
