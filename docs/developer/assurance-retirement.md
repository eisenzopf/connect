# Authentication retirement observations

The canonical peer retires the entire authenticated physical binding when its
credential/task expires or the installed authority policy signals revocation.
It emits signed `identity.assurance-changed`, then `auth.bye`, through the same
bounded reliable writer. The change reports the binding's verified previous
assurance, `current: anonymous`, and a bounded factual reason. Anonymous means
there is no authenticated binding left; it supplies no permission to continue.
No surviving identified assurance is inferred from an expired delegation.

These terminal observations are unscoped because they concern this physical
authentication binding. They do not invent a Connection, change a shared logical
Session's membership, or assert a durable business outcome. Ordinary successful
step-up retains its scoped assurance-change/result exchange.

`PeerAuthority::authority_lifetime` is required. The hook receives the exact
validated credential, Device, mapped Identity and held current authority lease.
It synchronously registers the appropriate `AuthorityLifetime` without awaiting
or reacquiring that authority lock. Each signal must cover the named fact for
this credential incarnation: signing key, credential, delegation, Device,
Identity or a deliberately generic authority retirement. Up to eight independent
signals can be composed. The peer observes their cancellation and never cancels
policy-owned tokens or a shared parent endpoint. These signals grant nothing;
every ordinary command/reply still passes the current authority/signature gate.

The actual earlier credential/task expiry determines `credential-expired` or
`delegation-expired`. `key-revoked` requires a trusted signing-key retirement
signal. A generic route-authority cancellation produces `authority-retired`,
without pretending to know a more specific cause. Wire-supplied reasons and
capability/assurance labels cannot establish those facts.

Initial authentication, refresh and step-up obtain candidate lifetime signals
under their current transition guards. A cancelled candidate cannot install.
Successful refresh/step-up switches the actor's watched incarnation; an old
credential signal cannot retire its successor. A failed candidate leaves the
still-valid prior binding available. Retirement of the current binding during
refresh validation prevents a replacement from borrowing its invalid authority.

Step-up also watches the candidate lifetime while executing the already-held
original mutation, before installing that candidate. Candidate retirement drops
that execution future and its real guard. The prior binding remains available
if it is still valid, and the existing dual step-up failure receipts preserve an
unknown business outcome once execution may have begun. Revocation is not proof
that the operation had no effect; reconcile the same logical operation ID.

The driven actor watches authority signals while idle, executing handlers,
waiting on exchanges or admitting writes. Retirement drops owned work, reply
verification and current guards, cancels its physical child lifetime, and retires
only its own managed routes. Terminal output has a one-second local writer bound
and cannot be delayed by a fresh grant acquisition. A failed, partial or blocked
writer may prevent terminal delivery; closure and guard release still happen.
There is no remote-delivery receipt or durable notification guarantee.

Direct `handle` integrations must drive the policy/expiry lifecycle themselves
and call `take_retirement_output`. It polls current retirement, closes the
binding, and returns the signed terminal pair once. Retain that output through
bounded writer admission. New control and waiter registration fail after
retirement. `run`/`run_frames` handle this lifecycle automatically.

Run `python3 scripts/check-gate6-assurance-retirement.py` for the source-bound
local qualification in `docs/evidence/gate-6/assurance-retirement.json`. New cases
use real held authority guards and Ed25519 signatures, including actual local
TLS/QUIC terminal frames, current/old/candidate signal isolation, task expiry,
refresh validation, handler cancellation, blocked-writer cleanup and step-up
candidate interruption. Existing actual QUIC/WebTransport/WSS fixtures verify
the ordered generic retirement observation and pending exchange expiry. Their
credential, key-revocation and resource policies remain controlled fixtures.

This implements the reusable auth retirement behavior. It does not install
Connect's durable service/store policy, persistent logical Session replay,
IdentityProvider/EndpointRuntime projection, invitation routing or checked media
credential rotation. It does not complete canonical media negotiation or any
Gate 6 acceptance criterion. The task-level signing and scope audit is recorded in [es256.md](es256.md) and
[authentication-audit.md](authentication-audit.md); the implementation plan records
its status after the source-bound checks pass.
