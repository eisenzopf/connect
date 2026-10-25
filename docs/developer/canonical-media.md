# Canonical physical media ownership

The canonical QUIC and WebTransport drivers now retain and poll one media reader
alongside their authenticated signaling actor. `NativePeerMedia` owns one
`PeerMediaRouter` for the physical peer. All logical Connections allocate handles
from that router; abandoned and retired handles stay burned. Reader ownership is
claimed once before the driver supplies its physical context to the handler
factory. Dropping even an unpolled reader retires the router. Normal signaling
completion fences media before the bounded final signaling FIN wait. There is
no detached reader task or per-Connection native datagram reader.

The drivers install datagrams only when the negotiated transport supports them.
WSS remains signaling plus separately negotiated WebRTC media; it cannot opt into
this native datagram resource. A physical context without datagrams can still
serve control, but native RTP configuration fails explicitly.

The reader parses bounded version-1 UCTP/full-RTP datagrams, resolves the exact
physical handle, checks direction and negotiated payload type, and invokes
checked ingress before queue buffering. Video retains complete RTP and the
existing 1,180-byte bound. Data is not smuggled over datagrams. Unknown, zero,
retired, malformed, mismatched, paused and precommit routes are rejected.
Canonical ingress refuses unguarded legacy channel registrations. Cooperative
polling keeps malformed media traffic from starving signaling. The reader does
not use the legacy unguarded publisher fanout path or invent quality observations.

The `rtcp-sr-sdes/1` source-clock profile extends the same wrapper with bounded
SR/SDES compounds. The physical reader checks the current inbound handle before
calling checked timing admission. Clock reports have separate accepted/refused
counters and never call media-flow or readiness observations. Actual RTP attaches
its current source-clock snapshot inside that original packet's admission.
Per-Connection CNAME normalization preserves cross-track timing; each stream/SSRC
incarnation and its report/route lifetime fence held metadata. Guarded outgoing
RTP can emit a current report after its successful submissions, under the same
delivery guards. Unsupported or lost mappings retire their prior output SSRC.
There is no idle report worker or retained permission. The exact framing and
component verification scope are in `contracts/native-media-profile.md` and
`docs/evidence/gate-6/browser-client-source-clock.json`.

The public Rust SDK uses `CanonicalClient::send_native_media_datagram` and
`receive_native_media_datagram` for both RTP and clock-only datagrams. The
pre-alpha RTP-only method names are removed. The SDK preserves the original
wire bytes and authenticates the physical lifetime on each operation; successful
submission does not prove server admission or recipient presentation.

Use `connect_client::canonical::observe_native_media_datagram` to distinguish
`ObservedNativeMediaDatagram::Rtp` from `Clock`. Clock observations preserve the
original physical handle/sequence and exact sender SSRC, NTP/RTP values, counters
and CNAME. They are metadata rather than decoded media or readiness. Consumers
must check the current negotiated receive binding and retire old mappings with
that binding. Malformed claimed clocks fail rather than being retried as RTP.
The audio-only private PCMU reader consumes clock-only metadata under one
original bounded read deadline, checks its own handle and returns only actual
160-byte PCMU frames. It does not qualify AV clock presentation.

`pack_sender_clock_datagram`, `SenderClockDatagram` and `SenderClockReport` are
also available through `connect_client::canonical`. A sender samples both tracks
from one source clock/CNAME and supplies counters from actual successful RTP
submissions. Reports belong to those media submissions; an idle report worker
cannot retain media permission. The public builder validates framing and identity,
without inventing a capture clock, sender statistics or authorization.

The browser/native functional fixture now uses these public APIs to send a
common audio/video clock and observe actual current-binding reports separately
from its decoded frames. Its sources remain preencoded and native received
packets are decoded offline. All88client library methods and every server test consumer compile/check pass,
including two controlled QUIC clock/PCM/deadline cases; see
`docs/evidence/gate-6/browser-client-native-clock-sdk.json`. These checks are
separate from actual interoperability and from live native encoder/presenter,
60-second, impairment and recovery qualification. Current browser/browser clean evidence is
in `docs/evidence/gate-6/browser-client-fixed-playout-deadline.json`; prepared
wire API evidence is in `docs/evidence/gate-6/browser-client-public-native-clock-wire.json`.

`CanonicalNativeAdapter::configure_rtp` installs a `StagedRtpMedia` backend and
one persistent `StreamController` on an existing Core-issued route. This is a
trusted server SPI, not a wire command or a readiness grant. Configuration checks
the full authenticated owner, exact Core Session/Connection, physical source,
and the allocation-time membership identity/epoch and endpoint/device/transport.
The sealed Core request exposes frozen membership and endpoint bindings only for
this comparison; they do not replace current policy.

Every change and media packet still requires the installed current `MediaPolicy`.
The same policy instance spans authorized source/destination controllers. Reoffers
retrieve `media_controller`; configuring the route again fails, so refresh or a
new API object cannot reset its default 16 cumulative opened-binding budget.
`staged_media` opts in only on configured routes; `streams` exposes guarded views
that refuse raw receiver/sender ownership. The adapter's default capability
descriptor remains empty. Generic accept/renegotiate and reliable-data methods
remain unsupported until their full canonical handlers/managed graph are installed.

A physical admission permit now has a separate delivery lifetime. Core retirement
cancels that lifetime synchronously without refunding quota. The native controller
is a child of that lifetime, so new changes, ingress publication, forwarding and
queued delivery fail before asynchronous adapter cleanup. In-flight bounded sends
observe cancellation too. Port readiness reports inactive once its owning lifetime
ends. The permit and native route stay allocated through uncertain cleanup.

Native end first fences delivery, then awaits the retained controller's shutdown.
A cancelled wait or unconfirmed bounded cleanup leaves the route and controller
owned and local liveness true. A retry resumes that cleanup. Only confirmed cleanup
removes the map entry and emits its terminal observation; it cannot retire another
route or reset the physical allocator. Physical EOF closes only that reader's
router and routes, preserving unrelated physical peers and the shared logical
Session. No local cleanup result is a provider hangup receipt.

`scripts/check-gate6-canonical-media.py` records source-bound qualification in
`docs/evidence/gate-6/canonical-media.json`. Local TLS QUIC/WebTransport tests use
the actual canonical drivers and Core-prepared routes, real RTP, and PCMU
encode/decode. They cover duplex routing, shared handles, continuing old audio
during a staged video addition, precommit drops, current denial/private audience,
pause, malformed traffic, refresh/cumulative history, per-route and physical EOF
isolation, frozen-origin denial, Core Session retirement, cleanup cancellation and
timeout/retry, WSS refusal, and single-reader ownership/error cleanup. A Core fixture additionally cancels an
already admitted blocked local send through its parent route lifetime, verifies
that the send future and authority guards are dropped, preserves cumulative
history, and leaves the unrelated controller live.

The test's media owner and readiness negotiator are controlled. The reserved wire
ACK establishes no Participant presence or completed media negotiation. This
increment does not qualify canonical offer/answer/ready command installation,
endpoint confirmation, managed mixer adoption, retained subscription commands,
production Connect grant-policy reuse, credential rotation inside an existing
controller, durable replay/recovery, browser/SIP/WebRTC/quality profiles, or any
Gate 6 acceptance criterion. Controllers retain their original credential bound
and fail closed at its expiry; preserving the resource object through refresh
does not authorize it with replacement credentials. Those integration tasks
remain open in G6-020/G6-026 and the dependent service/SDK work.

The [held media control hook](media-control.md) now lets an authenticated
invocation construct/change/pause/end its retained controller without reacquiring
its grant lock. Packet authority remains independently current. The production
store policy, full negotiation/managed graph and credential rotation remain open.
