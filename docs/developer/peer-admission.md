# Physical-peer managed admission (G6-026, in progress)

Canonical authentication now owns one core `PeerAdmissionOwner` per physical
peer. Its immutable context binds the validated principal's complete
issuer/tenant/subject ownership key, mapped Identity and verified Device. A
handler receives a reference through `AuthorizedEnvelope::peer_admission()`.
That reference is resource context, never an authorization grant. Standalone
`AuthenticatedDispatch` instances have no physical owner and return `None`.

Refresh and step-up retain the same context; replacing credentials does not
reset admission counts. Candidate owners become physical-peer owned only when
authentication commits. Failed initial candidates cannot publish ownership;
failed refresh cannot close or replace the valid context. The unique owner
retires on canonical peer closure, including EOF, expiry, cancellation and
transport shutdown. The parent endpoint and other peers remain independent.

The concrete core entry point is `Orchestrator::admit_peer_session_connection`.
Use the context and current principal from the authorized envelope, and resolve
the actual core Session/Connection and endpoint from trusted server state. The
wire body cannot select a different physical peer, principal, Device or core
instance. The installed current participant policy still owns membership,
endpoint verification, media permissions, revision/epoch and confirmation.
The native adapter/current policy must separately establish the exact wire
Connection-to-physical-peer mapping; quota membership does not establish it.

The managed admission path defaults to 32 distinct associated Sessions and 64
Connections for one peer, counting pending preparation, committed routes and
uncertain cleanup. Multiple Connections in the same Session count once toward
the Session cap. Shared logical Sessions do not belong exclusively to one peer.
The context binds once to its owning Orchestrator and cannot cross Core
instances. Explicit bounded configurations from 1 through 4,096 are supported
for trusted compositions; `snapshot()` reports the effective limits and counts
without exposing identifiers or credential material. Canonical peers use the
secure defaults.

Reservation occurs after current policy/relationship checks and protected core
lifecycle/transport ownership checks, before adapter acceptance or media
preparation. The adapter must declare authoritative local liveness; an adapter
without that guarantee is rejected before spending quota or accepting a route. A permit is retained by the actual core Connection lifecycle,
including cancellation and failed preparation. Cleanup releases it only after
that retired route has matching cleanup/terminal evidence and its owning adapter
reports the local route no longer live. An early ended hint, default/unknown
liveness, or an end result while the route is still live retains quarantine. An uncertain end retains capacity and its Session
association. Slot release removes only its exact record nonce. A stable
physical-owner marker remains in the bounded core Connection tombstone so a
logical admission replay cannot borrow another peer's route after cleanup.
Returning an earlier authorized receipt does not reaccept a route or spend
another permit.

Dropping the unique owner cancels its child lifetime and synchronously retires
only routes whose protected lifecycle still carries that owner's permit. The
existing core supervisor owns asynchronous cleanup; no new detached peer task
is created. Mixer delivery guards also check peer closure. Shared Session
participants and other peers' media keep running. Confirmed cleanup releases
counts; retirement alone never claims that an uncertain adapter stopped.

Tests exercise the actual managed core, mixer and controlled adapter, including
32-Session/64-Connection saturation before acceptance, multiple Connections per
Session, pending capacity, current revocation after preparation, cross-identity,
Device/Core isolation, cross-peer replay and stale cleanup. Real local pinned
TLS QUIC, HTTP/3 WebTransport and WSS signaling carry the same context into this
core path across refresh; verified step-up retains it too. Wire fixtures use an
explicit trusted mapping and controlled media route, not a qualified native
negotiation/provider backend.

The runner is `scripts/check-gate6-peer-admission.py`; its evidence is
`docs/evidence/gate-6/peer-admission.json`. This scope covers managed admission of
already registered preliminary routes. The subsequent
[Session-view increment](peer-sessions.md) reserves zero-Connection pending
Sessions under the same cap. The subsequent [native preparation increment](peer-native.md)
reserves Core IDs and quota before dormant native route allocation through
physical transport factories. Generic outbound/replacement paths and a
complete canonical engine handler/public Connect installation remain open.
The context is not another Conversation store and does not replace the journal.
Logical-Session replay/reconnect/restart, IdentityProvider/EndpointRuntime
bridging and the remaining assurance/media/browser/remote/provider acceptance
checks are not qualified here.
