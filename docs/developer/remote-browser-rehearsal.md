# Owned distinct-network browser rehearsal

`scripts/rehearse-gate6-remote-browser.py` runs actual Linux Chromium peers in
the existing local Lima VM against a Mac Remote-profile server. Only synthetic
canvas/video and oscillator/audio sources are used. There are no device,
model-account, provider or telephone calls.

The fixed owned image is
`sha256:533cdd9c6983863f0c7ae50b3a4f9a6157bfe5b88f84b06dd44a5c61e3234bd0`:
Linux arm64, Chromium 151.0.7922.34 and NSS 3.98. This browser supplies a network
component observation; it does not replace the approved installed
Chrome 155 / Firefox 158 matrix or native Linux x86_64 hosting/performance.

The Mac server binds WSS and media on wildcard interfaces and advertises the
explicit `wss://host.lima.internal:PORT/connect` contact. Its private CA issues
that exact DNS name. The media configuration binds 16 checked high UDP ports
and advertises `192.168.5.2`, the existing Lima host gateway, with All policy and
no external STUN/TURN services. Both browser peers run in the Linux kernel;
their WSS and media traffic cross the VM-to-Mac network. The HTTPS loopback
asset origin is inside the browser container, with its own leaf certificate
and private key. The Remote WSS origin allowlist contains that exact origin.

The first fresh container has no fixture CA and must specifically reject an
HTTPS request to the WSS listener with `net::ERR_CERT_AUTHORITY_INVALID`.
The next fresh container imports the fixture CA into its normal NSS website
trust database. The browser uses normal certificate and hostname verification;
no certificate-error bypass flag or ignored TLS error is enabled. The container
has a fresh temporary home and profile, a read-only root filesystem, no host
browser profiles and an explicitly pinned image. Root Chromium uses its normal
container `--no-sandbox` setting; this does not change TLS verification.
Audio output goes only to a private paced software sink. Measurements inspect
the independently decoded remote stream; no physical sound card or audible
room output is qualified.
The local image extends the pinned Playwright base through
`scripts/video/Dockerfile.remote-browser` with PulseAudio
`1:16.1+dfsg1-2ubuntu10.1`. A private Unix socket and software null sink pace
output to wall time without a host sound card. Before any media offer, a
bounded startup of at most five seconds waits for 0.25 seconds of audio clock.
Then a 1.2-second calibration requires the clock to advance at 0.8–1.2
times elapsed wall time. This source-validity check does not change the existing
quality thresholds. An unpaced ALSA null sink is not a valid source fixture.
The original audio-server process must join during cleanup. Browser DNS resolution
permits only `localhost` and the owned `host.lima.internal` gateway.

Prepare dependencies, build the source-bound `gate6_canonical_webtransport`
test executable, and build the public TypeScript SDK before running:

```sh
python3 scripts/rehearse-gate6-remote-browser.py \
  --server-build PATH_TO_SERVER_BUILD_SUMMARY \
  --server-inputs PATH_TO_SERVER_SOURCE_HASHES \
  --algorithm EdDSA \
  --output .local/gate-6/browser-remote/FRESH_RUN
```

The build summary and hash-manifest formats match the native-media runner.
The existing local Playwright module is also hashed, together with the public
SDK source, generated SDK distribution, fixture scripts and server build
inputs. Private keys, credentials, reports and logs remain beneath `.local/`.

The actual browser measurement uses the existing 60-second clean Opus/VP8
observer, including independent opposite audio tones and video source markers,
decoded and changing frame rates, stream selection, the original signed Session
end and paired termination. Its thresholds remain unchanged. Readiness alone
does not pass media. The runner requires the exact ignored child test, actual
guest/server exit codes, original browser joins, unchanged source/executable
bytes, two durable offers and reclaimed original RTC routes/capacity.

The whole rehearsal remains failed if CA refusal, import, media, thresholds or
cleanup fail. Retain failures explicitly. This direct-path fixture does not
qualify distinct-network TURN, ICE restart, SIP/RTP, WAN or full quality profiles.
