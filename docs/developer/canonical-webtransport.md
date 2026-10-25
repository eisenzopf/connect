# Canonical WebTransport composition

The optional canonical listener accepts `connect/2` over HTTP/3 on the configured
native TLS endpoint. It shares the current Store, enrolled Device/signing keys,
canonical handlers, logical operation receipts and physical-peer budget with
native QUIC. WebTransport media selects its actual verified Device Endpoint and
registered adapter. A retained logical Connection cannot change transport by
replaying its operation on another physical peer.

Configure the protected canonical server explicitly:

```toml
[webtransport]
mount_path = "/connect"
allowed_origins = ["https://app.example.test"]
advertised_url = "https://connect.example.test/connect"
```

The advertised HTTPS URL is required for the Remote network profile. Origins
must be unique exact serialized HTTPS origins, without a trailing slash. Missing,
repeated or foreign Origin and the wrong mount are refused before canonical
authentication. Explicit loopback HTTP page origins are allowed only in Local
development. Startup rejects credentials, query/fragment, whitespace/control
characters and zero port in the advertised contact. Native-only configuration
leaves WebTransport disabled. In Local mode, an omitted advertised URL uses the
actual bound IPv4 or bracketed IPv6 socket address. Fresh Local certificates
include localhost, 127.0.0.1 and ::1 identities; install explicit TLS trust.

The authenticated application SDK
[`transport_discovery`](canonical-http.md#canonical-transport-discovery) method
returns the configured contact or `null`, control limits and per-session datagram
negotiation. Install TLS trust and the enrolled server signing pin separately;
discovery supplies neither. The application HTTP listener remains loopback.

Media requires actual canonical offer/answer confirmation, current verified
transport-specific Endpoint authority and explicit signed stream subscriptions.
An authenticated command or an accepted Session alone supplies no audio route.
Physical peer closure fences its sole datagram reader and owned media bindings.
The server owns both supervisors through cancellation, signing-key expiry and
shutdown/drop.

## Isolated rehearsal

Prepare dependencies and use the versioned runner with an explicit build cache:

```sh
python3 scripts/rehearse-gate6-webtransport.py --target-dir /path/to/build-cache
python3 scripts/rehearse-gate6-webtransport.py --target-dir /path/to/build-cache --installed-browser
```

The second command explicitly starts an owned installed Chrome with a fresh
ignored profile. `CONNECT_GATE6_CHROME` selects its executable and
`CONNECT_GATE6_NODE` selects Node with built-in WebSocket support. The fixture
uses a short-lived ECDSA certificate with an explicitly supplied SHA-256 trust
hash, a separate enrolled server signing pin and an exact loopback page Origin.
It does not use a certificate-verification bypass or a personal browser profile.

The runner checks dependency inputs and generated OpenAPI, builds exact optimized
unit/integration executables, seals their bytes, matches nonzero exact methods,
and guards Rust, schema, patches, both JavaScript assets and relevant SDK docs.
Raw logs, owned Chrome diagnostics and private fixture configuration stay under
ignored `.local/`. The browser lane is reported as skipped unless explicitly
selected. The report distinguishes these local cases from the remaining G6-049
lifecycle/fault cases, production browser SDK G6-050, remote/provider, acceptance
and publication work. Current scoped evidence is in
[webtransport-composition.json](../evidence/gate-6/webtransport-composition.json).
