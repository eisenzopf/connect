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

Only numeric summaries, finite failure stages and source metadata are uploaded.
Keys, configs, full media observations and raw process logs remain under ignored
`.local/`. This fixture prepares current distinct-network browser evidence;
it does not qualify WAN, remote providers, relay, ICE restart, installation or
the complete Gate6 matrix. Actual Linux execution remains required.
