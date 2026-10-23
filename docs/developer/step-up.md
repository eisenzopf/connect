# Correlated assurance step-up

The canonical peer implements the G6-026 step-up exchange. Current task
qualification is recorded in [the authentication audit](authentication-audit.md); this increment is not
Connect service installation or Gate 6 acceptance.

The installed `PeerAuthority` supplies an `AssuranceRequirement` for a mutation
while its base-authority lease is held. Requirement resolution is synchronous
and must not reacquire the same authority lock. User-authorized requirements
name the expected delegating user and scopes; task-scoped requirements name the
task and scopes. Verified assurance must name the bound Identity and satisfy
both assurance and principal scopes. A user-authorized label does not implicitly
grant authority over a different task. Refresh still cannot change delegation
provenance or serve as implicit step-up.

The peer preflights the original envelope's signature, replay and current base
authority before it can hold the mutation. If assurance is insufficient, it
publishes a signed `identity.step-up-request` with a fresh unpredictable challenge
ID, installed methods, exact resource/operation scope and correlation to the
original envelope. Current authority is retained through challenge writer
admission, then released. Waiting for a client never parks an authority guard.
Only one mutation is held per peer, bounded by the original invocation deadline,
credential/task expiry and the existing 30-second invocation maximum. Another
request cannot evict it.

The held value is a private, non-cloneable proof of that already verified
envelope, bound to the originating dispatch incarnation and lifetime. Its
original envelope and expected revision remain immutable. It cannot be created
from a response or an envelope without a logical operation ID, and has no
public replay exemption. Exact resource fields include their absence when a
new-resource mutation legitimately precedes resource allocation.

A response must match the live challenge ID, peer generation and exact
Conversation, Session, Connection and operation scope, including absent fields.
A supplied revision cannot conflict with the held revision. Wrong scope or an
invalid signature cannot consume the pending proof. A matching, currently
authenticated response consumes it for one validation attempt. Unknown methods
never fall back to bearer; invalid credentials, wrong ownership/delegation,
denial, timeout or cancellation discard the hold without executing it.

The candidate credential must retain subject, issuer, tenant and mapped Identity
and establish the requested assurance provenance. The policy's `step_up` callback
validates both current credentials, the exact response and Identity/Device mapping.
A host can retain complete original-command authority here. Connect instead
starts with Binding-only readers, then uses the protected `step_up_media` hook to
rotate any exact retained physical media credential and obtain fresh original-
command authority. Binding-only readers must never authorize command execution.
The current requirement is checked again under the returned command guard after
asynchronous validation. Changed policy or revocation prevents execution; an
assurance match alone is insufficient.

The new dispatch consumes the private proof once while the exact prior dispatch
is still alive. It executes the original mutation under the verified candidate
principal and transition guard, without re-consuming its signature or changing
its logical operation ID/revision. A replay of the original signed envelope is
still rejected. A fresh signed envelope may recover the same logical operation
through the application's ledger. The local one-effect fixture is process-local;
durable operation recovery remains the Connect service's responsibility.

On completion the peer installs a new credential incarnation, rotates its token,
retires old waiters and returns signed `auth.session`, scoped
`identity.assurance-changed` with reason `verified-step-up`, and the original
mutation's correlated result. A business conflict can reject the preserved
expected revision even after credential verification succeeds. Current authority
remains held through result writer admission. Cancellation during actual
business execution may be uncertain; reconcile the same logical operation.

Successful refresh supersedes a pending challenge and emits a correlated
`step_up_superseded` problem for the held invocation. Late responses cannot execute
it. The driven reliable loop also publishes `step_up_expired` when a challenge
times out on an idle stream, leaving a still-valid peer available for keepalive
or other authorized work. EOF, peer expiry or cancellation discards the hold.
Direct `handle` integrations must drive the peer lifecycle themselves; expired
holds are discarded before their next input.

Run `python3 scripts/check-gate6-step-up.py` for local source-bound evidence.
New tests use real Ed25519 signatures and the actual canonical peer, including a
pinned local `uctp/2` TLS/QUIC stream. Credential, identity/device, revocation and
resource-policy callbacks are controlled fixtures. Production QUIC/WT adapter
installation, Connect store policy and durable canonical dispatch, browser/SDK
interoperability, multi-process/restart replay retention, media-quality, remote
and live-provider qualification remain separate work. No generic assurance-change
publication for revoked/expired authorities is inferred from the verified step-up
event.

The [authentication recovery implementation](auth-recovery.md) adds the retained
physical-peer replay fence, explicit signature problem taxonomy and dual scoped
step-up failure outcomes, including unknown interrupted execution. Its report
supersedes the earlier partial recovery boundaries for these source changes.


The [authentication retirement lifecycle](assurance-retirement.md) now supplies
required credential-incarnation retirement signals and a signed terminal
assurance-change before goodbye. Its separate report includes current/candidate/
successor isolation and blocked execution/writer cleanup. The earlier evidence
above retains its recorded source scope.

The Connect SDK's `CanonicalClient::request` and `request_value` return a
`ReplyBatch` when a signed, correctly scoped `identity.step-up-request` suspends
the command. Inspect `step_up_request()`; `ack()` returns the explicit
`step-up-required` application problem and does not report success. The client's
`step_up_required()` returns its pending required assurance. `complete_step_up(explicit_successor_bearer)` answers that one cached
challenge on the same physical peer with a fresh signed envelope, preserving its
original resource fields, logical operation ID and expected revision. The server
resumes the held command; the SDK does not send the original command again.
`StepUpCompletion` separates the verified authentication metadata from the
original command's replies. Check the replies' ACK and durable operation outcome
even when authentication succeeded.

Completion retains the original request deadline, previous credential expiry and
server signing pin. It validates both the correlated rotated `auth.session` and
the following `verified-step-up` assurance event before installing metadata.
Only that exact transition is nonterminal; unrelated assurance changes still
retire the peer. Unrelated facts remain queued. A definitive correlated refusal
preserves the previous local binding and consumes the challenge. Cancellation
before sending preserves the cached challenge; partial IO, deadline expiry and
incomplete or contradictory transitions retire the physical peer. Another batch
or refresh cannot replace a pending SDK challenge. Owned public/private/handoff
audio exposes the same explicit completion API. Its single reader accepts a
bounded exact assurance-transition expectation without restarting partial frames;
only the foreground's complete validation renews its credential deadline. A
signed late transition after definitive refusal remains terminal.

The server's two failure correlations remain separate. An original-command
problem received before the response's authentication refusal stays queued for
`receive()`. An interrupted execution's `details.outcome: unknown` remains
unknown; the authentication refusal does not turn it into a failed business
operation. Reconcile the original logical operation before attempting new work.

This SDK path does not grant assurance or mint a successor credential. The
[explicit user consent installation](user-assurance.md) supplies current human
provenance and shared Store-backed user-assurance policy for native and HTTP
commands. [Owned SDK qualification](../evidence/gate-6/auxiliary-owned-step-up.json)
covers 80 SDK tests on controlled real signed QUIC, including the original
mutation's identity, deadline, cancellation and same-reader expiry renewal.

The [production native step-up evidence](../evidence/gate-6/native-media-step-up.json)
now records real user consent and owned native DTMF for EdDSA and ES256. The host
rotates the exact physical source's credential, records its observed epoch,
releases transition readers, then freshly admits the unchanged original command.
The original envelope ID, logical operation and stream revision are preserved.
An unenrolled consent refuses before any media epoch or signal operation. A SQL
refusal after credential rotation remains a failed original outcome separately
from successful authentication; explicit retry keeps the logical operation and
does not rotate media again.

Both ordinary refresh and step-up retain a publication guard from the first
possible physical transition through successor binding commit. Cancellation or
publication failure retires that physical root and children; it never restores
an older usable gate after uncertain media effects. Controlled guard tests are
separate from production IO fault/recovery qualification.

The [signal-only continuation](../evidence/gate-6/native-signal-authority.json)
qualifies consent containing only `dtmf.send`, plus receiver refresh containing
only `dtmf.receive`, on both signing algorithms. One case verifies fresh
signaling through the same source and a receive-only successor. Another proves
an actual PCMU route before step-up and refuses new RTP while the receiver
retains full audio authority. A valid audio subscription returns an explicit
authorization refusal before any operation is created. Protocol data permission
is derived from named signal permission; Session, media and subscription grants
remain separate.

Task-scoped native profiles, production partial/recovery cases and exact
auxiliary-clause qualification remain open.
The [SDK component report](../evidence/gate-6/auxiliary-sdk-step-up.json) records
its controlled signed QUIC scope separately from production service, owned media,
exact auxiliary vectors and acceptance qualification.
