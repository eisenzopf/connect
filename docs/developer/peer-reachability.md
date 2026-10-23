# Authenticated peer reachability and local submission

This guide records the authenticated reachability increment; the final authentication audit tracks component qualification and downstream integration. The canonical runtime can now publish and renew a
real authenticated live-peer route and send guarded server envelopes over its
reliable writer. This is process-local transport state, not Conversation
membership, endpoint provisioning or a delivered/read receipt.

Install it through `CanonicalPeer::with_reachability`, supplying a bounded
`ReachabilityRegistry`, `TrustedPeerRoute` and `RouteAuthority`. The trusted
adapter supplies transport, configured server contact, priority and lease; a
client hello cannot choose them. An incoming client UDP address is not evidence
of a dialable endpoint. A peer without installed route context reports an empty
reachability list. Production adapters still need to install this context from
their actual transport state.

`RouteAuthority` requires both a credential/device/Identity revocation lifetime
and a current recipient-delivery authority callback. The synchronous lifetime
hook runs under the owning peer's authority lease and must retire when its
credential, signing key, device ownership or mapping is invalidated. The
delivery callback reacquires current recipient, audience and resource permissions
after dequeue. The submitting trusted application independently authorizes its
source and freezes the authorized business intent; registry lookup grants none.

Route preparation is invisible. The candidate is published synchronously only
after the complete auth/refresh/step-up candidate and signed session reply are
ready. It retains complete subject/issuer/tenant ownership, mapped Identity,
verified Device, physical peer ID, credential generation and lifetime signals.
Dropping a failed candidate leaves the current route intact. Successful refresh
or step-up replaces only this physical peer's exact registration; old cleanup
cannot remove the successor. Old handles and queued envelopes cannot borrow the
new credential. Concurrent Devices retain independent physical registrations.

The registry holds at most 4,096 registrations and returns at most 64 fresh
routes per target owner/Identity. Limits fail explicitly without eviction or
truncated ambiguity. Lookup order does not select a route or provide a failover
policy. Public applications must authorize the viewer before exposing hints.
Lookup filters cancelled physical/revocation lifetimes, closed writers and both
monotonic and UTC expiry. An absent fresh route means unknown reachability; it
does not assert that a Device is offline. Expired registrations retain their
capacity slot until their owning peer retires.

Auth session replies include the staged actual hint. A correctly signed,
currently authorized keepalive with this peer's token renews the registration
and returns its new hint in `ack.details.reachability`. Leases are at most 60
seconds and cannot outlive credential/task expiry. Heartbeat can renew a stale
observation while its physical/authenticated binding remains valid. It cannot
revive a retired incarnation or extend a previously captured handle's expiry.
It does not implement reconnect/resumption or restore a physical route after
process restart.

`RouteHandle::submit` queues a validated server envelope to the actual owning
actor, bounded to 32 requests. Before admission and after asynchronous delivery
checks, the actor verifies the captured registration, generation, lease and
current lifetime. It assigns a fresh wire ID/timestamp, signs the envelope and
holds recipient authority through bounded writer admission. Cancellation before
dequeue prevents a phantom write. The route lease, caller deadline and peer
credential deadline bound the write; revocation and closure interrupt it.

The returned `LocalSubmission` contains the actual wire envelope ID and means
only local reliable-writer admission. It does not mean remote delivery,
acceptance, readiness or a recipient receipt. If writing may have started and
the outcome is lost, `SubmissionUnknown` is explicit. Preserve the logical
operation/message identity and reconcile through the application ledger.
This one-way submission API does not implement the client's application-level
decision/acceptance flow or a new arbitrary-operation escape hatch.

Idle credential expiry, current authority retirement, EOF, writer failure and
peer closure remove only the exact owned registration. An authority lifetime
retirement also emits a minimal signed `auth.bye` and retires the peer's child
lifetime. Closing one peer does not cancel its shared parent endpoint, other
Devices or the policy-owned revocation token. A partial write timeout retires
the poisoned writer/peer and releases its authority guard.

Run `python3 scripts/check-gate6-peer-reachability.py` for source-bound local
evidence. The real pinned `uctp/2` TLS/QUIC fixture receives a registry-routed,
signed frame, exercises delivery denial and revocation while permission checking
is pending, and observes retirement. Other cases cover trusted address ownership,
heartbeat expiry, multiple Devices/owner domains, stale and cancelled queues,
capacity, failed candidates, refresh/step-up and partial writer uncertainty.
Credential/device/revocation/resource policies remain controlled fixtures.

Connect's `EndpointRuntime` observation bridge, durable identity/membership
authorization, incoming Session invitation/acceptance, reconnect and logical
Session replay, production QUIC/WT/WS profile factories, public SDK/browser and
remote/provider/media-quality qualification remain open. This registry is an
operational route source; it does not create a second authoritative conversation
store or qualify those later integrations. No Gate 6 acceptance is inferred.


The [authentication retirement lifecycle](assurance-retirement.md) now supplies
required credential-incarnation retirement signals and a signed terminal
assurance-change before goodbye. Its separate report includes current/candidate/
successor isolation and blocked execution/writer cleanup. The earlier evidence
above retains its recorded source scope.
