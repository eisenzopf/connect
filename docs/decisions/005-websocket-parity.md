# 005 — a bounded local WebSocket parity entry point

The default and public Rust SDK path remain native UCTP/QUIC with verified TLS.
The optional `websocket_bind` listener is plaintext **loopback-only**, for Gate 1
conformance comparison. It is not a provider media connector, WebRTC SDK, or a
TypeScript native QUIC implementation.

The entry point reuses rvoip's `UctpCoordinator`, retained principal, negotiated
profile, auth lifecycle guard and peer-task supervisor. It installs the same
`Arc<Service>` as QUIC, so both transports share SQLite transactions, scope
checks, receipts, cursors and notification hints. It adds only WS text framing
and the local listener lifecycle, not another application mutation path.

The pinned rvoip-websocket adapter has the application hook but no public
listener shutdown handle. Rather than silently detaching that accept loop,
this narrow parity entry point owns cancellable accept/upgrade work, a JoinSet
of peers, bounded channels and send deadlines. All peers drain at shutdown,
and a test rebinds the exact WS port afterward. Future full adapter integration
should resolve the upstream lifecycle interface explicitly.

Both wire paths have 1 MiB input-frame bounds. Durable read pages also stop at
512 KiB of serialized entries, leaving result/cursor framing headroom. A large
page may contain fewer than the requested count; its cursor points only past
returned or authority-filtered rows. No unread authorized row is skipped.

The `connections` limit applies per listener, not across the combined QUIC and
WS listener. Default: 64 per listener, 256 envelopes per peer queue. No observer
gets an unbounded live message backlog; journal pages recover accepted work.
WS output high-water diagnostics count queue occupancy and contain no payloads.
