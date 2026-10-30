# Owned distinct-network browser fixture

`scripts/rehearse-gate6-browser-bridge.py` runs the canonical Remote server on
an Ubuntu x86 host and current CFT156 browsers in a separate network namespace.
It creates one internal Docker bridge, binds the server's media and signed WSS
listeners to that bridge's private gateway, and gives the container only the
fixed `connect.fixture.internal` contact. It uses synthetic identities and media.
There are no provider, model-account or named remote endpoints.

The manually dispatched `gate6-browser-media.yml` workflow runs EdDSA and ES256
independently after producing source-bound server libraries at release opt3.
Its integration driver has its separately recorded opt0 profile. The pinned
browser image, integrity-locked automation and source hashes remain checked.

Each browser fixture requires normal untrusted-CA refusal, fresh Chrome profile CA import,
normally verified signed WSS, a paced software audio clock, the existing
decoded clean60 audio/video and AV checks, and a nominated duplex UDP path to
the configured gateway and port range from both original peers' `getStats()`.
The host and browser namespace hashes must differ. Original server, browser,
audio and container processes must join; routes, capacity and the created
bridge must drain. Forced cleanup cannot count as success.
The browser container uses the host operator's non-root UID/GID. Private0700/0600
fixture modes remain unchanged, all Linux capabilities are dropped, and no
ownership conversion is needed. The source/root filesystem stays read-only and
the network stays internal. This is an owned test profile, not deployment guidance.

Only numeric summaries, finite failure stages and source metadata are uploaded.
Keys, configs, full media observations and raw process logs remain under ignored
`.local/`. This fixture prepares current distinct-network browser evidence;
it does not qualify WAN, remote providers, relay, ICE restart, installation or
the complete Gate6 matrix. Actual Linux execution remains required.

## Owned distinct-network relay extension

The manual `relay` lane selects EdDSA/ES256 with UDP and TCP TURN control.
The `full` lane also includes those four cells. The native Linux x86 image is
pinned to `coturn/coturn@sha256:59af4221a2c45c1195a1a4a865542b3ea744be1dd8e228d11bde87c6631081cf`;
the launcher verifies the actual 4.18.0 binary and its digest. TURN runs in a
third namespace on the same internal bridge. A bounded private-config gate starts before address inspection, then execs the
verified daemon after atomic configuration publication; no static IPAM setup or
image external-IP discovery is used. It receives only
`NET_BIND_SERVICE` in its capability bounding set because the image executable
carries that file capability; all configured ports are above1024, with no host
network mode or published ports.

One synthetic REST secret issues credentials valid for180seconds, with quota8
and a32port relay pool. Explicit peer allowlists admit only this bridge's server
gateway and owned TURN address; all other IPv4 peers are denied. Every original
browser/server peer uses relay-only policy through protected server secret
references. Public stats must prove both local and remote relay candidates,
the exact private relay range, nominated duplex UDP and decoded opposite-source
content. Each cell additionally performs the original two ICE generations and
lost-answer recovery on the same browser objects, retaining the two-second
fresh decoded-content limit and unchanged clean60 quality bounds.

All original browser and server owners must join before coturn receives SIGUSR1.
The same verified4.18 oracle must observe zero allocations within10seconds,
then join the original container within a separate10seconds. The owned container
and internal bridge must be removed. Numeric summaries exclude REST credentials,
wire operation IDs and raw logs. Preparation and local ARM primitive probes do
not qualify native Linux x86 browser relay media.

The owned TURN bridge uses the exact relay range 32000–32031. Browser preflight admits ICE recovery on this internal bridge only for two relay-only original peers in the clean profile. Direct bridge ranges remain sixteen ports. The portable-startup x86 run 41100747477 proves STUN readiness and normal shutdown in all four cells, but its browser preflight refused the relay scene before any allocation. See `docs/evidence/gate-6/linux-browser-relay-scope.json`; this attempt does not qualify relay media.
