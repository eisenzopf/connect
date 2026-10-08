# Authenticated native dispatch experiment

Run `python3 scripts/check-dispatch.py` after dependency preparation. The source
is a Cargo example at `crates/connect-server/examples/quic_dispatch.rs` so it
uses the same dependency graph as the server rather than a separate lockfile.

The harness runs two separate processes with an ephemeral loopback listener,
a pinned generated certificate, a public test-only credential, and bounded
shutdown. It asserts unauthenticated rejection, advertised profile negotiation,
correlated response, retained actor identity, replay rejection, unsupported
profile rejection, and one application execution. The validator is a fixture
and is never included in the installed Connect binary.
