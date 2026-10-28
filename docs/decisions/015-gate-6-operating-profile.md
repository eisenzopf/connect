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

October 25 local WT measurement update: the installed Chrome runner is now
155.0.8059.85. Pin that patch version for the explicit 60-second capture/render
measurements and record its actual CDP version in each report. The original
155.0.8059.80 installation baseline remains historical. Firefox/fallback,
Linux, native interoperation and named remote/provider profiles retain their
separate qualification requirements; no media threshold changes are made.

October 27 local fallback measurement update: select the installed, code-signature
verified Firefox158.0.1 with public WebDriver BiDi/Puppeteer25.16.0 for the next
normal-trust macOS arm64 fallback matrix. The initial Firefox141/Playwright1490
manifest target remains an unexecuted historical planning target. Local Firefox
uses independent visible native windows, a fresh owned profile/CA trust and an
explicit owned loopback STUN fixture; its loopback ICE test preference remains
recorded in each browser profile. Native address obfuscation and normal TLS
validation remain enabled. Chrome remains155.0.8059.85. These target selections
do not change source content, capture/draw cadence, observers, thresholds or the
60second window. Prior failed audio observations remain failures. Linux/remote,
physical devices and provider lanes still require their own named execution.

October 27 native fallback receiver update: before the next normal installed
browser matrix, the SDK sets the same 80 ms preferred native jitter-buffer target
on its audio/video receivers before an offer. The installed Chrome/Firefox
capability probes retain this target; a private Firefox comparison measured
about 71–76 ms audio buffering with the target and 51–53 ms with automatic
buffering. These distinct runs do not establish causality or remove retained
audio failures. Sources, observers, thresholds, 60-second windows and independent
AV-sync checks remain unchanged. The target is a preference, not a hard latency
guarantee. Unsupported or ignored native control refuses before signaling.

Native endpoint TLS verifies configured server identity and trust. Hosted HTTP
requires HTTPS, WSS and HTTP/3 with declared origins and trusted proxy policy.
Provision short-lived scoped participant/device credentials through a trusted
backend; provider keys stay server-side. Configure public SIP/RTP addresses,
ICE/TURN and callback ingress independently. A reachable HTTPS route is not proof
of duplex telephone media.

October 27 isolated Linux HTTPS fixture target: use the existing local image
`sha256:a81873456efa5d13f9b67cfbecc4e11bf6946e924684f6ffc9a1161ca20fe2b5`
(Ubuntu 24.04.5 LTS amd64; Rust 1.97.1, LLVM 22.1.6, Clang 18.1.3). The local
Docker daemon runs in an arm64 Lima VM, so this amd64 userland is emulated.
The G6-053 functional fixture uses fresh synthetic state, a private CA, four
separate HTTPS roles, an internal container network and only host-loopback
published ports. Freeze this target before executing the host/client checks.
It cannot establish native x86 performance, named remote browser/media coverage,
the G6-056 installation lifecycle or any whole-gate acceptance criterion.

The G6-053 isolated remote-client fixture uses the same pinned Linux client
image and the current Mac arm64 server CLI (SHA256 recorded with its build).
The Linux client reaches four Mac loopback HTTPS listeners through the local
Lima `host.lima.internal` alias. That exact name and port are provisioned in
the private fixture CA/leaf and each Remote-profile public origin before start.
This is a distinct-kernel local hosting boundary, with no external host/provider
or model account. It does not substitute for Ubuntu server installation or the
named remote/media qualifications. Freeze this topology before its wire checks.

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
