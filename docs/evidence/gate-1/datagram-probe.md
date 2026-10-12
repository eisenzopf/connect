# G1-017 — Two-way native datagram media experiment

Command: `cargo run --locked -p connect-server --example quic_audio`.
Result: exit 0 on macOS arm64, Rust 1.97.1, 2026-10-07.

The example uses two real Quinn loopback endpoints with explicit TLS trust,
rvoip's `QuicDatagramMediaStream`, and the checked UCTP/RTP pack/unpack path.
It asserts:

- PCMU synthetic 160-byte payloads are delivered in both directions.
- Independent stream-local IDs (1 and 2), RTP payload type 0 and timestamp 160
  are retained.
- A reliable bidirectional stream carries control bytes while media pumps exist.
- QUIC disconnection and both media-pump closures finish inside a 15 s bound.

This is an architecture experiment, not a complete authenticated live-session
or voice qualification. Remaining gates cover routing, sustained media, loss,
codecs, WAN behavior and provider/other transport interoperability. No provider
credentials or live calls were used.
