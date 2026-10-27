# WebRTC local receive time

The RTC socket driver stamps each datagram immediately after its UDP receive
future completes. That monotonic instant survives protocol processing, the RTC
read queue and the remote-track event queue. It measures local userspace receipt;
it does not assert NIC arrival, remote capture time or source-clock synchronization.

The internal Rust APIs now carry that time explicitly:

- `RTCMessage::RtpPacket(track_id, packet, event_at)`: read events retain the
  original receive instant; outbound requests stamp local request creation.
- `TrackRemoteEvent::OnRtpPacket(packet, received_at)`: the original receive
  instant remains unchanged when an application polls later.

Audio and video reorder queues retain that instant. Their stale-packet checks use
monotonic age, and queue/channel waits consume the original 200 ms budget. Audio
DTMF observations also retain the original local timing. `MediaFrame.captured_at`
is derived from local receive age for downstream bookkeeping; it does not become
a remote capture-time claim. Source-clock evidence remains separate.

The [production-library probe](../evidence/gate-6/webrtc-receive-age-probe.rs)
sends real Core queue output through loopback ICE/DTLS/SRTP. It holds decrypted RTP
events before starting the media pump, verifies the retained age,
discards a packet older than 200 ms, and receives fresh media on the same Core
queue and physical peer. It also observes the retained instant directly from
the actual track event. PCMU with dynamic PT109 and PCMA with PT8 are covered.
The [evidence](../evidence/gate-6/webrtc-receive-age.json) records the original
failing reproduction, component regressions and remaining qualification.

This fixes queue-age preservation. Canonical Core staged media, original Store
attempt custody, authenticated ingress attribution, owned port pumps and sealed
actual readiness still need integration. Browser capture and full fallback,
Opus/VP8/data, ICE/source-clock and hosting profiles remain Gate 6 work.
