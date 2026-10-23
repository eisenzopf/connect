# Staged RTP transport backends

G6-020 remains in progress. The shared `rvoip_uctp::staged_media` implementation
connects staged engine ports to the actual peer media router and RTP datagram
transport. QUIC and WebTransport adapters expose an explicit `staged_rtp_media`
backend factory for an already authenticated route. These factories check its
current owner, expiry, canonical binding and lifetime. They require a profile-owned
negotiator; they do not enable or advertise a new public protocol profile.

Use the backend's `handle_allocator()` when creating its `StreamController`.
Every allocation reserves from the same `PeerMediaRouter` used by existing
media on that physical peer. It retains the reservation as an opaque owned
allocation, rather than copying the next integer. Abandoned handles remain
burned. Registration validates the reservation's peer and owner as well as the
exact canonical Session, Connection and stream binding. An unprepared allocation
can confirm that only its unused reservation was released; an allocated network
port still requires its registered cleanup callback.

The profile negotiator has two stages. `prepare` validates the peer's selected
format and exact IDs, local handle and generations. `confirm` runs after server
routes exist and must establish the peer's exact prepared readiness. The profile
must enforce authenticated version selection, request correlation, capability
intersection and cancellation. Neither stage may disturb existing media. The
shared backend rejects missing, extra, duplicate or mismatched answers before
activation. This interface is trusted backend composition, not client-supplied
readiness assertions or a replacement for canonical authentication.

The production peer reader invokes a checked ingress callback before channel
buffering for staged routes. A packet received while its engine port is prepared,
paused, revoked or retired is rejected immediately. It cannot become valid by
waiting in a raw input queue until a later activation. Normal peer-router routes
retain their existing bounded channels. No staged publisher is installed in the
unguarded fanout registry, and its `MediaStream` view refuses raw input/output
ownership; callers must use guarded engine packets.

Outbound packets pass through `QueuedMedia::deliver`, retaining current source,
destination and delivery guards through the nonblocking local transport send.
The backend writes the existing eight-byte UCTP header and full RTP. Audio uses
the actual negotiated payload type; video/screen preserve marker/timestamp while
rewriting hop-local payload type and SSRC and removing extensions/CSRCs. VP8
packetization respects the current peer budget and preserves source gaps. A
retired source clock route is rejected. No sender report, decode quality or A/V
alignment is inferred from these packet operations.

This RTP backend supports mono PCMU/PCMA at 8 kHz, mono Opus at 48 kHz and VP8
at 90 kHz. Static RTP payload types retain their codec meaning; other mappings
must be explicitly negotiated dynamic types. Reliable ordered application data
requires a reliable transport handler and is explicitly unsupported here. It is
never silently placed in media datagrams.

Retiring a peer-router binding synchronously fences the exact staged stream's
delivery. Its cleanup cancels the owning pump, removes only its own router/map
entries and awaits its retained task. Cancelling a cleanup wait preserves the
join future for a later attempt. Core cleanup confirmation and quarantine still
apply. Parent route loss also stops the pump. Unrelated audio, Sessions and peer
bindings remain owned independently.

The local transport checks use the production QUIC reader and actual TLS/QUIC
datagrams with a controlled readiness provider. They are distinct from canonical
envelope-v2 negotiation, WebTransport browser execution, device capture, VP8
decode/render, SIP interoperability, managed mixer integration and the complete
G6-020 matrix. Connect's durable service and public dispatch remain G6-037/G6-038.
No new HTTP operation is exposed by this backend increment.

The canonical drivers now install a physical `NativePeerMedia` reader/router,
and `CanonicalNativeAdapter::configure_rtp` retains the backend and controller
on the Core-issued route. See [canonical media ownership](canonical-media.md)
for the actual local QUIC/WebTransport evidence and its remaining negotiation,
policy, managed-graph and readiness limitations.
