# Canonical transport drivers (G6-026, in progress)

The upstream `rvoip-uctp::v2::server::CanonicalProfile` composes the existing
canonical authentication peer, an authenticated handler, bounded physical peer
admission and optional authenticated reachability. The concrete driver modules
are `rvoip_quic::canonical`, `rvoip_webtransport::canonical` and
`rvoip_websocket::canonical` (feature `wss`). Each exposes `start` and returns a
shared `ProfileServer`. These are explicit connect/2 entry points. Connect's
public server installation and durable operation policy remain G6-037 work.

The caller owns the listening endpoint and feeds a bounded Tokio channel of
accepted connections (QUIC/WebTransport) or TCP streams (WSS). The channel must
have at most 4096 slots. `CanonicalProfile::new` requires a physical peer limit
between 1 and 4096. The supervisor rejects capacity excess, retains every
admitted peer task, and closes its admission receiver on drain. The listening
endpoint owner must handle a failed channel send by closing that newly accepted
connection and must bound its own TLS/QUIC handshakes before channel admission.
A shared listener can retain its other protocols; retiring one canonical peer
closes only that peer's physical connection. QUIC and WebTransport finish the
signaling send stream and wait at most one second for acknowledgement before
closing the physical connection, so an immediately queued final `auth.bye` is
not discarded by eager connection close. Server shutdown interrupts that wait;
this bounded transport acknowledgement does not create an application receipt.

`begin_drain` stops admission while active peers finish normally. `shutdown`
cancels active peers and awaits the retained supervisor, allowing two seconds
for cooperative cleanup before aborting and joining remaining tasks. Its join
is shared, so cancelling one shutdown waiter does not discard ownership. The
last control's drop cancels the supervisor and its peers. Physical admission,
rejection and active counts are available through `stats`; these are transport
observations, never delivery or authorization evidence. A peer task panic
retires the server and produces a closed shutdown result.

All drivers construct the same `CanonicalPeer` before waiting for the first
application stream or HTTP upgrade. Its ten-second authentication deadline
includes that wait. QUIC verifies the actual TLS-negotiated `uctp/2` ALPN and
uses the first bidirectional signaling stream with four-byte network-order
length framing. WebTransport verifies `h3`, accepts HTTP/3 CONNECT only at the
configured mount, checks Origin when an allowlist is configured, and uses one
bidirectional signaling stream with the same length framing. The native
WebTransport reader preserves wake registration when a stream header and first
payload arrive together. WSS owns TLS termination and HTTP upgrade, checks the
configured mount and Origin, and accepts JSON text messages. Binary WebSocket
messages retire the peer. Both frame and message allocation limits are 1 MiB;
QUIC/WebTransport reject an oversized length prefix before allocating its body.
No WebSocket subprotocol extension is invented: the dedicated mount and v2
`auth.hello` application-profile selection establish the canonical profile.

Configured HTTP Origin allowlists require exactly one matching header. `None`
is an explicit native deployment choice that disables Origin enforcement;
empty sets deny every upgrade. HTTP failures precede credential validation.
Drivers do not log raw credentials, headers, URLs or frame bodies. Legacy v1
envelopes cannot execute through these canonical entry points. Existing
historical upstream adapters are separate paths and do not qualify as a
canonical service installation.

The shared framed peer actor retains signed replies, credential refresh,
step-up, expiry/revocation, pending reply ownership, replay protection and the
bounded outbound route writer. Reachability uses configured server contact
information, never the client's observed UDP/TCP port. Current recipient,
audience and resource authorization must be supplied through `RouteAuthority`;
`LocalSubmission` still means local writer admission only. The journal remains
authoritative for application mutations and reconciliation.

Qualification is recorded separately in
`docs/evidence/gate-6/canonical-transports.json`. Local pinned TLS fixtures do
not qualify browsers, remote deployment/providers, durable Session reconnect,
media quality, or the remaining Gate 6 acceptance checks.


## Transport selection and reconnect implementation (G6-052)

Implementation has started after G6-050/G6-051 completion. Each physical
`BrowserClient.open()` now captures its enrolled actor/device/signing and server
signature-trust values before its first await. Editing the caller's draft for a
new connection cannot change the existing peer. Required inherited properties
are captured too; key handles and the original cancellation signal retain their
intended semantics. Five new component cases are included in all 327 passing
SDK checks. See
[`transport-reconnect-context.json`](../evidence/gate-6/transport-reconnect-context.json).

Preferred transport selection, fallback reasons, fresh authorization, cursor
recovery and the required outage/reconnect matrix remain in progress. No new
automatic mutation resend or transport selection is introduced by this checkpoint.
