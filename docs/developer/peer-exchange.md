# Scoped peer exchanges (G6-026, in progress)

Driven canonical handlers can use `AuthorizedEnvelope::exchange` to send one
scoped request to the same physical peer and await its correlated reply. The
actor continues reading replies and advancing its writer while the handler
waits. This supports server-initiated negotiation without making the handler
responsible for the native reader, writer or signing keys.

Authentication transitions, application commands and independent registry
delivery work remain ordered. The driver owns one active work future, a bounded
command FIFO, bounded exchange registrations and verification futures, and one
bounded writer pump. Every future stays inside that physical actor; no detached
worker is spawned. An in-handler exchange has a separate reply lane, so filling
the command FIFO cannot prevent its registered reply from being processed.
Grant acquisition for independent delivery is polled alongside the writer; it
cannot block the response write whose lease a queued grant writer needs.

The current `pending_capacity` bounds each command/exchange queue and the
retained control-output slots (1 through 256, fixture/default composition 2).
Writer queue capacity is twice that value plus four; frame allocation still
obeys the 1 MiB envelope bound. One physical control action runs at a time.
All work, reply validation and writes have absolute deadlines clamped to their
original credential/invocation lifetime and the existing 30-second maximum.
Refresh waits behind an active invocation, then retires old incarnation waiters;
it cannot lend its new authority to that invocation. Pending step-up expiry is
reported before starting work already queued behind an earlier action.

An exchange request must use the invocation's exact Conversation, Session,
Connection and logical-operation scope and a supported registered request family.
The actor signs a fresh wire ID/timestamp, registers that actual request before
writing, and keeps the logical operation ID unchanged. It never uses a backend
placeholder ID for correlation. Existing supported families are Connection
offer/answer, Connection update/ACK, Stream subscribe/unsubscribe/ACK and scoped
operation request/update. Unscoped resource creation and arbitrary unsolicited
message families are not exchange requests.

The installed policy must implement both `exchange_request` and `exchange_reply`.
The defaults fail closed. Each synchronous hook inspects the still-held current
lease (`AuthorityLease::authority::<T>()`), approves the exact outbound request
or registered reply observation and retains the needed protection. It must not
await or reacquire that guard's grant/key-store lock. Command floors are checked
independently, and a role or original command permission does not automatically
authorize a different outbound action.

Reply policy returns `ExchangeReplyAuthority`: a current lease and a
`PrincipalVerificationKey` snapshot selected under that guard for the exact
subject, issuer, tenant and key ID. The verifier avoids a second key-store lookup
while a writer queues. It uses the same configured verifier, clock, signature
algorithm, key validity/revocation checks, JCS body and GLOBAL atomic replay store
as ordinary verification. The physical replay fence is also consumed. There is
no signature or anti-replay exception. The key snapshot is trusted policy
composition, never a client-supplied grant.

Wrong signatures, family or scope cannot satisfy a registration. Wrong/denied
replies leave the bounded waiter available for a later valid fresh envelope.
Duplicate replies cannot complete it twice. Returning from the handler, timing
out, EOF, source retirement or shutdown cancels its exchange capability even if
the handler saved its envelope. A retained lease protects only that ongoing
invocation and response; it is not future execution authority.

`SubmissionUnknown` means the write may have started but no valid observation
was obtained. Preserve the logical operation ID and reconcile through the
application journal; do not infer acceptance or automatically redial. A reply
is a validated observation, not permission for later activation or a media
readiness fact. The media/application handler still validates its negotiated
bindings, epochs, generations and actual readiness under current authority.

Direct `handle`/standalone dispatch integrations do not own this actor capability
and return Unsupported for `exchange`. Driven handlers must use it for an
in-handler same-peer wait, rather than waiting on an externally registered
direct reply ticket or awaiting a registry submission back into that same
ordered actor. External registry submissions retain current delivery policy
and report bounded LOCAL writer admission only. Manual integrations own their
own reader/writer scheduling and direct reply tickets.

`scripts/check-gate6-peer-exchange.py` records source-bound actor, key/replay,
local TLS QUIC/WebTransport/WSS and earlier native/Session/auth/media regressions
in `docs/evidence/gate-6/peer-exchange.json`. The fixture callback uses scoped
Connection update/ACK observations; its logical counter is not native media
qualification. Full canonical media/engine installation, production
ParticipantPolicy/MediaPolicy lease reuse, invitation/reachability/store bridges,
durable logical Session replay, remaining assurance/lifecycle audit and public
Connect/SDK/browser/remote/provider/quality acceptance remain open.
