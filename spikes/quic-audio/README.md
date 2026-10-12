# Native datagram architecture probe

`cargo run --locked -p connect-server --example quic_audio`

The Cargo example reuses rvoip's `QuicDatagramMediaStream` and the UCTP substrate
helpers over two real loopback QUIC endpoints. It sends synthetic 20 ms PCMU
payloads in both directions, decodes the checked UCTP/RTP packets, separately
exchanges bytes on a reliable stream, disconnects and closes both media pumps.
Certificate trust is explicit. No recording, account, phone number or provider
request is involved.

This is a framing and lifecycle experiment. It does not qualify full authenticated
live-session routing, codecs beyond PCMU, network loss, WAN performance, Vapi,
SIP/PSTN, WebRTC or video. Those retain their subsequent delivery gates.
