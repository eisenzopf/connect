# Canonical TLS WebSocket control

The canonical server optionally hosts signed `connect/2` control on a separate
TCP listener. It shares the Store, enrolled identities and signing keys,
canonical handlers, operation ledger and global physical-peer budget with native
QUIC and WebTransport. Enable it in protected server TOML:

```toml
[websocket]
bind = "127.0.0.1:8443"
mount_path = "/connect"
allowed_origins = ["https://app.example.test"]
advertised_url = "wss://connect.example.test:8443/connect"
```

The TLS certificate/key are the canonical server's configured certificate/key.
Remote hosting requires an explicit advertised WSS contact and a bind permitted
by the Remote network profile. Local omission derives the actual bound TCP
IPv4 or bracketed IPv6 contact, including the chosen port when binding port zero.
The UDP QUIC/WebTransport port is independent.

Use one to 32 unique exact serialized HTTPS Origins. Local development also
permits explicit localhost/loopback HTTP Origins. Missing, repeated or foreign
Origins and a wrong mount fail before canonical authentication. Invalid network
addresses, paths, credentials/query/fragment in contacts, whitespace/control
characters, zero contact ports and excessive configuration sizes fail validation.

The host owns its bounded TCP intake, TLS upgrades and canonical peer supervisor.
Excess raw admissions are dropped; the shared physical factory also enforces the
combined live-peer budget across transports. Authentication has its own deadline.
Signing-key expiry or intake failure retires linked transports. Shutdown/drop
cancels active WSS physical lifetimes synchronously; explicit shutdown awaits the
original owned supervisor and intake joins. Cancelling that wait does not detach
the intake. Readiness retains unexpected intake failure.

Authenticated `GET /v2/discovery` advertises a `websocket` descriptor with the
configured WSS URL and `origin_required: true`, or `null` when disabled. Discovery
installs no certificate trust, signing trust, Device verification or media grant.
See the generated [HTTP/OpenAPI reference](../../contracts/openapi-canonical-http.json).

The production TypeScript `BrowserClient` accepts `transport: "websocket"` and a
`wss:` URL. Its platform WebSocket verifies TLS; install certificate trust
separately. WebTransport certificate-hash options are refused in this mode.
Canonical envelopes use one bounded JSON text message each. Signatures,
correlation, replay protection and caller-owned logical operation IDs are shared
with the WebTransport client. A timeout after mutation submission preserves
uncertainty; use current authority on a fresh client to query the original
operation status. There is no automatic mutation resend or transport migration.

WSS control hosting alone does not establish media. An explicit protected
`[webrtc]` configuration installs Connection-bound canonical SDP/ICE and staged
mono Opus with optional VP8. It currently requires Local media addressing and a
bounded peer capacity. Both Chrome and Firefox pass the original 60-second
functional profiles after actual UDP failure, with original operation IDs and
owned teardown. The signed transition-control matrix also passes; see the
[G6-051 completion evidence](../evidence/gate-6/browser-fallback-completion.json).
Remote ICE/TURN, repeated outage recovery and live movement quality retain their
separate scope. A WT datagram MediaPeer on WSS refuses `unsupported-transport`.

## Isolated rehearsal

```sh
python3 scripts/rehearse-gate6-websocket-control.py --target-dir /path/to/build-cache
```

The runner prepares locked dependencies, checks generated OpenAPI, runs the SDK
suite and selects exact optimized configuration and TLS integration methods.
The Node SDK lane uses its built-in WebSocket with an explicit fixture Origin
and `NODE_EXTRA_CA_CERTS` for the fixture certificate. TLS verification remains
enabled. It exercises actual network SDK control and status-only recovery, not
an installed browser or WebRTC. Raw credentials and logs stay under ignored
`.local/`; source and original/sealed executable hashes are checked around the
run. Failed or omitted requirements remain open in the task evidence.
