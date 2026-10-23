# Canonical authenticated dispatch backend

This guide records the initial G6-026 dispatch increment; current task qualification is recorded by the final authentication audit. The reusable `rvoip_uctp::v2::auth_dispatch` backend
accepts bounded envelope-v2 bytes for one authenticated peer incarnation and
calls one installed application handler. It does not install the public
`uctp/2` or `connect/2` handshake in an existing adapter. Connect's durable
canonical handler remains G6-037.

`AuthenticationMethods` advertises `bearer` only when a `BearerValidator` is
installed. Its credential backend retains the validated principal, requires a
finite expiry, issuer and tenant, and rejects anonymous/pseudonymous assurance.
Unknown methods, experimental AAuth/DPoP and an unverified actor token fail
explicitly. They cannot select the bearer validator through a fallback. This
credential helper is not the four-envelope challenge/profile handshake.

Construct `AuthenticatedDispatch` only with a trusted validator result and a
current-authority policy. Its constructor is backend composition, not a public
JSON credential assertion. Each incarnation has a child of the physical route's
cancellation token, independent bounded waiters and an immutable principal.
Reauthentication must establish a new incarnation; this increment does not
implement refresh or step-up state transitions.

Every envelope admitted by this backend requires a valid inline EdDSA or ES256
JCS signature; see [the signing qualification](es256.md). `Sig9421Verifier::verify_for_principal` resolves its key through
`KeyResolver::resolve_for_principal`, including subject, issuer and tenant.
The default implementation returns no key; global or context-only resolvers
cannot silently supply another subject's key. Production resolvers must enforce
ownership, algorithm, rotation and current key revocation. Existing contextual
verification retains its own entry point for previously qualified profiles.
Signature verification uses the original envelope including unknown optional
fields. Typed execution normalization happens afterward.

The installed `CurrentAuthority` policy must acquire an owned guard covering
current credential and signing-key revocation, grants and resource scope. The gate checks it
before verification, drops that guard while awaiting replay storage, then
reacquires it after verification. A revocation during verification therefore
prevents dispatch. The resulting `AuthorizedEnvelope` retains its guard through
handler execution and supplies the authenticated principal separately from
presentation metadata. The application must reject conflicting actor assertions
and must not reuse this invocation's authority for detached or subsequent work.

Reusing a signed envelope is rejected by anti-replay storage. A retry uses a new
envelope ID/signature with the same logical operation ID, passes current authority
again and reaches the application's operation ledger. The backend does not
deduplicate business effects or persist outcomes. The local test handler's
one-effect set demonstrates dispatch identity separation, not durable recovery.
Replay-store failure prevents execution. Deployment must install the appropriate
durable atomic replay store; in-memory fixtures do not qualify restart behavior.

`register_reply` registers before the caller writes its outbound request. Only
explicit already-scoped request families are accepted. Replies must match the
exact conversation, session, connection, operation ID (including absence) and
allowed family, as well as `in_reply_to`. They pass authentication, signature,
replay and current authority before consuming the waiter. A mismatched reply
leaves it registered and cannot fall through to the ordinary application handler.
Creation and pre-auth correlation require their own state-specific policies.

Capacity is bounded to at most 256 pending requests. Invocation and waiter
deadlines are capped at 30 seconds and the current credential/task-assurance
expiry. Dropping a ticket or its wait future removes only that registration;
generation tokens prevent stale cleanup from removing a newer registration.
Dropping/closing the peer wakes its waiters and leaves the parent route owned by
its caller. No authorization guard remains parked in an unpolled reply receiver.
A received reply is an observation; later graph activation or mutation must
acquire current authority again. Cancellation during a handler can leave an
uncertain business operation, which must be reconciled under its same logical ID.

Run `python3 scripts/check-gate6-auth-dispatch.py` for source-bound local evidence.
The new cases execute real signatures and the actual dispatch/waiter backend.
They do not qualify transport handshake/profile installation, refresh/keepalive,
assurance provenance changes, pending step-up challenge validation, Connect
store-policy integration, durable restart recovery, remote or provider behavior.
G6-026 and all Gate 6 acceptance remain incomplete.

The separate [canonical peer runtime](authenticated-peer.md) now owns the
four-envelope handshake, credential metadata, refresh/keepalive and peer expiry.
It installs this dispatcher per authenticated incarnation and has its own local
TLS/QUIC evidence. This backend's earlier evidence remains scoped to its original
source inputs; production adapter/service installation and step-up remain open.


The canonical secure profile additionally enforces typed command floors after
signature verification and before handler/waiter admission: `uctp:session` for
Session/Connection and related conversation control, `uctp:data` for messages
and DTMF, and `uctp:subscribe` for subscriptions. This is independent of the
required current resource-authority lease. Missing scope returns 403
`insufficient_scope`; scopes asserted in envelope extensions or payloads confer
nothing. Generic ACK/error replies inherit the registered request's scope and
cannot consume its waiter without it. A resumed step-up request is checked
against the successor token's scope before the execution/unknown-outcome marker.
Generic operation commands need the installed authority policy to resolve the
original operation and check its original command floor, current grants and
resources; no generic operation envelope grants blanket authority.
The exhaustive typed mapping and current qualification are in
[`secure-profile.md`](secure-profile.md).


Verified signatures consume the bounded physical replay fence even when their
command scope is denied. A full peer fence preflights new IDs before verifier or
shared replay-store work, while the post-verification atomic claim still handles
concurrent trusted dispatch. Existing IDs retain signature/freshness and replay
checks. Resource-capacity denial establishes no signature or resource authority.

Current handlers may borrow a concrete owned authority guard through
`AuthorizedEnvelope::authority::<T>()`; see [pending Session views](peer-sessions.md).
The borrowed view lasts only for this invocation and does not grant future
replay authority. A missing/mismatched type must deny before effects.

Driven handlers now have a bounded same-peer `AuthorizedEnvelope::exchange`
capability. [Scoped peer exchanges](peer-exchange.md) documents its exact-scope
policy hooks, held current key guard, shared replay verification and invocation
cancellation. A saved envelope cannot extend that execution lifetime.
