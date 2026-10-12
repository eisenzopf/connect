# Gate 3 media spike

The executable native proof is `crates/connect-server/tests/gate3_parity.rs`.

It uses the production Connect server, Rust SDK, pinned rvoip UCTP coordinator,
QUIC adapter, Orchestrator subscription router and full RTP datagram framing. Two
authenticated physical peers bind one durable Connect Session, publish PCMU
streams, subscribe to one another and exchange 160-byte, 20 ms frames in both
directions. HTTP and native control then verify the same durable state.

Selected native profile:

- PCMU / G.711 mu-law, 8 kHz, mono
- RTP payload type 0
- 160 samples and bytes per 20 ms frame
- full RTP inside the UCTP datagram payload
- reliable `stream.opened` announcements with peer-global `stream_local_id`

The first datagram can race the reliable announcement of a lazily allocated
subscriber stream and may be dropped. The proof sends a second canary after the
announcement window; subsequent frames are asserted byte-for-byte. This is a
media loss property, not a control failure.

Still required before the media milestone closes:

- live Vapi duplex qualification with the configured saved assistant
- one/four session duration and impairment measurements
- local SIP/RTP composition and controlled Telnyx reachability
- browser WebRTC composition and interoperability

`crates/connect-server/tests/gate3_vapi_media.rs` is the account-free Vapi
proof. It binds an accepted Connect Session to the pinned `rvoip-vapi`
adapter, verifies a bounded source-attributed context command, exchanges PCMU
in both directions with a loopback Vapi-compatible HTTP/WebSocket peer, and
records the delivery outcome. The test also exposed and fixed the native SDK's
sendrecv return-audio path; it is retained as a regression check.

The loopback provider also emits a duplicated final assistant transcript. Connect
maps the assistant role from the durable Session, appends one `voice_transcript`
item to the same Conversation and makes it available to later context views.

See `docs/decisions/007-gate-3-media-profile.html` for the frozen profile and
honest qualification boundary.
