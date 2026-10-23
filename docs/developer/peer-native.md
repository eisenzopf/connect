# Native Connection preparation (G6-026, in progress)

`Orchestrator::prepare_peer_native_connection` reserves a never-reused Core
Connection ID and the physical peer's Session/Connection quota before it calls
an adapter allocation hook. Native factories no longer need to allocate a route
before Core can account for it. The default cap is 64 Connections and the union
of 32 explicit or Connection-associated Sessions per physical peer.

The caller supplies the fresh authenticated principal, the exact
`PeerAdmission` context, a server-resolved `PeerNativeIntent`, a current
`PeerNativeAuthorityLease` and a trusted native transport context. The lease
protects the actor's active membership, scoped preparation permission and
verified endpoint/device. Core checks owner, Identity, tenant, Conversation,
Session state, endpoint transport and current endpoint verification. Roles and
wire presentation fields grant no permission. This is a trusted server SPI;
public handlers must resolve and authorize their wire aliases through the
application's authoritative state.

Core rejects capacity, known/retired IDs, cross-Core use, closed Sessions and
unsupported adapters before allocation. It installs the permit in the actual
Connection lifecycle before invoking the synchronous, dormant adapter hook.
The hook cannot await, call Core, publish signaling/events, claim readiness or
start detached work. It must allocate only the requested ID. Its sealed request exposes the owned
typed transport resource as an `Arc`; no wrapper or wire-supplied object is
needed. Terminal hints preserve the caller's end reason. Hook failure or
panic retires the candidate and uses existing supervised cleanup; it cannot
optimistically refund an allocated route. As before, authoritative local
liveness and matching cleanup are required before quota release.

`PreparedPeerNativeConnection` owns an unretained candidate. Dropping it retires
that route. `retain` requires fresh current leased authority and the same
membership epoch and endpoint. Retention leaves a Connecting route, with no
active Participant, Session Connection binding, media or delivery claim.
Checked participant admission can transfer the existing permit rather than
charging the same route twice. It must still pass current policy, endpoint,
epoch, physical source, readiness and commit checks. This transfer is tested
with a controlled ready media adapter; production canonical media negotiation
and mixer/stream-controller adoption remain open.

Session end retires native preparations even before participant admission.
Physical EOF retires only its owner's native routes; unrelated peers and shared
logical Sessions survive. Source closure fails continuing preparation checks
immediately, while the adapter still reports allocated local resources as live
until their actual removal. Quarantine retains that charge through uncertainty.
Existing Core tombstones prevent reusing the ID after failed preparation/end.

`CanonicalProfile::with_physical_handlers` installs a `PhysicalHandlerFactory`.
The actual drivers call it only after their TLS/ALPN and, where applicable,
mount/Origin checks, using a server-created `NativeTransportContext`. QUIC
retains the accepted `quinn::Connection`, WebTransport the accepted Session,
and WSS an opaque lifetime context; WSS exposes no cloneable DATAGRAM handle.
A factory constructs one physical handler instead of trying to recover a
native transport from a client-supplied Connection ID.

`NativePhysicalBinding` binds that context at most once to the exact
`PeerAdmission` reference. Refresh keeps that same binding. A different physical
peer, even with the same Identity/Device, cannot borrow it. The concrete
`CanonicalNativeAdapter` registers dormant native routes only through Core's
sealed request. It has authoritative local liveness and bounded terminal-event
fallback. Its capability descriptor advertises no media support, and its
readiness, media negotiation, messaging, transfer and legacy originate methods
fail explicitly until those facilities are installed. Native transport handles
are trusted server resources, not authorization grants or public response data.

The physical quota applies to Connections on that actual native peer. A SIP or
provider leg controlled by an operation is not automatically owned by the
operation caller's physical peer; retiring that caller must not destroy a
retained remote leg. Generic outbound/replacement integration must preserve this
ownership distinction and use the relevant native target's prepared resource.
No legacy adapter origination is a native allocation fallback.

`scripts/check-gate6-peer-native.py` records source-bound native preparation,
actual local TLS QUIC/WebTransport/WSS factory integration and the prior
Session/auth/media regressions in `docs/evidence/gate-6/peer-native.json`.
Wire fixtures resolve trusted aliases under a held typed lease and return an
informational reserved ACK, not a completed `connection.answer`. They do not
qualify complete canonical engine installation, invitation delivery/acceptance,
Connect durable policy, logical-operation replay across peers/restarts, media
quality, SDK/browser/remote/provider profiles or Gate 6 acceptance. Session/media
cleanup cancellation and final assurance observations also remain open.

[Scoped peer exchanges](peer-exchange.md) let a driven handler await a
same-peer correlated reply while the actor reads and writes signaling. Native
negotiators must use the invocation capability and its explicit current policy
hooks; a returned reply still does not establish media readiness.

The explicit [canonical RTP installation](canonical-media.md) now connects
these Core-issued routes to the actual physical driver reader/router and a
retained staged controller. Its default capabilities still advertise no media;
full canonical negotiation/managed mixer handlers remain open. The local Session
teardown cancellation correction is qualified separately in
[Session cleanup](session-cleanup.md).
