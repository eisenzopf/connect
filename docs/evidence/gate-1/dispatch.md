# G1-006 — Authenticated native QUIC dispatch

Command: `python3 scripts/check-dispatch.py`.
Result: exit 0, 2026-10-07, macOS arm64, Rust 1.97.1.

Two independently launched processes used an ephemeral loopback QUIC endpoint
with ALPN `uctp/1` and explicit certificate trust. The client verified:

1. A pre-auth application command receives 401 without executing the handler.
2. The challenge advertises `connect-control/1`.
3. The fixture credential authenticates the retained principal `probe-client`.
4. The application reply has the request's envelope ID in `in_reply_to`.
5. The actor is obtained from authenticated context and execution count is one.
6. Replaying the same envelope returns 409 and does not execute a second time.
7. An unsupported profile receives 501 before application dispatch.
8. Both processes exit cleanly; the server reports `executions=1`.

This proves the dispatch seam and native path, not durable messaging or the
installed product. The fixture validator is a Cargo example and is not compiled
into the Connect server distribution. Default application handling and stock
core dispatch are mutually exclusive for an envelope; regression evidence is
tracked separately in `baseline.md` when the broader suite finishes.
