# Canonical secure defaults (G6-026 / G6-020, in progress)

The canonical authenticated dispatcher now enforces the secure profile's default
command scopes. The parser produces a typed payload, signature verification
proves its retained original bytes, and the dispatcher checks the retained
principal's applicable scope before handler execution or correlated waiter
admission. The required current-authority leases still check credentials, keys,
identity/device mappings, effective grants and the exact resource. Possessing a
scope is only a prerequisite; neither a scope nor an assurance label replaces
current resource authorization.

| Scope | Typed families |
| --- | --- |
| `uctp:session` | Conversation/membership control, Session control and observations, Connection control/ICE/quality/replacement, stream lifecycle observations, capability advertisement and vCon recording control |
| `uctp:data` | Message send/receipt/history and DTMF |
| `uctp:subscribe` | Stream subscription/unsubscription and conversation subscription/snapshot/event |

`v2::scope_policy::required_scope` exhaustively matches all 65 payload variants:
38 have the Session floor, six data, five subscribe, and 16 use their own
state-specific authority checks. Authentication and identity/step-up prove or
transition credentials; the original held command is checked against the
successor principal before execution starts. ACK/error have no independent
blanket scope: a correlated reply inherits its registered request's required
scope before removing the waiter. Generic operation commands require installed
policy to resolve the original operation, apply its original command floor and
retain current resource authority. They do not establish an operation-independent
grant. There is no permissive canonical fallback or payload-selected scope.
Valid signed IDs still consume the physical replay fence when command scope is
denied. Once that fence is full, the single-owner peer rejects new IDs before
invoking the verifier/shared replay store; its later atomic consume remains the
authoritative verified claim. A capacity denial does not establish signature
validity or grant authority. Missing scope yields 403 `insufficient_scope`.
Refresh uses the newly verified
principal; removing/restoring a required scope immediately changes admission.

The peer retains a separate eight-entry pre-auth ID fence for hello/response,
consuming an ID before credential validation awaits. Duplicate/cancelled attempts
cannot repeat credential work with that ID. Fresh retries keep the current
challenge correlation, absolute ten-second deadline and separate failed-attempt
limit. The initial hello counts as one entry. IDs are never evicted to make room;
capacity failure retires the unauthenticated peer. The normal signature replay
fence and external replay store remain independently active. Challenge
capabilities expose the configured pre-auth capacity and fixed default scope
names.

`StreamLifecycle` and the staged `StreamController` default to 16 cumulative
new transport bindings per Connection. The controller checks a batch's required
new ports before allocating router handles or polling adapter preparation.
Unchanged bindings cost zero; replacements cost one per new port. Preparation
can partially open a batch before failing or being cancelled, so a polled batch
consumes its whole new-port count conservatively. Cleanup releases live-port
capacity after confirmed closure, but never refunds cumulative opens. Reoffers,
pause and logical end retain that history. The model checks committed bindings
again at commit without mutating the previous snapshot on failure. The controller
must stay owned across the same Connection's reoffers. Explicit limits from one
through 4,096 are supported and reported through `opened_stream_limit`,
`opened_binding_count` and `stream_open_budget`; physical active/quarantined
capacity remains separately bounded.

The qualification runner is `scripts/check-gate6-secure-profile.py` and its report
is `docs/evidence/gate-6/secure-profile.json`. Tests use real Ed25519/JCS and local
pinned TLS QUIC, HTTP/3 WebTransport and WSS for scope denial/recovery. Actual
staged RTP/router checks exercise replacement exhaustion and continued PCMU
forwarding, including failed/cancelled preparation accounting. Controlled policy
fixtures are explicit; they do not qualify Connect's installed store policy.

Per-peer 32-Session/64-Connection admission ownership, core IdentityProvider and
Connect EndpointRuntime integration, incoming Session invitation/acceptance,
logical-Session replay across physical reconnect/restart, final assurance
observation audit and Connect's public canonical listener/dispatch remain open.
No browser, remote/provider, media-quality acceptance or Gate 6 closure is
inferred. The Gate 6 journal remains the authority for durable mutations and
uncertain operation reconciliation.
