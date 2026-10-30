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

Each browser fixture requires normal untrusted-CA refusal, private NSS CA import,
a normal trusted HTTPS response, a paced software audio clock, the existing
decoded clean60 audio/video and AV checks, and a nominated duplex UDP path to
the configured gateway and port range from both original peers' `getStats()`.
The host and browser namespace hashes must differ. Original server, browser,
audio and container processes must join; routes, capacity and the created
bridge must drain. Forced cleanup cannot count as success.
The Ubuntu runner returns only its freshly created fixture directory from the
root container to the runner UID with noninteractive sudo. Private modes remain
unchanged and recursive ownership changes do not follow profile symlinks.
The root fixture container retains only `DAC_OVERRIDE` so it can access the
runner-owned0700/0600 fixture while the host server is using it. All other
capabilities are dropped; the source/root filesystem stays read-only and the
network stays internal. This is an owned test profile, not deployment guidance.

Only numeric summaries, finite failure stages and source metadata are uploaded.
Keys, configs, full media observations and raw process logs remain under ignored
`.local/`. This fixture prepares current distinct-network browser evidence;
it does not qualify WAN, remote providers, relay, ICE restart, installation or
the complete Gate6 matrix. Actual Linux execution remains required.
