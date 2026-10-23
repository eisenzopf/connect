# Pending Session views (G6-026, in progress)

`Orchestrator::start_peer_session` reserves a physical peer's Session view
before it creates the actual Core Session or emits `SessionStarted`. A pending
Session counts even with zero Connections. The default 32-Session limit is the
union of explicit views and Sessions associated through pending, committed or
quarantined Connection permits. Multiple views/Connections in the same Session
cost one Session slot. Releasing a view does not refund an uncertain Connection.

The application supplies a stable server-selected Session ID, Conversation,
medium and bounded unique invitation targets in `PeerSessionStart`. It also
supplies a `PeerSessionAuthorityLease` protecting current scoped create/attach
grants and resolved active memberships. Core checks the actor against the fresh
principal, physical peer, mapped Identity, Conversation and tenant. Requested
recipients must match the protected current membership set. A role grants no
permission. Payload presentation IDs cannot select an actor or physical owner.
The lease is trusted server composition, never a client-created proof.

Canonical handlers can borrow the installed owned guard's concrete type using
`AuthorizedEnvelope::authority::<T>()`. The result borrows the invocation; it
must not become a cached grant. Canonical control retains the same guard from
handler execution through bounded correlated reply delivery, instead of dropping
it and rereading policy between an authorized effect and its reply. Each new
invocation still acquires fresh current authority. This lets a policy retain a single
owned read guard through handler commit and response delivery, including when
a grant writer queues. Core does not reacquire that policy lock. The policy
must fail closed if the typed lease is absent or mismatched. Connecting a real
durable store policy remains separate integration work.

Creation starts an `Initiating` Session with no active Participants or
Connections. Targets are frozen runtime invitation intent, not delivered
invitations, accepted presence, subscriptions, or media recipients. The
Session can subsequently install checked participant control. Invitation
delivery and acceptance still need their own authorized application workflow.

A same-physical-peer replay with the same intent, creator and membership epochs
returns the original Session ID without another event, Session, or view. It
does not resurrect an ended Session or a released view. Different intent,
recipient epochs or physical creation owner conflict. A separately authorized
`attach_peer_session` associates a live existing Session with the creator or
its frozen invitee, checking current membership and epoch again. Other peers
can neither inherit old creation receipts nor attach through a reused membership
epoch. Administrative/external projection Sessions require current attach
policy but have no peer-created invitation origin.

Core retains up to 10,000 peer-created runtime origins, including terminal
ones. Trusted hosts may lower this finite limit before first creation. At
capacity it refuses new IDs instead of evicting tombstones. The application
journal still owns durable logical-operation replay, restart recovery and
Session expiry. This bounded in-memory origin map establishes none of those
claims and must not substitute for the journal.

The unique physical owner's drop removes only its views and retires its owned
Connection routes. Other peers' views and the shared logical Session and
Conversation survive. Explicit Session end removes all explicit views;
Connection charges remain until their existing lifecycle proves cleanup.
Creation, external projection and administrative starts serialize their origin
checks. A pending forced Conversation close fences new creation/attachment and
reopen; its RAII fence drops on cancellation. Session-end/media cleanup
cancellation and durable Conversation coordination still need the complete
application lifecycle qualification.

`scripts/check-gate6-peer-sessions.py` records source-bound core, canonical
local TLS QUIC/WebTransport/WSS, auth, media-regression and schema checks in
`docs/evidence/gate-6/peer-sessions.json`. The controlled wire fixture resolves
explicit wire aliases to Core IDs under its held lease. It does not qualify
native allocation, invitation delivery, provider media, public Connect handler
installation, SDK/browser or remote hosting behavior. G6-026 remains open.

[Native Connection preparation](peer-native.md) extends reservation to actual
Core-issued native Connection IDs before local adapter allocation. Its physical
transport factories retain the accepted server transport and cannot infer a
source from a wire Connection ID. Full negotiation remains separate.
